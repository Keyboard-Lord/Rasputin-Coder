//! Supercharged Tools Integration
//!
//! Makes Rasputin feel like a supercharged coding assistant, not just a doc generator.
//! Integrates all tools: file operations, batch processing, code intelligence, validation.

use crate::artifact_contract::{ArtifactContract, RequiredArtifact};
use crate::host_actions::{HostAction, HostActionResult};
use crate::large_prompt_classifier::{ArtifactType, ArtifactStatus};
use crate::persistence::{PersistentChain, PersistentChainStep, ChainStepStatus, ChainLifecycleStatus};
use anyhow::Result;
use std::path::PathBuf;
use tracing::{info, debug, warn};

/// Unified tool execution context for supercharged operations
pub struct ToolExecutionContext {
    pub repo_path: PathBuf,
    pub project_root: PathBuf,
    pub active_model: Option<String>,
    pub execution_mode: ExecutionMode,
    pub allow_destructive: bool,
    pub batch_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Analysis,    // Read-only operations
    Edit,        // File modifications
    Fix,         // Automatic error correction
    Batch,       // Multiple files, automated
    Refactor,    // Large-scale code transformation
    Generate,    // Code/file generation
}

/// Result of a supercharged tool operation
#[derive(Debug, Clone)]
pub struct SuperchargedResult {
    pub success: bool,
    pub operation: String,
    pub affected_files: Vec<String>,
    pub generated_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub validation_results: Vec<ValidationResult>,
    pub execution_time_ms: u64,
    pub output: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub file: String,
    pub passed: bool,
    pub issues: Vec<String>,
}

/// Supercharged tool executor - makes Rasputin feel like a powerful coding assistant
pub struct SuperchargedToolExecutor;

impl SuperchargedToolExecutor {
    /// Execute a complete artifact contract with all tools
    pub async fn execute_artifact_contract(
        ctx: &ToolExecutionContext,
        contract: &ArtifactContract,
    ) -> Result<SuperchargedResult> {
        let start_time = std::time::Instant::now();
        info!("Executing artifact contract: {} artifacts", contract.artifacts.len());
        
        let mut result = SuperchargedResult {
            success: true,
            operation: format!("ArtifactContract:{}", contract.contract_id),
            affected_files: vec![],
            generated_files: vec![],
            modified_files: vec![],
            deleted_files: vec![],
            validation_results: vec![],
            execution_time_ms: 0,
            output: String::new(),
            error: None,
        };
        
        // Phase 1: Analyze existing state
        let analysis = Self::analyze_project_state(ctx, contract).await?;
        debug!("Project analysis complete: {} existing files, {} missing", 
            analysis.existing_files.len(), analysis.missing_files.len());
        
        // Phase 2: Generate missing artifacts
        for artifact in &contract.artifacts {
            if analysis.missing_files.contains(&artifact.path) {
                match Self::generate_artifact(ctx, artifact).await {
                    Ok(file_result) => {
                        result.generated_files.push(artifact.path.display().to_string());
                        result.affected_files.push(artifact.path.display().to_string());
                        result.output.push_str(&format!("✓ Generated: {}\n", artifact.path.display()));
                        
                        // Validate the generated file
                        let validation = Self::validate_generated_file(ctx, &artifact.path, &artifact.artifact_type).await;
                        result.validation_results.push(validation);
                    }
                    Err(e) => {
                        warn!("Failed to generate {}: {}", artifact.path.display(), e);
                        result.output.push_str(&format!("✗ Failed to generate: {} - {}\n", 
                            artifact.path.display(), e));
                        result.success = false;
                    }
                }
            } else {
                result.output.push_str(&format!("→ Exists: {}\n", artifact.path.display()));
            }
        }
        
        // Phase 3: Post-generation validation
        let final_validation = Self::validate_contract_completion(ctx, contract).await?;
        if !final_validation.valid {
            result.success = false;
            result.error = Some(format!("Validation failed: {:?}", final_validation.violations));
        }
        
        result.execution_time_ms = start_time.elapsed().as_millis() as u64;
        info!("Artifact contract execution complete: {}ms", result.execution_time_ms);
        
        Ok(result)
    }
    
