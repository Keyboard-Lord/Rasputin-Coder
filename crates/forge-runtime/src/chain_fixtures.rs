//! Chain Fixture Test Infrastructure
//!
//! Provides deterministic replay-grade testing for chain execution.
//! All chain behavior is tested via hand-authored fixtures, not live Ollama.

#[allow(unused_imports)]
use crate::types::{
    ChainId, StepOutcome, StepStatus, TaskChain, ValidationDecision, ValidationReport,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A chain step fixture defines the inputs and expected outputs for a single step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStepFixture {
    /// Step index in the chain
    pub index: usize,
    /// Description of what this step should do
    pub description: String,
    /// Simulated planner outputs for this step (in sequence)
    pub planner_outputs: Vec<PlannerOutputFixture>,
    /// Expected step outcome
    pub expected_outcome: StepOutcomeFixture,
    /// Whether this step should trigger checkpoint creation on success
    pub expect_checkpoint: bool,
    /// Simulated file mutations from tool calls
    pub file_mutations: Vec<FileMutationFixture>,
}

/// A planner output fixture (tool call or completion)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PlannerOutputFixture {
    #[serde(rename = "tool_call")]
    ToolCall {
        name: String,
        arguments: HashMap<String, String>,
    },
    #[serde(rename = "completion")]
    Completion { reason: String },
    #[serde(rename = "failure")]
    Failure { reason: String, recoverable: bool },
}

/// Expected step outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StepOutcomeFixture {
    #[serde(rename = "resolved")]
    Resolved {
        summary: String,
        files_modified: Vec<String>,
    },
    #[serde(rename = "failed")]
    Failed { reason: String, recoverable: bool },
    #[serde(rename = "blocked")]
    Blocked { reason: String },
}

impl StepOutcomeFixture {
    #[allow(dead_code)]
    pub fn to_outcome(&self, temp_dir: &std::path::Path) -> StepOutcome {
        match self {
            StepOutcomeFixture::Resolved {
                summary,
                files_modified,
            } => {
                let files: Vec<PathBuf> = files_modified.iter().map(|f| temp_dir.join(f)).collect();
                StepOutcome::Resolved {
                    summary: summary.clone(),
                    files_modified: files,
                }
            }
            StepOutcomeFixture::Failed {
                reason,
                recoverable,
            } => StepOutcome::Failed {
                reason: reason.clone(),
                recoverable: *recoverable,
            },
            StepOutcomeFixture::Blocked { reason } => StepOutcome::Blocked {
                reason: reason.clone(),
            },
        }
    }
}

/// File mutation for simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMutationFixture {
    pub path: String,
    pub content: String,
    pub should_fail_validation: Option<String>, // stage name if should fail
}

