//! Stub Planner - Deterministic Rule-Based Planner
//!
//! The original Forge deterministic planner, now implementing the Planner trait.
//! Used for testing, deterministic mode, and when no model backend is available.

use crate::planner::state_view::StateView;
use crate::planner::traits::Planner;
use crate::types::{
    CompletionReason, ForgeError, PlannerOutput, ToolArguments, ToolCall, ToolName,
};
use std::path::{Path, PathBuf};

/// Sample file for demo
const SAMPLE_FILE: &str = "hello.txt";
const INITIAL_CONTENT: &str = "hello world";
const PATCHED_CONTENT: &str = "hello world from forge";

/// Compute content hash using same algorithm as runtime
fn compute_hash(content: &str) -> String {
    crate::crypto_hash::compute_content_hash(content)
}

/// Deterministic stub planner implementing the canonical Planner trait.
///
/// Exercises write_file → read_file → apply_patch → complete flow.
/// PHASE 2.5: apply_patch requires expected_hash (mandatory)
pub struct StubPlanner;

impl StubPlanner {
    pub fn new() -> Self {
        Self
    }

    /// Check if file is already written in state view
    fn is_file_written(&self, state: &StateView, path: &PathBuf) -> bool {
        let target_name = path.file_name();
        state
            .files_written
            .iter()
            .any(|written| written == path || written.file_name() == target_name)
    }

    /// Check if file is fully read in state view
    fn is_file_fully_read(&self, state: &StateView, path: &Path) -> bool {
        state.files_read.iter().any(|f| {
            // Compare file names to handle both relative and absolute paths
            f.path.file_name() == path.file_name() && f.is_full_read
        })
    }

    /// Get file hash from state view
    fn get_file_hash(&self, state: &StateView, path: &Path) -> Option<String> {
        state
            .files_read
            .iter()
            .find(|f| f.path.file_name() == path.file_name())
            .map(|f| f.content_hash.clone())
    }
}

impl Default for StubPlanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Planner for StubPlanner {
    fn generate(&self, state: &StateView) -> Result<PlannerOutput, ForgeError> {
        let sample_path = PathBuf::from(SAMPLE_FILE);

        // Iteration 0: Create the file if it doesn't exist
        if !self.is_file_written(state, &sample_path) {
            let mut arguments = ToolArguments::new();
            arguments.set("path", SAMPLE_FILE);
            arguments.set("content", INITIAL_CONTENT);

            return Ok(PlannerOutput::ToolCall(ToolCall {
                name: ToolName::new("write_file").unwrap(),
                arguments,
            }));
        }

        // Iteration 1: Read the file
        if !self.is_file_fully_read(state, &sample_path) {
            let mut arguments = ToolArguments::new();
            arguments.set("path", SAMPLE_FILE);
            // Full read, no offset/limit

            return Ok(PlannerOutput::ToolCall(ToolCall {
                name: ToolName::new("read_file").unwrap(),
                arguments,
            }));
        }

        // Iteration 2: Apply patch with mandatory hash binding
        // PHASE 2.5: expected_hash is now MANDATORY
        let expected_hash = self
            .get_file_hash(state, &sample_path)
            .unwrap_or_else(|| compute_hash(INITIAL_CONTENT));

        if state.iteration <= 2 {
            let mut arguments = ToolArguments::new();
            arguments.set("file_path", SAMPLE_FILE);
            arguments.set("old_text", INITIAL_CONTENT);
            arguments.set("new_text", PATCHED_CONTENT);

            // PHASE 2.5: Bind patch to specific file state via hash
            arguments.set("expected_hash", &expected_hash);

            return Ok(PlannerOutput::ToolCall(ToolCall {
                name: ToolName::new("apply_patch").unwrap(),
                arguments,
            }));
        }

        // Iteration 3: Complete
        Ok(PlannerOutput::Completion {
            reason: CompletionReason::new(
                "File hello.txt updated with content 'hello world patched' - all requirements met",
            ),
        })
    }