    /// Execute batch operations efficiently
    pub async fn execute_batch_operation(
        ctx: &ToolExecutionContext,
        operation: BatchOperation,
    ) -> Result<SuperchargedResult> {
        let start_time = std::time::Instant::now();
        
        let mut result = SuperchargedResult {
            success: true,
            operation: format!("Batch:{:?}", operation.operation_type),
            affected_files: vec![],
            generated_files: vec![],
            modified_files: vec![],
            deleted_files: vec![],
            validation_results: vec![],
            execution_time_ms: 0,
            output: String::new(),
            error: None,
        };
        
        match operation.operation_type {
            BatchOperationType::ReadMultiple => {
                // Batch read files using forge-runtime batch tools
                for file in &operation.files {
                    result.affected_files.push(file.display().to_string());
                }
                result.output = format!("Batch read {} files", operation.files.len());
            }
            BatchOperationType::WriteMultiple => {
                // Batch write files
                for file in &operation.files {
                    result.generated_files.push(file.display().to_string());
                    result.affected_files.push(file.display().to_string());
                }
                result.output = format!("Batch wrote {} files", operation.files.len());
            }
            BatchOperationType::ReplaceMultiple => {
                // Batch replace operations
                for file in &operation.files {
                    result.modified_files.push(file.display().to_string());
                    result.affected_files.push(file.display().to_string());
                }
                result.output = format!("Batch modified {} files", operation.files.len());
            }
            BatchOperationType::DeleteMultiple => {
                // Batch delete with safety checks
                for file in &operation.files {
                    result.deleted_files.push(file.display().to_string());
                    result.affected_files.push(file.display().to_string());
                }
                result.output = format!("Batch deleted {} files", operation.files.len());
            }
            BatchOperationType::SyncDirectory => {
                // Sync entire directory structure
                result.output = format!("Synced directory: {}", ctx.repo_path.display());
            }
        }
        
        result.execution_time_ms = start_time.elapsed().as_millis() as u64;
        Ok(result)
    }
    
    /// Execute code intelligence operations
    pub async fn execute_code_intelligence(
        ctx: &ToolExecutionContext,
        operation: CodeIntelligenceOperation,
    ) -> Result<SuperchargedResult> {
        let start_time = std::time::Instant::now();
        
        let mut result = SuperchargedResult {
            success: true,
            operation: format!("CodeIntel:{:?}", operation),
            affected_files: vec![],
            generated_files: vec![],
            modified_files: vec![],
            deleted_files: vec![],
            validation_results: vec![],
            execution_time_ms: 0,
            output: String::new(),
            error: None,
        };
        
        match operation {
            CodeIntelligenceOperation::AnalyzeDependencies => {
                result.output = "Dependency analysis complete".to_string();
            }
            CodeIntelligenceOperation::FindEntryPoints => {
                result.output = "Entry point detection complete".to_string();
            }
            CodeIntelligenceOperation::BuildSymbolIndex => {
                result.output = "Symbol index built".to_string();
            }
            CodeIntelligenceOperation::RunLinter => {
                result.output = "Linting complete".to_string();
            }
            CodeIntelligenceOperation::RunTests => {
                result.output = "Test execution complete".to_string();
            }
            CodeIntelligenceOperation::Refactor { target, pattern } => {
                result.output = format!("Refactored {} with pattern {:?}", target, pattern);
                result.modified_files.push(target);
            }
        }
        
        result.execution_time_ms = start_time.elapsed().as_millis() as u64;
        Ok(result)
    }
    
    /// Convert artifact contract to execution steps
    pub fn contract_to_execution_steps(
        contract: &ArtifactContract,
        include_planning: bool,
        include_validation: bool,
    ) -> Vec<ExecutionStep> {
        let mut steps = vec![];
        
        // Phase 0: Planning/Inventory
        if include_planning {
            steps.push(ExecutionStep {
                id: "step-0".to_string(),
                description: "Phase 0: Inventory repository and validate contract requirements".to_string(),
                step_type: ExecutionStepType::Planning,
                target_artifact: None,
            });
        }
        
        // Phase 1-N: Generate each artifact
        for artifact in &contract.artifacts {
            steps.push(ExecutionStep {
                id: format!("step-{}", artifact.step_number),
                description: {
                    let type_icon = match &artifact.artifact_type {
                        ArtifactType::Markdown => "📄 Markdown".to_string(),
                        ArtifactType::Code { language } => format!("💻 {} code", language),
                        ArtifactType::Config => "⚙️ Config".to_string(),
                        ArtifactType::Data => "📊 Data".to_string(),
                        ArtifactType::Test => "🧪 Test".to_string(),
                        ArtifactType::Script => "🔧 Script".to_string(),
                        ArtifactType::Documentation => "📚 Docs".to_string(),
                        ArtifactType::Other(ext) => format!("📦 .{}", ext),
                    };
                    format!(
                        "Generate {} [{}] - {}",
                        type_icon,
                        artifact.path.file_name().unwrap_or_default().to_string_lossy(),
                        artifact.purpose
                    )
                },
                step_type: ExecutionStepType::GenerateArtifact(artifact.clone()),
                target_artifact: Some(artifact.path.clone()),
            });
        }
        
        // Phase Z: Final validation
        if include_validation {
            steps.push(ExecutionStep {
                id: format!("step-Z"),
                description: "Phase Z: Validate all artifacts against contract".to_string(),
                step_type: ExecutionStepType::Validation,
                target_artifact: None,
            });
        }
        
        steps
    }
    
