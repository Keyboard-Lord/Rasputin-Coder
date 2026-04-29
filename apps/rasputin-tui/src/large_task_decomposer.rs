//! Large Task Decomposer
//!
//! Breaks large multi-artifact prompts into executable, bounded steps.
//! Never sends huge prompts directly to Forge - always decompose first.

use crate::artifact_contract::{ArtifactContract, RequiredArtifact, EffortLevel};
use crate::large_prompt_classifier::{LargePromptClassifier, PromptClassification};
use crate::persistence::{PersistentChain, PersistentChainStep, ChainStepStatus, ChainLifecycleStatus};
use std::path::Path;
use tracing::{info, debug, warn};

/// Result of decomposing a large task
#[derive(Debug, Clone)]
pub struct DecomposedTask {
    pub original_prompt: String,
    pub contract: ArtifactContract,
    pub chain: PersistentChain,
    pub steps: Vec<DecomposedStep>,
    pub strategy: DecompositionStrategy,
}

#[derive(Debug, Clone)]
pub struct DecomposedStep {
    pub step_number: usize,
    pub description: String,
    pub prompt: String,
    pub artifact: Option<RequiredArtifact>,
    pub step_type: StepType,
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepType {
    Planning,           // Inventory and source map
    SourceMapping,      // Build repository understanding
    ArtifactGeneration, // Create one specific artifact
    Validation,         // Verify contract fulfillment
    Refinement,         // Improve existing artifacts
    Recovery,           // Self-correction retry step
}

/// Recovery strategy for failed steps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStrategy {
    SimplifyPrompt,     // Reduce complexity, strip constraints
    AddExamples,        // Include example output format
    AlternativeApproach, // Try different angle/perspective
    HumanEscalation,    // Flag for user intervention
}

/// Self-correction configuration
#[derive(Debug, Clone)]
pub struct SelfCorrectionConfig {
    pub max_retries: usize,
    pub enable_auto_recovery: bool,
    pub recovery_strategies: Vec<RecoveryStrategy>,
}

