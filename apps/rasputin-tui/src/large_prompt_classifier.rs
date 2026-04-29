//! Large Prompt Classifier and Artifact Contract System
//!
//! Detects project-style large prompts and decomposes them into bounded chains.
//! Handles prompts like "analyze the whole repo and create exactly 15 canonical docs"
//! without timing out before iteration 1.
//!
//! USER-FACING COMMANDS (simple):
//! - "docs" or "create docs" → Auto-generate canonical documentation
//! - "analyze and create <pattern>" → Extract files from pattern, create each
//! - "create <file1>, <file2>, ..." → Simple comma-separated file list

use crate::artifact_contract::{ArtifactContract, RequiredArtifact, EffortLevel};
use crate::persistence::{ChainLifecycleStatus, ChainStepStatus, PersistentChain, PersistentChainStep};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Detection result for large project prompts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptClassification {
    /// Regular chat/task prompt - direct execution
    Regular,
    /// Large project prompt with artifact contract - needs decomposition
    LargeProject(ArtifactContract),
    /// Large prompt but unclear contract - needs clarification
    Ambiguous { reason: String },
}

/// Type of artifact being produced
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactType {
    Markdown,
    Code { language: String },
    Config,
    Data,
    Documentation,
    Test,
    Script,
    Other(String),
}

/// Status of an artifact in the contract
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArtifactStatus {
    Missing,
    Exists,
    Empty,
    Drafted,
    Validated,
    Failed,
}

/// Validation rules for artifact contracts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactValidationRule {
    ExactCount { count: usize },
    RequiredPaths { paths: Vec<PathBuf> },
    NonEmpty,
    /// Require specific file extension (e.g., "md", "rs", "py")
    Extension { ext: String },
    /// Require all artifacts to be same type
    UniformType,
    RepoGrounded,
}

/// Contract validation result
#[derive(Debug, Clone)]
pub struct ContractValidationResult {
    pub valid: bool,
    pub violations: Vec<String>,
    pub warnings: Vec<String>,
}

/// Large prompt classifier - detects project-style prompts
pub struct LargePromptClassifier;

impl LargePromptClassifier {
    /// Score a prompt and classify it
    pub fn classify(prompt: &str, repo_path: Option<&Path>) -> PromptClassification {
        let score = Self::score_prompt(prompt);
        
        debug!("Prompt classification score: {}/100", score);
        
        // High score = definitely a large project prompt
        if score >= 70 {
            // Try to extract artifact contract
            match Self::extract_contract(prompt, repo_path) {
                Some(contract) => {
                    info!("Detected large project prompt with artifact contract");
                    PromptClassification::LargeProject(contract)
                }
                None => {
                    warn!("High project score but couldn't extract contract");
                    PromptClassification::Ambiguous {
                        reason: "High project indicators but unclear artifact requirements".to_string(),
                    }
                }
            }
        } else if score >= 40 {
            // Medium score = ambiguous
            PromptClassification::Ambiguous {
                reason: "Some project indicators but not definitive".to_string(),
            }
        } else {
            // Low score = regular prompt
            PromptClassification::Regular
        }
    }

    /// Score a prompt based on project indicators (0-100)
    fn score_prompt(prompt: &str) -> u8 {
        let lower = prompt.to_lowercase();
        let mut score: u8 = 0;
        
        // Check for scope indicators (+15 each, max 45)
        let scope_indicators = [
            "entire repository", "deeply analyze", "all code files",
            "whole codebase", "complete analysis", "full repo",
        ];
        for indicator in &scope_indicators {
            if lower.contains(indicator) {
                score += 15;
                break; // Only count once
            }
        }
        
        // Check for quantity indicators (+20 each, max 40)
        if lower.contains("exactly") && lower.matches(char::is_numeric).count() > 0 {
            score += 20;
        }
        if lower.contains("15") || lower.contains("fifteen") {
            score += 20; // Canonical 15 docs pattern
        }
        
        // Check for output file indicators (+10 each, max 30)
        let file_indicators = [
            "produce exactly", "create exactly", "generate exactly",
            "output files", ".md files", "markdown files",
            "deliverables", "artifacts",
        ];
        for indicator in &file_indicators {
            if lower.contains(indicator) {
                score += 10;
            }
        }
        score = score.min(30); // Cap at 30
        
        // Check for strictness indicators (+10 each, max 20)
        let strict_indicators = [
            "do not hallucinate", "base everything strictly",
            "strictly on repo", "no external knowledge",
        ];
        for indicator in &strict_indicators {
            if lower.contains(indicator) {
                score += 10;
            }
        }
        score = score.min(20); // Cap at 20
        
        // Check for multiple numbered deliverables (+10)
        let numbered_items = lower.matches(char::is_numeric).count();
        if numbered_items >= 5 {
            score += 10;
        }
        
        // Check for markdown output (+5)
        if lower.matches(".md").count() >= 3 {
            score += 5;
        }
        
        // Length bonus (+10 if >2000 chars)
        if prompt.len() > 2000 {
            score += 10;
        }
        
        // Canonical docs pattern detection (+25 if matches)
        if Self::is_canonical_15_docs_pattern(prompt) {
            score += 25;
        }
        
        score.min(100)
    }

