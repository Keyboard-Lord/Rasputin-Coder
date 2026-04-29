//! Artifact Contract - Explicit deliverable specification
//!
//! Defines what artifacts must be produced, their types, and validation rules.
//! This is the core data structure that drives the large task decomposition.

use crate::large_prompt_classifier::{ArtifactType, ArtifactValidationRule, ArtifactStatus};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Explicit contract for artifact generation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactContract {
    /// Unique contract identifier
    pub contract_id: String,
    /// Root directory for all artifacts
    pub root_dir: PathBuf,
    /// List of required artifacts to produce
    pub artifacts: Vec<RequiredArtifact>,
    /// Validation rules to apply
    pub validation_rules: Vec<ArtifactValidationRule>,
    /// Original prompt summary
    pub source_prompt_summary: String,
    /// Detected patterns in the prompt
    pub detected_patterns: Vec<String>,
    /// Confidence score (0-100)
    pub confidence: u8,
}

/// Required artifact specification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequiredArtifact {
    pub path: PathBuf,
    pub purpose: String,
    pub status: ArtifactStatus,
    pub step_number: usize,
    pub artifact_type: ArtifactType,
    /// Dependencies on other artifacts (step numbers)
    pub dependencies: Vec<usize>,
    /// Estimated effort (tokens, lines, or complexity score)
    pub estimated_effort: EffortLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffortLevel {
    Small = 1,    // < 100 lines
    Medium = 2,   // 100-500 lines  
    Large = 3,    // 500-1000 lines
    XLarge = 4,   // > 1000 lines
}

/// Contract validation result
#[derive(Debug, Clone)]
pub struct ContractValidationResult {
    pub valid: bool,
    pub violations: Vec<ContractViolation>,
    pub warnings: Vec<String>,
    pub completion_pct: u8,
    pub artifacts_by_status: HashMap<ArtifactStatus, usize>,
}

#[derive(Debug, Clone)]
pub struct ContractViolation {
    pub rule: ArtifactValidationRule,
    pub message: String,
    pub affected_artifacts: Vec<PathBuf>,
}

impl ArtifactContract {
    /// Create a new empty contract
    pub fn new(root_dir: impl Into<PathBuf>, summary: impl Into<String>) -> Self {
        Self {
            contract_id: format!("contract-{}", uuid::Uuid::new_v4()),
            root_dir: root_dir.into(),
            artifacts: vec![],
            validation_rules: vec![ArtifactValidationRule::RepoGrounded],
            source_prompt_summary: summary.into(),
            detected_patterns: vec![],
            confidence: 0,
        }
    }