    fn planner_type(&self) -> &'static str {
        "stub"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::state_view::{FileReadInfo, StateView, ToolInfo};
    use crate::types::ExecutionMode;

    fn create_test_state() -> StateView {
        StateView {
            task: "test".to_string(),
            session_id: "test-session".to_string(),
            iteration: 0,
            max_iterations: 10,
            mode: ExecutionMode::Edit,
            files_read: vec![],
            files_written: vec![],
            available_tools: vec![
                ToolInfo::new(ToolName::new("write_file").unwrap(), "Write a file"),
                ToolInfo::new(ToolName::new("read_file").unwrap(), "Read a file"),
                ToolInfo::new(ToolName::new("apply_patch").unwrap(), "Apply a patch"),
            ],
            recent_executions: vec![],
            last_validation: None,
            recent_errors: vec![],
            repo_root: PathBuf::from("."),
            allowed_paths: vec![],
        }
    }

    #[test]
    fn test_stub_planner_writes_file_first() {
        let planner = StubPlanner::new();
        let state = create_test_state();

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::ToolCall(tc) => {
                assert_eq!(tc.name.as_str(), "write_file");
                assert_eq!(tc.arguments.get("path"), Some("hello.txt"));
            }
            _ => panic!("Expected ToolCall for write_file"),
        }
    }

    #[test]
    fn test_stub_planner_reads_after_write() {
        let planner = StubPlanner::new();
        let mut state = create_test_state();
        state.files_written.push(PathBuf::from(SAMPLE_FILE));

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::ToolCall(tc) => {
                assert_eq!(tc.name.as_str(), "read_file");
            }
            _ => panic!("Expected ToolCall for read_file"),
        }
    }

    #[test]
    fn test_stub_planner_reads_after_absolute_write_path() {
        let planner = StubPlanner::new();
        let mut state = create_test_state();
        state
            .files_written
            .push(PathBuf::from("/tmp/forge-tests/hello.txt"));

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::ToolCall(tc) => {
                assert_eq!(tc.name.as_str(), "read_file");
                assert_eq!(tc.arguments.get("path"), Some("hello.txt"));
            }
            _ => panic!("Expected ToolCall for read_file"),
        }
    }

    #[test]
    fn test_stub_planner_patches_after_read() {
        let planner = StubPlanner::new();
        let mut state = create_test_state();
        state.files_written.push(PathBuf::from(SAMPLE_FILE));
        state.files_read.push(FileReadInfo {
            path: PathBuf::from(SAMPLE_FILE),
            content_hash: compute_hash(INITIAL_CONTENT),
            size_bytes: 11,
            total_lines: 1,
            is_full_read: true,
            read_at_iteration: 1,
            content_excerpt: Some(INITIAL_CONTENT.to_string()),
        });
        state.iteration = 2;

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::ToolCall(tc) => {
                assert_eq!(tc.name.as_str(), "apply_patch");
                // Verify expected_hash is present
                assert!(tc.arguments.get("expected_hash").is_some());
            }
            _ => panic!("Expected ToolCall for apply_patch"),
        }
    }

    #[test]
    fn test_stub_planner_completes_after_patch() {
        let planner = StubPlanner::new();
        let mut state = create_test_state();
        state.files_written.push(PathBuf::from(SAMPLE_FILE));
        state.files_read.push(FileReadInfo {
            path: PathBuf::from(SAMPLE_FILE),
            content_hash: compute_hash(INITIAL_CONTENT),
            size_bytes: 11,
            total_lines: 1,
            is_full_read: true,
            read_at_iteration: 1,
            content_excerpt: Some(INITIAL_CONTENT.to_string()),
        });
        state.iteration = 3;

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::Completion { .. } => {
                // Success
            }
            _ => panic!("Expected Completion"),
        }
    }
}