    /// Check if prompt matches the canonical 15 docs pattern
    fn is_canonical_15_docs_pattern(prompt: &str) -> bool {
        let lower = prompt.to_lowercase();
        
        // Must have "15" or "fifteen"
        let has_15 = lower.contains("15") || lower.contains("fifteen");
        
        // Must have "doc" or "documentation"
        let has_docs = lower.contains("doc") || lower.contains("documentation");
        
        // Must have "canonical" or specific structure words
        let has_structure = lower.contains("canonical") 
            || lower.contains("numbered")
            || lower.contains("01_") || lower.contains("15_");
        
        has_15 && has_docs && has_structure
    }

    /// Extract artifact contract from prompt
    fn extract_contract(prompt: &str, repo_path: Option<&Path>) -> Option<ArtifactContract> {
        // Check for canonical 15 docs pattern first
        if Self::is_canonical_15_docs_pattern(prompt) {
            let root = repo_path.unwrap_or(Path::new("."));
            return Some(ArtifactContract::canonical_15_docs(root));
        }
        
        // Try to extract explicit file list
        let artifacts = Self::extract_file_list(prompt)?;
        
        let root = repo_path.unwrap_or(Path::new("."));
        Some(ArtifactContract::from_artifacts(root, artifacts, prompt.chars().take(200).collect::<String>()))
    }

    /// Extract list of files from prompt - detects any file extension
    fn extract_file_list(prompt: &str) -> Option<Vec<RequiredArtifact>> {
        let mut artifacts = vec![];
        let mut step_num = 1;
        
        // Look for numbered file patterns: "1. filename.ext", "2. filename.ext"
        for line in prompt.lines() {
            let trimmed = line.trim();
            
            // Check for numbered items with any file extension
            if let Some(filename) = Self::extract_filename_with_ext(trimmed) {
                let purpose = Self::extract_purpose_from_line(trimmed, &filename)
                    .unwrap_or_else(|| format!("Artifact {}", step_num));
                
                let artifact_type = Self::infer_artifact_type(&filename);
                
                artifacts.push(RequiredArtifact {
                    path: PathBuf::from(&filename),
                    purpose,
                    status: ArtifactStatus::Missing,
                    step_number: step_num,
                    artifact_type,
                    dependencies: vec![],
                    estimated_effort: EffortLevel::Medium,
                });
                step_num += 1;
            }
        }
        
        // Check for Markdown code blocks with file paths
        let in_code_block = prompt.contains("```") || prompt.contains("`");
        if in_code_block && artifacts.is_empty() {
            // Extract from markdown links or code blocks
            for line in prompt.lines() {
                if (line.contains('`') || line.contains('[')) && Self::has_file_extension(line) {
                    if let Some(filename) = Self::extract_filename(line) {
                        let artifact_type = Self::infer_artifact_type(&filename);
                        artifacts.push(RequiredArtifact {
                            path: PathBuf::from(&filename),
                            purpose: format!("Artifact {}", step_num),
                            status: ArtifactStatus::Missing,
                            step_number: step_num,
                            artifact_type,
                            dependencies: vec![],
                            estimated_effort: EffortLevel::Medium,
                        });
                        step_num += 1;
                    }
                }
            }
        }
        
        if artifacts.is_empty() {
            None
        } else {
            Some(artifacts)
        }
    }

    /// Check if line contains a file extension pattern
    fn has_file_extension(line: &str) -> bool {
        // Match patterns like filename.ext or filename.rs or filename.py
        line.split_whitespace().any(|word| {
            let word = word.trim_matches(|c| c == '`' || c == '[' || c == ']' || c == '(' || c == ')' || c == '"' || c == '\'');
            word.contains('.') && word.split('.').last().map(|ext| ext.len() >= 2 && ext.len() <= 6).unwrap_or(false)
        })
    }

    /// Extract filename with extension from a line
    fn extract_filename_with_ext(line: &str) -> Option<String> {
        // Look for patterns like "1. filename.ext" or "- filename.ext"
        let words: Vec<&str> = line.split_whitespace().collect();
        
        for word in &words {
            let clean = word.trim_matches(|c| c == '`' || c == '[' || c == ']' || c == '(' || c == ')' || c == '"' || c == '\'' || c == ':' || c == ',' || c == '.');
            if clean.contains('.') {
                let parts: Vec<&str> = clean.split('.').collect();
                if parts.len() >= 2 {
                    let ext = parts.last().unwrap();
                    // Valid extensions are 2-6 chars (rs, py, js, ts, go, java, yaml, json, etc.)
                    if ext.len() >= 2 && ext.len() <= 6 && ext.chars().all(|c| c.is_alphanumeric()) {
                        return Some(clean.to_string());
                    }
                }
            }
        }
        None
    }