    /// Create a contract from existing artifacts
    pub fn from_artifacts(
        root_dir: impl Into<PathBuf>,
        artifacts: Vec<RequiredArtifact>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            contract_id: format!("contract-{}", uuid::Uuid::new_v4()),
            root_dir: root_dir.into(),
            artifacts,
            validation_rules: vec![ArtifactValidationRule::RepoGrounded],
            source_prompt_summary: summary.into(),
            detected_patterns: vec![],
            confidence: 0,
        }
    }

    /// Build the canonical 15 documentation contract
    pub fn canonical_15_docs(root_dir: impl Into<PathBuf>) -> Self {
        let root = root_dir.into();
        let docs_dir = root.join("docs");
        
        let artifact_specs = vec![
            (1, "01_PROJECT_OVERVIEW.md", "Project overview and scope", EffortLevel::Medium),
            (2, "02_ARCHITECTURE.md", "System architecture", EffortLevel::Large),
            (3, "03_TECHNOLOGY_STACK.md", "Technology choices and rationale", EffortLevel::Medium),
            (4, "04_API_REFERENCE.md", "API documentation", EffortLevel::Large),
            (5, "05_CONFIGURATION.md", "Configuration options", EffortLevel::Medium),
            (6, "06_DEVELOPMENT_GUIDE.md", "How to develop", EffortLevel::Medium),
            (7, "07_TESTING_STRATEGY.md", "Testing approach", EffortLevel::Medium),
            (8, "08_DEPLOYMENT.md", "Deployment instructions", EffortLevel::Medium),
            (9, "09_SECURITY.md", "Security considerations", EffortLevel::Medium),
            (10, "10_PERFORMANCE.md", "Performance characteristics", EffortLevel::Medium),
            (11, "11_INTEGRATION.md", "Integration patterns", EffortLevel::Medium),
            (12, "12_TROUBLESHOOTING.md", "Common issues and solutions", EffortLevel::Medium),
            (13, "13_MIGRATION.md", "Migration guide", EffortLevel::Medium),
            (14, "14_CHANGELOG.md", "Version history", EffortLevel::Small),
            (15, "15_FUTURE_ROADMAP.md", "Future plans", EffortLevel::Small),
        ];

        let artifacts: Vec<RequiredArtifact> = artifact_specs
            .into_iter()
            .map(|(num, filename, purpose, effort)| RequiredArtifact {
                path: docs_dir.join(filename),
                purpose: purpose.to_string(),
                status: ArtifactStatus::Missing,
                step_number: num,
                artifact_type: ArtifactType::Markdown,
                dependencies: vec![], // Docs can be parallel, but overview should be first
                estimated_effort: effort,
            })
            .collect();

        Self {
            contract_id: format!("contract-15docs-{}", uuid::Uuid::new_v4()),
            root_dir: root,
            artifacts,
            validation_rules: vec![
                ArtifactValidationRule::ExactCount { count: 15 },
                ArtifactValidationRule::NonEmpty,
                ArtifactValidationRule::Extension { ext: "md".to_string() },
                ArtifactValidationRule::RepoGrounded,
            ],
            source_prompt_summary: "Generate 15 canonical documentation files".to_string(),
            detected_patterns: vec![
                "exactly_15".to_string(),
                "canonical_docs".to_string(),
                "repo_analysis".to_string(),
            ],
            confidence: 95,
        }
    }

    /// Build a multi-language SDK contract
    pub fn multi_language_sdk(root_dir: impl Into<PathBuf>, languages: &[&str]) -> Self {
        let root = root_dir.into();
        let sdk_dir = root.join("sdk");
        
        let mut artifacts = vec![];
        for (i, lang) in languages.iter().enumerate() {
            let (filename, purpose): (&str, &str) = match *lang {
                "rust" => ("src/lib.rs", "Rust SDK implementation"),
                "python" => ("client.py", "Python SDK"),
                "javascript" | "js" => ("index.js", "JavaScript SDK"),
                "typescript" | "ts" => ("index.ts", "TypeScript SDK with types"),
                "go" => ("client.go", "Go SDK"),
                "java" => ("src/main/java/Client.java", "Java SDK"),
                "csharp" | "cs" => ("Client.cs", "C# SDK"),
                "ruby" => ("lib/client.rb", "Ruby SDK"),
                "swift" => ("Sources/Client.swift", "Swift SDK"),
                "kotlin" => ("src/main/kotlin/Client.kt", "Kotlin SDK"),
                _ => ("client", "SDK"),
            };
            
            artifacts.push(RequiredArtifact {
                path: sdk_dir.join(lang.to_lowercase()).join(filename),
                purpose: purpose.to_string(),
                status: ArtifactStatus::Missing,
                step_number: i + 1,
                artifact_type: ArtifactType::Code { language: lang.to_string() },
                dependencies: vec![], // SDKs are independent
                estimated_effort: EffortLevel::Large,
            });
        }

        let count = artifacts.len();
        
        Self {
            contract_id: format!("contract-sdk-{}", uuid::Uuid::new_v4()),
            root_dir: root,
            artifacts,
            validation_rules: vec![
                ArtifactValidationRule::ExactCount { count },
                ArtifactValidationRule::NonEmpty,
                ArtifactValidationRule::RepoGrounded,
            ],
            source_prompt_summary: format!("Generate SDK clients for {} languages", languages.join(", ")),
            detected_patterns: vec!["multi_language".to_string(), "sdk_generation".to_string()],
            confidence: 90,
        }
    }

    /// Build a test suite contract
    pub fn test_suite(root_dir: impl Into<PathBuf>, coverage: TestCoverageLevel) -> Self {
        let root = root_dir.into();
        let tests_dir = root.join("tests");
        
        let test_files = match coverage {
            TestCoverageLevel::Basic => vec![
                ("unit.rs", "Unit tests for core functions"),
                ("integration.rs", "Integration tests"),
            ],
            TestCoverageLevel::Standard => vec![
                ("unit.rs", "Unit tests for core functions"),
                ("integration.rs", "Integration tests"),
                ("e2e.rs", "End-to-end tests"),
                ("edge_cases.rs", "Edge case and error handling"),
            ],
            TestCoverageLevel::Comprehensive => vec![
                ("unit.rs", "Unit tests for core functions"),
                ("integration.rs", "Integration tests"),
                ("e2e.rs", "End-to-end tests"),
                ("edge_cases.rs", "Edge case handling"),
                ("performance.rs", "Performance and load tests"),
                ("security.rs", "Security tests"),
                ("compatibility.rs", "Compatibility tests"),
            ],
        };

        let artifacts: Vec<RequiredArtifact> = test_files
            .into_iter()
            .enumerate()
            .map(|(i, (filename, purpose))| RequiredArtifact {
                path: tests_dir.join(filename),
                purpose: purpose.to_string(),
                status: ArtifactStatus::Missing,
                step_number: i + 1,
                artifact_type: ArtifactType::Test,
                dependencies: if i > 0 { vec![i] } else { vec![] },
                estimated_effort: EffortLevel::Large,
            })
            .collect();

        let count = artifacts.len();
        
        Self {
            contract_id: format!("contract-tests-{}", uuid::Uuid::new_v4()),
            root_dir: root,
            artifacts,
            validation_rules: vec![
                ArtifactValidationRule::ExactCount { count },
                ArtifactValidationRule::NonEmpty,
                ArtifactValidationRule::RepoGrounded,
            ],
            source_prompt_summary: format!("Generate {} test files", count),
            detected_patterns: vec!["test_suite".to_string(), "comprehensive_testing".to_string()],
            confidence: 85,
        }
    }

    /// Validate the contract against current state
    pub fn validate(&self) -> ContractValidationResult {
        let mut violations = vec![];
        let mut warnings = vec![];
        
        // Count by status
        let mut by_status: HashMap<ArtifactStatus, usize> = HashMap::new();
        for artifact in &self.artifacts {
            *by_status.entry(artifact.status.clone()).or_insert(0) += 1;
        }
        
        // Calculate completion percentage
        let total = self.artifacts.len();
        let completed = by_status.get(&ArtifactStatus::Validated).unwrap_or(&0)
            + by_status.get(&ArtifactStatus::Drafted).unwrap_or(&0);
        let completion_pct = if total > 0 {
            ((completed as f32 / total as f32) * 100.0) as u8
        } else {
            0
        };
        
        // Apply validation rules
        for rule in &self.validation_rules {
            match rule {
                ArtifactValidationRule::ExactCount { count } => {
                    if self.artifacts.len() != *count {
                        violations.push(ContractViolation {
                            rule: rule.clone(),
                            message: format!(
                                "Expected exactly {} artifacts, found {}",
                                count, self.artifacts.len()
                            ),
                            affected_artifacts: vec![],
                        });
                    }
                }
                ArtifactValidationRule::RequiredPaths { paths } => {
                    for required in paths {
                        if !self.artifacts.iter().any(|a| a.path == *required) {
                            violations.push(ContractViolation {
                                rule: rule.clone(),
                                message: format!("Missing required artifact: {}", required.display()),
                                affected_artifacts: vec![required.clone()],
                            });
                        }
                    }
                }
                ArtifactValidationRule::NonEmpty => {
                    let empty = self.artifacts.iter()
                        .filter(|a| matches!(a.status, ArtifactStatus::Empty))
                        .map(|a| a.path.clone())
                        .collect::<Vec<_>>();
                    if !empty.is_empty() {
                        violations.push(ContractViolation {
                            rule: rule.clone(),
                            message: format!("{} artifacts are empty", empty.len()),
                            affected_artifacts: empty,
                        });
                    }
                }
                ArtifactValidationRule::Extension { ext } => {
                    let wrong_ext: Vec<_> = self.artifacts.iter()
                        .filter(|a| !a.path.extension().map(|e| e == ext.as_str()).unwrap_or(false))
                        .map(|a| a.path.clone())
                        .collect();
                    if !wrong_ext.is_empty() {
                        warnings.push(format!(
                            "{} artifacts don't have .{} extension",
                            wrong_ext.len(), ext
                        ));
                    }
                }
                ArtifactValidationRule::RepoGrounded => {
                    // This is a semantic check - content must be based on repo
                    // Implemented during actual validation phase
                }
                ArtifactValidationRule::UniformType => {
                    let first_type = self.artifacts.first().map(|a| &a.artifact_type);
                    let uniform = self.artifacts.iter().all(|a| Some(&a.artifact_type) == first_type);
                    if !uniform {
                        warnings.push("Artifacts are not of uniform type".to_string());
                    }
                }
            }
        }
        
        ContractValidationResult {
            valid: violations.is_empty(),
            violations,
            warnings,
            completion_pct,
            artifacts_by_status: by_status,
        }
    }

    /// Check if all artifacts are complete
    pub fn is_complete(&self) -> bool {
        self.artifacts.iter().all(|a| {
            matches!(a.status, ArtifactStatus::Validated | ArtifactStatus::Drafted)
        })
    }

    /// Get artifacts ready for generation (missing or failed)
    pub fn pending_artifacts(&self) -> Vec<&RequiredArtifact> {
        self.artifacts.iter()
            .filter(|a| matches!(a.status, ArtifactStatus::Missing | ArtifactStatus::Failed))
            .collect()
    }

    /// Get execution order considering dependencies
    pub fn execution_order(&self) -> Vec<usize> {
        let mut order = vec![];
        let mut completed = std::collections::HashSet::new();
        
        // Keep iterating until all artifacts are ordered
        while order.len() < self.artifacts.len() {
            let mut added = false;
            for (idx, artifact) in self.artifacts.iter().enumerate() {
                if completed.contains(&idx) {
                    continue;
                }
                // Check if dependencies are satisfied
                let deps_satisfied = artifact.dependencies.iter().all(|d| completed.contains(&(d - 1)));
                if deps_satisfied || artifact.dependencies.is_empty() {
                    order.push(idx);
                    completed.insert(idx);
                    added = true;
                }
            }
            if !added {
                // Circular dependency - break by adding remaining
                for (idx, _) in self.artifacts.iter().enumerate() {
                    if !completed.contains(&idx) {
                        order.push(idx);
                    }
                }
                break;
            }
        }
        
        order
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TestCoverageLevel {
    Basic,         // 2 test files
    Standard,      // 4 test files
    Comprehensive, // 7 test files
}

/// Extract artifact contract from natural language prompt
pub fn extract_contract_from_prompt(
    prompt: &str,
    repo_path: impl Into<PathBuf>,
) -> Option<ArtifactContract> {
    let lower = prompt.to_lowercase();
    let root = repo_path.into();
    
    // Check for canonical 15 docs pattern
    if (lower.contains("15") || lower.contains("fifteen"))
        && lower.contains("doc")
        && (lower.contains("canonical") || lower.contains("standard"))
    {
        return Some(ArtifactContract::canonical_15_docs(root));
    }
    
    // Check for SDK generation pattern
    let sdk_languages = ["rust", "python", "javascript", "typescript", "go", "java", "csharp", "ruby"];
    let requested_langs: Vec<&str> = sdk_languages.iter()
        .filter(|lang| lower.contains(*lang))
        .copied()
        .collect();
    
    if !requested_langs.is_empty() && lower.contains("sdk") {
        return Some(ArtifactContract::multi_language_sdk(root, &requested_langs));
    }
    
    // Check for test suite pattern
    if lower.contains("test") && (lower.contains("suite") || lower.contains("comprehensive")) {
        let coverage = if lower.contains("comprehensive") {
            TestCoverageLevel::Comprehensive
        } else if lower.contains("basic") {
            TestCoverageLevel::Basic
        } else {
            TestCoverageLevel::Standard
        };
        return Some(ArtifactContract::test_suite(root, coverage));
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_15_docs() {
        let contract = ArtifactContract::canonical_15_docs("/tmp/test");
        assert_eq!(contract.artifacts.len(), 15);
        assert_eq!(contract.confidence, 95);
        
        // Check first artifact
        let first = &contract.artifacts[0];
        assert!(first.path.to_string_lossy().contains("01_PROJECT_OVERVIEW.md"));
        assert_eq!(first.step_number, 1);
    }

    #[test]
    fn test_multi_language_sdk() {
        let contract = ArtifactContract::multi_language_sdk("/tmp/test", &["rust", "python", "go"]);
        assert_eq!(contract.artifacts.len(), 3);
        
        // Check Rust artifact
        let rust = contract.artifacts.iter().find(|a| {
            matches!(a.artifact_type, ArtifactType::Code { language } if language == "rust")
        });
        assert!(rust.is_some());
        assert!(rust.unwrap().path.to_string_lossy().contains("lib.rs"));
    }

    #[test]
    fn test_execution_order() {
        let contract = ArtifactContract::canonical_15_docs("/tmp/test");
        let order = contract.execution_order();
        assert_eq!(order.len(), 15);
        // All step numbers 0-14 should be present
        for i in 0..15 {
            assert!(order.contains(&i));
        }
    }

    #[test]
    fn test_validate_complete_contract() {
        let mut contract = ArtifactContract::canonical_15_docs("/tmp/test");
        // Mark all as validated
        for artifact in &mut contract.artifacts {
            artifact.status = ArtifactStatus::Validated;
        }
        
        let result = contract.validate();
        assert!(result.valid);
        assert_eq!(result.completion_pct, 100);
    }

    #[test]
    fn test_extract_contract_from_prompt() {
        let prompt = "Create exactly 15 canonical documentation files for the project";
        let contract = extract_contract_from_prompt(prompt, "/tmp/test");
        assert!(contract.is_some());
        assert_eq!(contract.unwrap().artifacts.len(), 15);
    }
}