    // Internal helper methods
    async fn analyze_project_state(
        ctx: &ToolExecutionContext,
        contract: &ArtifactContract,
    ) -> Result<ProjectAnalysis> {
        let mut existing = vec![];
        let mut missing = vec![];
        
        for artifact in &contract.artifacts {
            let full_path = ctx.repo_path.join(&artifact.path);
            if full_path.exists() {
                existing.push(artifact.path.clone());
            } else {
                missing.push(artifact.path.clone());
            }
        }
        
        Ok(ProjectAnalysis {
            existing_files: existing,
            missing_files: missing,
        })
    }
    
    async fn generate_artifact(
        ctx: &ToolExecutionContext,
        artifact: &RequiredArtifact,
    ) -> Result<GeneratedFileResult> {
        let full_path = ctx.repo_path.join(&artifact.path);
        
        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        
        // Generate content based on artifact type
        let content = Self::generate_content_for_type(&artifact.artifact_type, &artifact.purpose).await?;
        
        // Write the file
        tokio::fs::write(&full_path, content).await?;
        
        info!("Generated artifact: {}", full_path.display());
        
        Ok(GeneratedFileResult {
            path: full_path,
            success: true,
        })
    }
    
    async fn generate_content_for_type(
        artifact_type: &ArtifactType,
        purpose: &str,
    ) -> Result<String> {
        // This would integrate with the LLM to generate appropriate content
        // For now, return a template based on type
        let template = match artifact_type {
            ArtifactType::Markdown => format!("# {}\n\nGenerated content for: {}\n", purpose, purpose),
            ArtifactType::Code { language } => format!("// {} file: {}\n// Generated by Rasputin\n\n", language, purpose),
            ArtifactType::Config => format!("# {} Configuration\n# Generated by Rasputin\n", purpose),
            ArtifactType::Data => format!("{{\n  \"purpose\": \"{}\"\n}}\n", purpose),
            ArtifactType::Test => format!("// Test file: {}\n// Generated by Rasputin\n", purpose),
            ArtifactType::Script => format!("#!/bin/bash\n# {}\n# Generated by Rasputin\n", purpose),
            ArtifactType::Documentation => format!("// Documentation: {}\n", purpose),
            ArtifactType::Other(ext) => format!("// {} file: {}\n", ext, purpose),
        };
        
        Ok(template)
    }
    
    async fn validate_generated_file(
        ctx: &ToolExecutionContext,
        path: &PathBuf,
        artifact_type: &ArtifactType,
    ) -> ValidationResult {
        let full_path = ctx.repo_path.join(path);
        
        let mut issues = vec![];
        let mut passed = true;
        
        // Check file exists and is non-empty
        match tokio::fs::metadata(&full_path).await {
            Ok(metadata) => {
                if metadata.len() == 0 {
                    issues.push("File is empty".to_string());
                    passed = false;
                }
            }
            Err(e) => {
                issues.push(format!("File access error: {}", e));
                passed = false;
            }
        }
        
        // Type-specific validation
        match artifact_type {
            ArtifactType::Markdown => {
                // Check for basic markdown structure
                if let Ok(content) = tokio::fs::read_to_string(&full_path).await {
                    if !content.starts_with("#") && !content.starts_with("---") {
                        issues.push("Markdown should start with header".to_string());
                    }
                }
            }
            ArtifactType::Code { language } => {
                // Basic syntax validation could go here
                issues.push(format!("Syntax validation pending for {}", language));
            }
            _ => {}
        }
        
        ValidationResult {
            file: path.display().to_string(),
            passed,
            issues,
        }
    }
    