    /// Extract purpose description from a line
    fn extract_purpose_from_line(line: &str, filename: &str) -> Option<String> {
        // Remove the filename and common prefixes
        let without_file = line.replace(filename, "");
        let without_prefix = without_file
            .trim_start_matches(|c: char| c == '-' || c == '*' || c == '.' || c == ' ' || c.is_numeric())
            .trim();
        
        if without_prefix.is_empty() {
            None
        } else {
            Some(without_prefix.trim().trim_matches(|c| c == ':' || c == '-' || c == ' ').to_string())
        }
    }

    /// Infer artifact type from filename extension
    fn infer_artifact_type(filename: &str) -> ArtifactType {
        let ext = filename.split('.').last().unwrap_or("").to_lowercase();
        match ext.as_str() {
            "md" | "markdown" => ArtifactType::Markdown,
            "rs" => ArtifactType::Code { language: "rust".to_string() },
            "py" => ArtifactType::Code { language: "python".to_string() },
            "js" => ArtifactType::Code { language: "javascript".to_string() },
            "ts" => ArtifactType::Code { language: "typescript".to_string() },
            "go" => ArtifactType::Code { language: "go".to_string() },
            "java" => ArtifactType::Code { language: "java".to_string() },
            "c" | "h" => ArtifactType::Code { language: "c".to_string() },
            "cpp" | "hpp" | "cc" => ArtifactType::Code { language: "cpp".to_string() },
            "rb" => ArtifactType::Code { language: "ruby".to_string() },
            "swift" => ArtifactType::Code { language: "swift".to_string() },
            "kt" => ArtifactType::Code { language: "kotlin".to_string() },
            "yaml" | "yml" => ArtifactType::Config,
            "json" => ArtifactType::Data,
            "toml" => ArtifactType::Config,
            "ini" => ArtifactType::Config,
            "sh" | "bash" | "zsh" => ArtifactType::Script,
            "test" | "spec" => ArtifactType::Test,
            _ => ArtifactType::Other(ext),
        }
    }

    /// Extract filename from markdown line
    fn extract_filename(line: &str) -> Option<String> {
        // Try backticks first: `filename.any`
        if let Some(start) = line.find('`') {
            if let Some(end) = line[start+1..].find('`') {
                let candidate = &line[start+1..start+1+end];
                if candidate.contains('.') {
                    return Some(candidate.to_string());
                }
            }
        }
        
        // Try brackets: [filename.any]
        if let Some(start) = line.find('[') {
            if let Some(end) = line[start+1..].find(']') {
                let candidate = &line[start+1..start+1+end];
                if candidate.contains('.') {
                    return Some(candidate.to_string());
                }
            }
        }
        
        None
    }
}

/// Convert an artifact contract into a persistent chain
pub fn contract_to_chain(
    contract: &ArtifactContract,
    repo_path: impl Into<String>,
) -> PersistentChain {
    let now = chrono::Local::now();
    
    // Build steps from artifacts
    let steps: Vec<PersistentChainStep> = contract.artifacts.iter().map(|artifact| {
        PersistentChainStep {
            id: format!("step-{}", artifact.step_number),
            description: format!(
                "Generate {} - {}",
                artifact.path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("doc-{}", artifact.step_number)),
                artifact.purpose
            ),
            status: ChainStepStatus::Pending,
            retry_of: None,
            retry_attempt: 0,
            execution_outcome: None,
            execution_result_class: None,
            execution_results: vec![],
            failure_reason: None,
            recovery_step_kind: None,
            evidence_snapshot: None,
            force_override_used: false,
            tool_calls: vec![],
            result_summary: None,
            validation_passed: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            replay_record: None,
        }
    }).collect();
    
    PersistentChain {
        id: format!("chain-{}", uuid::Uuid::new_v4()),
        name: format!("Artifact Contract: {} artifacts", contract.artifacts.len()),
        objective: contract.source_prompt_summary.clone(),
        raw_prompt: format!(
            "Large prompt decomposition for artifact contract.\n\nContract ID: {}\nRoot: {}\nArtifacts: {}\n\nValidation Rules: {:?}",
            contract.contract_id,
            contract.root_dir.display(),
            contract.artifacts.len(),
            contract.validation_rules
        ),
        status: ChainLifecycleStatus::Draft,
        steps,
        active_step: None,
        repo_path: Some(repo_path.into()),
        conversation_id: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
        archived: false,
        total_steps_executed: 0,
        total_steps_failed: 0,
        execution_outcome: None,
        force_override_used: false,
        objective_satisfaction: crate::state::ObjectiveSatisfaction::default(),
        selected_context_files: vec![],
        context_state: None,
        pending_checkpoint: None,
        git_grounding: None,
        audit_log: crate::state::AuditLog::new(),
    }
}