impl Default for SelfCorrectionConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            enable_auto_recovery: true,
            recovery_strategies: vec![
                RecoveryStrategy::SimplifyPrompt,
                RecoveryStrategy::AddExamples,
                RecoveryStrategy::AlternativeApproach,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecompositionStrategy {
    Sequential,     // One at a time, dependencies first
    ParallelSafe,   // Can run independent artifacts together
    DependencyTree, // Respect explicit dependencies
}

/// Large Task Decomposer - the core engine
pub struct LargeTaskDecomposer;

impl LargeTaskDecomposer {
    /// Main entry point: decompose a prompt into executable chain
    pub fn decompose(
        prompt: &str,
        repo_path: impl AsRef<Path>,
    ) -> Option<DecomposedTask> {
        debug!("Attempting to decompose prompt: {} chars", prompt.len());
        
        // Step 1: Classify the prompt
        let classification = LargePromptClassifier::classify(prompt, Some(repo_path.as_ref()));
        
        match classification {
            PromptClassification::LargeProject(contract) => {
                info!("Detected large project with {} artifacts", contract.artifacts.len());
                
                // Step 2: Determine strategy
                let strategy = Self::determine_strategy(&contract);
                
                // Step 3: Decompose into steps
                let steps = Self::create_steps(&contract, &strategy);
                
                // Step 4: Create persistent chain
                let chain = Self::create_chain(&contract, &steps, repo_path.as_ref());
                
                Some(DecomposedTask {
                    original_prompt: prompt.to_string(),
                    contract,
                    chain,
                    steps,
                    strategy,
                })
            }
            PromptClassification::Regular => {
                debug!("Prompt classified as regular - no decomposition needed");
                None
            }
            PromptClassification::Ambiguous { reason } => {
                warn!("Ambiguous classification: {}", reason);
                None
            }
        }
    }
    
    /// Determine the best decomposition strategy
    fn determine_strategy(contract: &ArtifactContract) -> DecompositionStrategy {
        // Check for dependencies
        let has_deps = contract.artifacts.iter().any(|a| !a.dependencies.is_empty());
        
        if has_deps {
            DecompositionStrategy::DependencyTree
        } else if contract.artifacts.len() > 10 {
            // Large projects benefit from parallel where safe
            DecompositionStrategy::ParallelSafe
        } else {
            DecompositionStrategy::Sequential
        }
    }
    
    /// Create decomposed steps from contract
    fn create_steps(
        contract: &ArtifactContract,
        strategy: &DecompositionStrategy,
    ) -> Vec<DecomposedStep> {
        let mut steps = vec![];
        let mut step_num = 0;
        
        // Phase 0: Planning and inventory
        step_num += 1;
        steps.push(DecomposedStep {
            step_number: step_num,
            description: "Phase 0: Inventory repository and analyze structure".to_string(),
            prompt: Self::build_planning_prompt(contract),
            artifact: None,
            step_type: StepType::Planning,
            estimated_tokens: 2000,
        });
        
        // Phase 1: Source mapping (only for very large repos >10 artifacts)
        if contract.artifacts.len() > 10 {
            step_num += 1;
            steps.push(DecomposedStep {
                step_number: step_num,
                description: "Phase 1: Map key modules and APIs".to_string(),
                prompt: Self::build_source_mapping_prompt(contract),
                artifact: None,
                step_type: StepType::SourceMapping,
                estimated_tokens: 2000,
            });
        }
        
        // Phase 2-N: Generate each artifact
        // Order depends on strategy
        let artifact_order = match strategy {
            DecompositionStrategy::DependencyTree => {
                contract.execution_order()
            }
            DecompositionStrategy::Sequential => {
                (0..contract.artifacts.len()).collect()
            }
            DecompositionStrategy::ParallelSafe => {
                // Sort by effort (smaller first for quick wins)
                let mut indexed: Vec<_> = contract.artifacts.iter()
                    .enumerate()
                    .map(|(i, a)| (i, a.estimated_effort))
                    .collect();
                indexed.sort_by_key(|(_, effort)| *effort);
                indexed.into_iter().map(|(i, _)| i).collect()
            }
        };
        
        for artifact_idx in artifact_order {
            let artifact = &contract.artifacts[artifact_idx];
            step_num += 1;
            
            let prompt = Self::build_artifact_prompt(contract, artifact);
            let estimated_tokens = Self::estimate_tokens(&prompt);
            
            steps.push(DecomposedStep {
                step_number: step_num,
                description: format!(
                    "Phase {}: Create {}",
                    step_num,
                    artifact.path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| format!("artifact-{}", artifact_idx))
                ),
                prompt,
                artifact: Some(artifact.clone()),
                step_type: StepType::ArtifactGeneration,
                estimated_tokens,
            });
        }
        
        // Phase Z: Final validation
        step_num += 1;
        steps.push(DecomposedStep {
            step_number: step_num,
            description: "Phase Z: Validate all artifacts against contract".to_string(),
            prompt: Self::build_validation_prompt(contract),
            artifact: None,
            step_type: StepType::Validation,
            estimated_tokens: 1500,
        });
        
        steps
    }
    
    /// Create persistent chain from decomposed steps
    fn create_chain(
        contract: &ArtifactContract,
        steps: &[DecomposedStep],
        repo_path: &Path,
    ) -> PersistentChain {
        let now = chrono::Local::now();
        
        let chain_steps: Vec<PersistentChainStep> = steps.iter().map(|step| {
            PersistentChainStep {
                id: format!("step-{}", step.step_number),
                description: step.description.clone(),
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
        
        let chain_id = format!("decomposed-{}", uuid::Uuid::new_v4());
        
        PersistentChain {
            id: chain_id,
            name: format!("{} artifacts: {}", contract.artifacts.len(), contract.source_prompt_summary.chars().take(50).collect::<String>()),
            objective: contract.source_prompt_summary.clone(),
            raw_prompt: format!(
                "DECOMPOSED TASK\nContract: {}\nStrategy: {:?}\nArtifacts: {}\n\nOriginal:\n{}",
                contract.contract_id,
                steps.iter().map(|s| &s.step_type).collect::<Vec<_>>(),
                contract.artifacts.len(),
                contract.source_prompt_summary
            ),
            status: ChainLifecycleStatus::Ready,
            steps: chain_steps,
            active_step: None,
            repo_path: Some(repo_path.display().to_string()),
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
    
    /// Build planning phase prompt - COMPACT
    fn build_planning_prompt(contract: &ArtifactContract) -> String {
        format!(
            "PHASE 0: INVENTORY (Step 1 of {} total steps)\n\n\
            Quick repo scan. List:\n\
            - Top-level directories\n\
            - Main technologies used\n\
            - Key source files\n\n\
            Output: Brief summary to guide the {} artifact generations.",
            contract.artifacts.len() + 2, // +2 for inventory and validation
            contract.artifacts.len()
        )
    }
    
    /// Build source mapping phase prompt - COMPACT
    fn build_source_mapping_prompt(_contract: &ArtifactContract) -> String {
        format!(
            "PHASE 1: SOURCE MAP\n\n\
            Map the codebase:\n\
            - Main modules and their purposes\n\
            - Public APIs/interfaces\n\
            - Configuration points\n\n\
            Output: Summary to guide artifact generation."
        )
    }
    
    /// Build artifact generation prompt - COMPACTED to ≤3000 chars, ONE FILE ONLY
    fn build_artifact_prompt(
        contract: &ArtifactContract,
        artifact: &RequiredArtifact,
    ) -> String {
        let filename = artifact.path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "artifact".to_string());
        
        // Compact, focused prompt ≤3000 characters
        let prompt = format!(
            "Create file: {}\n\n\
            Purpose: {}\n\n\
            READ FIRST:\n\
            - README.md, package.json, Cargo.toml (if exist)\n\
            - src/ directory structure\n\
            - Any existing docs/ files\n\n\
            THEN:\n\
            1. Write content strictly based on repository analysis\n\
            2. Follow existing project patterns\n\
            3. Output ONLY the file content\n\
            4. NO markdown code blocks\n\
            5. NO explanations\n\n\
            HARD CONSTRAINTS:\n\
            - Target ONLY this single file\n\
            - Do NOT modify source code\n\
            - Do NOT create other files\n\n\
            File {}/{}",
            artifact.path.display(),
            artifact.purpose,
            artifact.step_number,
            contract.artifacts.len()
        );
        
        // Ensure ≤3000 chars
        if prompt.len() > 3000 {
            format!(
                "Create: {}\n\n\
                Purpose: {}\n\n\
                Read repo files first. Write ONLY this file. No other changes. File {}/{}",
                artifact.path.display(),
                artifact.purpose,
                artifact.step_number,
                contract.artifacts.len()
            )
        } else {
            prompt
        }
    }
    
    /// Build validation phase prompt - COMPACT
    fn build_validation_prompt(contract: &ArtifactContract) -> String {
        format!(
            "PHASE Z: VALIDATION (Final Step)\n\n\
            Check:\n\
            1. All {} artifacts exist and are non-empty\n\
            2. No source code was modified\n\
            3. Content is repo-grounded\n\n\
            Required files:\n{}\n\n\
            Report: ✓ Complete or list issues.",
            contract.artifacts.len(),
            contract.artifacts.iter()
                .map(|a| format!("- {}\n", a.path.file_name().unwrap_or_default().to_string_lossy()))
                .collect::<String>()
        )
    }
    
    /// Estimate tokens for a prompt
    fn estimate_tokens(prompt: &str) -> usize {
        // Rough approximation: ~4 chars per token
        (prompt.len() / 4).max(500).min(8000)
    }
    
    /// Check if a prompt needs decomposition
    pub fn needs_decomposition(prompt: &str) -> bool {
        let classification = LargePromptClassifier::classify(prompt, None);
        matches!(classification, PromptClassification::LargeProject(_))
    }
    
    /// Quick check for artifact count in prompt
    pub fn estimate_artifact_count(prompt: &str) -> usize {
        // Count numbered items with file extensions
        prompt.lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed.chars().next().map(|c| c.is_numeric()).unwrap_or(false)
                    && trimmed.contains('.')
            })
            .count()
    }

    /// Create decomposed task directly from an artifact contract
    pub fn from_contract(contract: ArtifactContract) -> DecomposedTask {
        Self::decompose_from_contract(contract, DecompositionStrategy::Sequential)
    }

    /// Decompose from an existing contract with chosen strategy
    fn decompose_from_contract(
        contract: ArtifactContract,
        strategy: DecompositionStrategy,
    ) -> DecomposedTask {
        use crate::persistence::{ChainLifecycleStatus, ChainStepStatus, PersistentChain, PersistentChainStep};
        use chrono::Local;
        
        let _root_dir = contract.root_dir.clone();
        let original_prompt = contract.source_prompt_summary.clone();
        
        // Build steps from contract
        let mut steps: Vec<DecomposedStep> = vec![];
        let mut step_num = 0;
        
        // Phase 0: Planning
        step_num += 1;
        steps.push(DecomposedStep {
            step_number: step_num,
            description: "Phase 0: Inventory repository and analyze structure".to_string(),
            prompt: Self::build_planning_prompt(&contract),
            artifact: None,
            step_type: StepType::Planning,
            estimated_tokens: 2000,
        });
        
        // Phase 1: Source mapping (only for very large repos >10 artifacts)
        if contract.artifacts.len() > 10 {
            step_num += 1;
            steps.push(DecomposedStep {
                step_number: step_num,
                description: "Phase 1: Map key modules and APIs".to_string(),
                prompt: Self::build_source_mapping_prompt(&contract),
                artifact: None,
                step_type: StepType::SourceMapping,
                estimated_tokens: 2000,
            });
        }
        
        // Phase 2-N: Generate each artifact
        let artifact_indices: Vec<usize> = match strategy {
            DecompositionStrategy::DependencyTree => {
                contract.execution_order()
            }
            _ => (0..contract.artifacts.len()).collect(),
        };
        
        for idx in artifact_indices {
            if let Some(artifact) = contract.artifacts.get(idx) {
                step_num += 1;
                steps.push(DecomposedStep {
                    step_number: step_num,
                    description: format!("Create {}", artifact.path.display()),
                    prompt: Self::build_artifact_prompt(&contract, artifact),
                    artifact: Some(artifact.clone()),
                    step_type: StepType::ArtifactGeneration,
                    estimated_tokens: 2500,
                });
            }
        }
        
        // Phase Z: Validation
        step_num += 1;
        steps.push(DecomposedStep {
            step_number: step_num,
            description: "Validate contract fulfillment".to_string(),
            prompt: Self::build_validation_prompt(&contract),
            artifact: None,
            step_type: StepType::Validation,
            estimated_tokens: 1500,
        });
        
        let now = Local::now();
        
        // Create persistent chain with all required fields
        let chain = PersistentChain {
            id: format!("decomposed-{}", uuid::Uuid::new_v4()),
            name: format!("Decomposed: {} artifacts", contract.artifacts.len()),
            objective: contract.source_prompt_summary.clone(),
            raw_prompt: original_prompt.clone(),
            status: ChainLifecycleStatus::Draft,
            steps: steps.iter().enumerate().map(|(i, step)| {
                PersistentChainStep {
                    id: format!("step-{}", i + 1),
                    description: step.description.clone(),
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
            }).collect(),
            active_step: Some(0),
            repo_path: Some(contract.root_dir.to_string_lossy().to_string()),
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
        };
        
        DecomposedTask {
            original_prompt,
            contract,
            steps,
            chain,
            strategy,
        }
    }

    /// Generate recovery step for a failed artifact generation
    pub fn create_recovery_step(
        failed_step: &DecomposedStep,
        artifact: &RequiredArtifact,
        retry_count: usize,
        strategy: RecoveryStrategy,
    ) -> DecomposedStep {
        let recovery_prompt = Self::build_recovery_prompt(artifact, retry_count, strategy);
        
        DecomposedStep {
            step_number: failed_step.step_number, // Keep same position
            description: format!(
                "🔧 Recovery {} for {}",
                retry_count,
                artifact.path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "artifact".to_string())
            ),
            prompt: recovery_prompt,
            artifact: Some(artifact.clone()),
            step_type: StepType::Recovery,
            estimated_tokens: 1500,
        }
    }

    /// Build recovery prompt with adjusted strategy
    fn build_recovery_prompt(
        artifact: &RequiredArtifact,
        retry_count: usize,
        strategy: RecoveryStrategy,
    ) -> String {
        let base = format!(
            "RECOVERY ATTEMPT {} for: {}\n\n\
            Previous attempt failed. Try again with adjusted approach.\n\n",
            retry_count,
            artifact.path.display()
        );

        let strategy_instruction = match strategy {
            RecoveryStrategy::SimplifyPrompt => {
                "SIMPLIFIED APPROACH:\n\
                - Write ONLY the basic content\n\
                - Skip advanced sections\n\
                - Focus on core purpose only\n\
                - Keep it simple and direct"
            }
            RecoveryStrategy::AddExamples => {
                "WITH EXAMPLE FORMAT:\n\
                - Look at existing files in the repo for format examples\n\
                - Match the style of similar existing files\n\
                - Follow established patterns\n\
                - Use consistent structure"
            }
            RecoveryStrategy::AlternativeApproach => {
                "ALTERNATIVE APPROACH:\n\
                - Try a different structure\n\
                - Focus on different aspects\n\
                - Use bullet points instead of paragraphs if stuck\n\
                - Just get the key information down"
            }
            RecoveryStrategy::HumanEscalation => {
                "ESCALATION NEEDED:\n\
                This artifact requires human review.\n\
                Please check the requirements and try manually."
            }
        };

        format!(
            "{}\n{}\n\n\
            HARD CONSTRAINTS (still apply):\n\
            - Target ONLY this single file: {}\n\
            - Do NOT modify source code\n\
            - Output ONLY file content, no explanations",
            base,
            strategy_instruction,
            artifact.path.display()
        )
    }

    /// Generate all recovery strategies for a failed step
    pub fn generate_recovery_steps(
        failed_step: &DecomposedStep,
        artifact: &RequiredArtifact,
    ) -> Vec<DecomposedStep> {
        let config = SelfCorrectionConfig::default();
        let mut recovery_steps = vec![];

        for (i, strategy) in config.recovery_strategies.iter().enumerate() {
            recovery_steps.push(Self::create_recovery_step(
                failed_step,
                artifact,
                i + 1,
                *strategy,
            ));
        }

        recovery_steps
    }
}

/// Helper to format decomposed task for display
pub fn format_decomposition_summary(task: &DecomposedTask) -> String {
    let mut lines = vec![
        format!("🎯 Decomposed: {} artifacts", task.contract.artifacts.len()),
        format!("   Strategy: {:?}", task.strategy),
        format!("   Steps: {}", task.steps.len()),
        String::new(),
        "Execution Plan:".to_string(),
    ];
    
    for step in &task.steps {
        let icon = match step.step_type {
            StepType::Planning => "📋",
            StepType::SourceMapping => "🗺️",
            StepType::ArtifactGeneration => "⚡",
            StepType::Validation => "✓",
            StepType::Refinement => "🔧",
            StepType::Recovery => "🔄",
        };
        lines.push(format!(
            "   {} Step {}: {} ({} tokens)",
            icon,
            step.step_number,
            step.description.chars().take(50).collect::<String>(),
            step.estimated_tokens
        ));
    }
    
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_decompose_15_docs() {
        let prompt = r#"
Analyze the entire repository and create exactly 15 canonical documentation files:
1. docs/01_PROJECT_OVERVIEW.md
2. docs/02_ARCHITECTURE.md
...
15. docs/15_FUTURE_ROADMAP.md
"#;
        
        let task = LargeTaskDecomposer::decompose(prompt, "/tmp/test");
        assert!(task.is_some());
        
        let task = task.unwrap();
        assert_eq!(task.contract.artifacts.len(), 15);
        assert!(task.steps.len() > 15); // Planning + mapping + artifacts + validation
        
        // Check structure
        assert!(matches!(task.steps[0].step_type, StepType::Planning));
        assert!(matches!(task.steps.last().unwrap().step_type, StepType::Validation));
    }
    
    #[test]
    fn test_regular_prompt_no_decomposition() {
        let prompt = "How do I write a Rust function?";
        let task = LargeTaskDecomposer::decompose(prompt, "/tmp/test");
        assert!(task.is_none());
    }
    
    #[test]
    fn test_estimate_artifact_count() {
        let prompt = r#"
1. file1.rs
2. file2.py
3. file3.js
Something else
4. file4.md
"#;
        let count = LargeTaskDecomposer::estimate_artifact_count(prompt);
        assert_eq!(count, 4);
    }
}