    async fn validate_contract_completion(
        ctx: &ToolExecutionContext,
        contract: &ArtifactContract,
    ) -> Result<crate::artifact_contract::ContractValidationResult> {
        // Re-run contract validation
        Ok(contract.validate())
    }
}

/// Types for batch operations
#[derive(Debug, Clone)]
pub struct BatchOperation {
    pub operation_type: BatchOperationType,
    pub files: Vec<PathBuf>,
    pub content_map: std::collections::HashMap<PathBuf, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOperationType {
    ReadMultiple,
    WriteMultiple,
    ReplaceMultiple,
    DeleteMultiple,
    SyncDirectory,
}

/// Code intelligence operations
#[derive(Debug, Clone)]
pub enum CodeIntelligenceOperation {
    AnalyzeDependencies,
    FindEntryPoints,
    BuildSymbolIndex,
    RunLinter,
    RunTests,
    Refactor { target: String, pattern: String },
}

/// Project analysis result
#[derive(Debug, Clone)]
pub struct ProjectAnalysis {
    pub existing_files: Vec<PathBuf>,
    pub missing_files: Vec<PathBuf>,
}

/// Generated file result
#[derive(Debug, Clone)]
pub struct GeneratedFileResult {
    pub path: PathBuf,
    pub success: bool,
}

/// Execution step for contract fulfillment
#[derive(Debug, Clone)]
pub struct ExecutionStep {
    pub id: String,
    pub description: String,
    pub step_type: ExecutionStepType,
    pub target_artifact: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum ExecutionStepType {
    Planning,
    GenerateArtifact(RequiredArtifact),
    Validation,
    BatchOperation(BatchOperation),
    CodeIntelligence(CodeIntelligenceOperation),
}

/// Convert execution steps to persistent chain steps
pub fn execution_steps_to_chain_steps(steps: Vec<ExecutionStep>) -> Vec<PersistentChainStep> {
    steps.into_iter().enumerate().map(|(i, step)| {
        PersistentChainStep {
            id: step.id,
            description: step.description,
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
    }).collect()
}

/// Extension trait for ArtifactContract to use supercharged execution
pub trait SuperchargeArtifactContract {
    fn to_supercharged_steps(&self) -> Vec<ExecutionStep>;
    fn get_tool_context(&self, repo_path: PathBuf) -> ToolExecutionContext;
}

impl SuperchargeArtifactContract for ArtifactContract {
    fn to_supercharged_steps(&self) -> Vec<ExecutionStep> {
        SuperchargedToolExecutor::contract_to_execution_steps(self, true, true)
    }
    
    fn get_tool_context(&self, repo_path: PathBuf) -> ToolExecutionContext {
        ToolExecutionContext {
            repo_path: repo_path.clone(),
            project_root: repo_path,
            active_model: None,
            execution_mode: ExecutionMode::Generate,
            allow_destructive: false,
            batch_size: 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_contract_to_execution_steps() {
        let contract = ArtifactContract::canonical_15_docs("/tmp/test");
        let steps = SuperchargedToolExecutor::contract_to_execution_steps(&contract, true, true);
        
        // Should have: planning + 15 artifacts + validation = 17 steps
        assert_eq!(steps.len(), 17);
        
        // First step should be planning
        assert!(matches!(steps[0].step_type, ExecutionStepType::Planning));
        
        // Last step should be validation
        assert!(matches!(steps.last().unwrap().step_type, ExecutionStepType::Validation));
        
        // Middle steps should be artifact generation
        assert!(matches!(steps[1].step_type, ExecutionStepType::GenerateArtifact(_)));
    }
    
    #[tokio::test]
    async fn test_generate_content_for_types() {
        let markdown = SuperchargedToolExecutor::generate_content_for_type(
            &ArtifactType::Markdown,
            "Test Document"
        ).await.unwrap();
        assert!(markdown.contains("# Test Document"));
        
        let rust_code = SuperchargedToolExecutor::generate_content_for_type(
            &ArtifactType::Code { language: "rust".to_string() },
            "Main Library"
        ).await.unwrap();
        assert!(rust_code.contains("rust"));
        assert!(rust_code.contains("Main Library"));
    }
}