/// Simple command parser - converts natural language to artifact contracts
/// This hides all complexity behind simple commands
pub struct SimpleCommandParser;

impl SimpleCommandParser {
    /// Parse simple commands like "docs" or "create docs"
    pub fn parse_simple(input: &str, repo_path: impl AsRef<Path>) -> Option<ArtifactContract> {
        let normalized = input.trim().to_lowercase();
        let root = repo_path.as_ref().to_path_buf();
        
        // "docs" or "create docs" → canonical 15 docs
        if normalized == "docs" || normalized == "create docs" {
            return Some(ArtifactContract::canonical_15_docs(root));
        }
        
        // "create <file1>, <file2>..." → simple file list
        if normalized.starts_with("create ") {
            let files_part = &normalized[7..]; // after "create "
            let files: Vec<&str> = files_part.split(',').map(|s| s.trim()).collect();
            
            if !files.is_empty() {
                let artifacts: Vec<RequiredArtifact> = files.iter().map(|f| {
                    RequiredArtifact {
                        path: root.join(f),
                        purpose: format!("Create {}", f),
                        status: ArtifactStatus::Missing,
                        step_number: 0,
                        artifact_type: ArtifactType::Documentation,
                        dependencies: vec![],
                        estimated_effort: crate::artifact_contract::EffortLevel::Medium,
                    }
                }).collect();
                
                return Some(ArtifactContract::from_artifacts(
                    root,
                    artifacts,
                    input.chars().take(100).collect::<String>(),
                ));
            }
        }
        
        // "test files for X" → test suite (simplified - just create basic test files)
        if normalized.contains("test") && normalized.contains("for") {
            // Create basic test artifacts
            let test_artifacts = vec![
                RequiredArtifact {
                    path: root.join("tests").join("mod.rs"),
                    purpose: "Test module entry point".to_string(),
                    status: ArtifactStatus::Missing,
                    step_number: 1,
                    artifact_type: ArtifactType::Code { language: "rust".to_string() },
                    dependencies: vec![],
                    estimated_effort: crate::artifact_contract::EffortLevel::Medium,
                },
            ];
            return Some(ArtifactContract::from_artifacts(
                root,
                test_artifacts,
                "Create test files",
            ));
        }
        
        None
    }
    
    /// Check if this is a simple command that can be auto-handled
    pub fn is_simple_command(input: &str) -> bool {
        let normalized = input.trim().to_lowercase();
        normalized == "docs" || 
        normalized == "create docs" ||
        normalized.starts_with("create ") ||
        (normalized.contains("test") && normalized.contains("for"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_regular_prompt() {
        let prompt = "How do I write a Rust function?";
        let result = LargePromptClassifier::classify(prompt, None);
        assert_eq!(result, PromptClassification::Regular);
    }

    #[test]
    fn test_simple_command_docs() {
        let contract = SimpleCommandParser::parse_simple("docs", "/tmp/test");
        assert!(contract.is_some());
        assert_eq!(contract.unwrap().artifacts.len(), 15);
    }

    #[test]
    fn test_simple_command_create_files() {
        let contract = SimpleCommandParser::parse_simple(
            "create README.md, CONTRIBUTING.md, LICENSE",
            "/tmp/test"
        );
        assert!(contract.is_some());
        assert_eq!(contract.unwrap().artifacts.len(), 3);
    }

    #[test]
    fn test_classify_canonical_15_docs() {
        let prompt = r#"
Analyze the entire repository and produce exactly 15 canonical documentation files:

1. 01_OVERVIEW_AND_ARCHITECTURE.md
2. 02_INSTALLATION_AND_SETUP.md
...
15. 15_FUTURE_ROADMAP.md

Do not hallucinate. Base everything strictly on repo content.
"#;
        let result = LargePromptClassifier::classify(prompt, None);
        
        match result {
            PromptClassification::LargeProject(contract) => {
                assert_eq!(contract.artifacts.len(), 15);
            }
            _ => panic!("Expected LargeProject classification"),
        }
    }

    #[test]
    fn test_contract_validation() {
        let contract = ArtifactContract::canonical_15_docs("/tmp/test");
        
        // Initially all missing
        let counts = contract.count_by_status();
        assert_eq!(counts.get(&ArtifactStatus::Missing), Some(&15));
        
        // Validation should fail (all missing)
        let result = contract.validate();
        assert!(!result.valid);
        assert!(!result.violations.is_empty());
    }
}