/// A complete chain fixture
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainFixture {
    /// Chain identifier
    pub chain_id: String,
    /// Chain objective
    pub objective: String,
    /// Total expected steps
    pub total_steps: usize,
    /// Step fixtures
    pub steps: Vec<ChainStepFixture>,
    /// Expected final status
    pub expected_final_status: ChainStatusFixture,
    /// Expected number of checkpoints created
    pub expected_checkpoints: usize,
    /// Initial workspace files (path -> content)
    pub initial_workspace: HashMap<String, String>,
    /// Expected final workspace files (path -> content)
    pub expected_final_workspace: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainStatusFixture {
    Complete,
    Failed,
    Pending,
    Running,
}

impl ChainStatusFixture {
    #[allow(dead_code)]
    pub fn to_chain_status(&self) -> crate::types::ChainStatus {
        match self {
            ChainStatusFixture::Complete => crate::types::ChainStatus::Complete,
            ChainStatusFixture::Failed => crate::types::ChainStatus::Failed,
            ChainStatusFixture::Pending => crate::types::ChainStatus::Pending,
            ChainStatusFixture::Running => crate::types::ChainStatus::Running,
        }
    }
}

/// Loader for chain fixtures
pub struct ChainFixtureLoader;

impl ChainFixtureLoader {
    /// Load a fixture from embedded JSON string
    pub fn load_from_str(json: &str) -> Result<ChainFixture, FixtureError> {
        serde_json::from_str(json).map_err(FixtureError::from)
    }

    /// Load a fixture from file path
    #[allow(dead_code)]
    pub fn load_from_file(path: &std::path::Path) -> Result<ChainFixture, FixtureError> {
        let content = std::fs::read_to_string(path)?;
        Self::load_from_str(&content)
    }
}

/// Fixture loading/validation errors
#[derive(Debug)]
#[allow(dead_code)]
pub enum FixtureError {
    Io(String),
    Json(String),
    InvalidFixture(String),
}

impl From<std::io::Error> for FixtureError {
    fn from(e: std::io::Error) -> Self {
        FixtureError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for FixtureError {
    fn from(e: serde_json::Error) -> Self {
        FixtureError::Json(e.to_string())
    }
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixtureError::Io(s) => write!(f, "IO error: {}", s),
            FixtureError::Json(s) => write!(f, "JSON error: {}", s),
            FixtureError::InvalidFixture(s) => write!(f, "Invalid fixture: {}", s),
        }
    }
}

impl std::error::Error for FixtureError {}

/// Chain replay executor - runs a fixture deterministically
pub struct ChainReplayExecutor {
    temp_dir: tempfile::TempDir,
    #[allow(dead_code)]
    checkpoint_dir: PathBuf,
}

impl ChainReplayExecutor {
    pub fn new() -> Result<Self, FixtureError> {
        let temp_dir = tempfile::TempDir::new()?;
        let checkpoint_dir = temp_dir.path().join("checkpoints");
        std::fs::create_dir_all(&checkpoint_dir)?;

        Ok(Self {
            temp_dir,
            checkpoint_dir,
        })
    }

    pub fn temp_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }

    #[allow(dead_code)]
    pub fn checkpoint_dir(&self) -> &std::path::Path {
        &self.checkpoint_dir
    }

    /// Set up initial workspace from fixture
    pub fn setup_workspace(&self, fixture: &ChainFixture) -> Result<(), FixtureError> {
        for (path, content) in &fixture.initial_workspace {
            let full_path = self.temp_dir.path().join(path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full_path, content)?;
        }
        Ok(())
    }

    /// Verify final workspace matches fixture expectations
    #[allow(dead_code)]
    pub fn verify_workspace(&self, fixture: &ChainFixture) -> Result<(), FixtureError> {
        for (path, expected_content) in &fixture.expected_final_workspace {
            let full_path = self.temp_dir.path().join(path);
            let actual_content = std::fs::read_to_string(&full_path)
                .map_err(|e| FixtureError::Io(format!("Failed to read {}: {}", path, e)))?;

            if actual_content != *expected_content {
                return Err(FixtureError::InvalidFixture(format!(
                    "Workspace mismatch for {}: expected {:?}, got {:?}",
                    path, expected_content, actual_content
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple three-step chain fixture for testing
    pub const THREE_STEP_CHAIN: &str = r#"{
        "chain_id": "test-chain-001",
        "objective": "Create a simple Rust project",
        "total_steps": 3,
        "steps": [
            {
                "index": 0,
                "description": "Create Cargo.toml",
                "planner_outputs": [
                    {
                        "type": "tool_call",
                        "name": "write_file",
                        "arguments": {
                            "path": "Cargo.toml",
                            "content": "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\""
                        }
                    },
                    {
                        "type": "completion",
                        "reason": "Created Cargo.toml"
                    }
                ],
                "expected_outcome": {
                    "type": "resolved",
                    "summary": "Created Cargo.toml",
                    "files_modified": ["Cargo.toml"]
                },
                "expect_checkpoint": true,
                "file_mutations": [
                    {
                        "path": "Cargo.toml",
                        "content": "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"",
                        "should_fail_validation": null
                    }
                ]
            },
            {
                "index": 1,
                "description": "Create src directory and main.rs",
                "planner_outputs": [
                    {
                        "type": "tool_call",
                        "name": "write_file",
                        "arguments": {
                            "path": "src/main.rs",
                            "content": "fn main() {\n    println!(\"Hello, world!\");\n}"
                        }
                    },
                    {
                        "type": "completion",
                        "reason": "Created main.rs"
                    }
                ],
                "expected_outcome": {
                    "type": "resolved",
                    "summary": "Created src/main.rs",
                    "files_modified": ["src/main.rs"]
                },
                "expect_checkpoint": true,
                "file_mutations": [
                    {
                        "path": "src/main.rs",
                        "content": "fn main() {\n    println!(\"Hello, world!\");\n}",
                        "should_fail_validation": null
                    }
                ]
            },
            {
                "index": 2,
                "description": "Verify project builds",
                "planner_outputs": [
                    {
                        "type": "completion",
                        "reason": "Project structure complete and verified"
                    }
                ],
                "expected_outcome": {
                    "type": "resolved",
                    "summary": "Chain complete",
                    "files_modified": []
                },
                "expect_checkpoint": true,
                "file_mutations": []
            }
        ],
        "expected_final_status": "complete",
        "expected_checkpoints": 3,
        "initial_workspace": {},
        "expected_final_workspace": {
            "Cargo.toml": "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"",
            "src/main.rs": "fn main() {\n    println!(\"Hello, world!\");\n}"
        }
    }"#;

    #[test]
    fn test_load_three_step_fixture() {
        let fixture =
            ChainFixtureLoader::load_from_str(THREE_STEP_CHAIN).expect("Should parse fixture");

        assert_eq!(fixture.chain_id, "test-chain-001");
        assert_eq!(fixture.total_steps, 3);
        assert_eq!(fixture.steps.len(), 3);
        assert_eq!(fixture.expected_checkpoints, 3);
    }

    #[test]
    fn test_replay_executor_workspace_setup() {
        let executor = ChainReplayExecutor::new().expect("Should create executor");
        let fixture =
            ChainFixtureLoader::load_from_str(THREE_STEP_CHAIN).expect("Should parse fixture");

        // Setup should work even with empty initial workspace
        executor
            .setup_workspace(&fixture)
            .expect("Should setup workspace");

        // After setup, we can verify the temp directory exists
        assert!(executor.temp_path().exists());
    }
}
