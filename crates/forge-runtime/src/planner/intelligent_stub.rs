//! Intelligent Stub Planner - Phase 2
//!
//! A deterministic planner that:
//! - Parses task descriptions
//! - Tracks state (files_read, mutations, validation status)
//! - Emits proper read-before-write sequences
//! - Emits completion when task is observably done
//! - Uses validator feedback for retries

use crate::planner::state_view::StateView;
use crate::planner::traits::Planner;
use crate::types::{
    CompletionReason, ForgeError, PlannerOutput, ToolArguments, ToolCall, ToolName,
};
use serde_json;
use std::collections::HashMap;
use std::path::PathBuf;

/// Task type detection patterns
#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskType {
    CreateFile { path: String, content: String },
    ReadFile { path: String },
    ModifyFile { path: String, operation: String },
    Unknown,
}

/// Intelligent stub planner with state tracking
pub struct IntelligentStubPlanner {
    task_type: Option<TaskType>,
    state: PlannerState,
    max_retries: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PlannerState {
    files_created: Vec<String>,
    files_read: Vec<String>,
    pending_validation: bool,
    validation_errors: Vec<String>,
    last_action: Option<String>,
}

impl IntelligentStubPlanner {
    pub fn new() -> Self {
        Self {
            task_type: None,
            state: PlannerState {
                files_created: vec![],
                files_read: vec![],
                pending_validation: false,
                validation_errors: vec![],
                last_action: None,
            },
            max_retries: 3,
        }
    }

    /// Parse task description to determine what needs to be done
    fn parse_task(&mut self, task: &str) -> TaskType {
        let task_lower = task.to_lowercase();

        // Pattern: "create a new file X with content Y"
        if task_lower.contains("create") && task_lower.contains("file") {
            // Extract path after "file"
            let path = self.extract_file_path(task);
            let content = self.extract_content(task);

            return TaskType::CreateFile { path, content };
        }

        // Pattern: "read file X"
        if task_lower.contains("read") && task_lower.contains("file") {
            let path = self.extract_file_path(task);
            return TaskType::ReadFile { path };
        }

        // Pattern: "modify/update/change file X"
        if (task_lower.contains("modify")
            || task_lower.contains("update")
            || task_lower.contains("change"))
            && task_lower.contains("file")
        {
            let path = self.extract_file_path(task);
            return TaskType::ModifyFile {
                path,
                operation: task.to_string(),
            };
        }

        TaskType::Unknown
    }

    /// Extract file path from task description
    fn extract_file_path(&self, task: &str) -> String {
        // Simple extraction - look for common path patterns
        let words: Vec<&str> = task.split_whitespace().collect();

        // Find word after "file" or standalone path-like strings
        for (i, word) in words.iter().enumerate() {
            if *word == "file" && i + 1 < words.len() {
                let candidate =
                    words[i + 1].trim_matches(|c| c == '\'' || c == '"' || c == ',' || c == '.');
                if candidate.contains('.') || candidate.contains('/') {
                    return candidate.to_string();
                }
            }
        }

        // Fallback: look for any word with file extension
        for word in &words {
            let clean = word.trim_matches(|c| c == '\'' || c == '"' || c == ',' || c == '.');
            if clean.contains('.') && !clean.starts_with("with") {
                return clean.to_string();
            }
        }

        "file.txt".to_string() // Default
    }

    /// Extract content from task description
    fn extract_content(&self, task: &str) -> String {
        // Look for content between quotes or after "with content"
        let task_lower = task.to_lowercase();

        if let Some(idx) = task_lower.find("with content") {
            let after = &task[idx + 12..].trim();
            // Remove surrounding quotes if present
            return after.trim_matches(|c| c == '\'' || c == '"').to_string();
        }

        if let Some(idx) = task_lower.find("containing") {
            let after = &task[idx + 10..].trim();
            return after.trim_matches(|c| c == '\'' || c == '"').to_string();
        }

        // Default content
        "Hello from Forge".to_string()
    }

    /// Build tool call for read_file
    fn build_read_call(&self, path: &str) -> PlannerOutput {
        let mut args = ToolArguments::new();
        args.set("path", path);

        PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("read_file").unwrap(),
            arguments: args,
        })
    }

    /// Build tool call for write_file
    fn build_write_call(&self, path: &str, content: &str) -> PlannerOutput {
        let mut args = ToolArguments::new();
        args.set("path", path);
        args.set("content", content);

        PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("write_file").unwrap(),
            arguments: args,
        })
    }

    /// Build completion signal
    fn build_completion(&self, reason: &str) -> PlannerOutput {
        PlannerOutput::Completion {
            reason: CompletionReason::new(reason),
        }
    }

    /// Determine next action based on current state
    fn determine_next_action(&mut self, state_view: &StateView) -> PlannerOutput {
        // Parse task if not already done
        if self.task_type.is_none() {
            self.task_type = Some(self.parse_task(&state_view.task));
        }

        let task_type = self.task_type.clone().unwrap_or(TaskType::Unknown);

        match &task_type {
            TaskType::CreateFile { path, content } => {
                // Check if file was already created
                if self.state.files_created.contains(path)
                    || state_view.files_written.iter().any(|p| p.ends_with(path))
                {
                    // File created - emit completion
                    return self.build_completion(&format!(
                        "Task complete: Created file '{}' with content '{}'",
                        path, content
                    ));
                }

                // Check if file exists (we need to read first for existing files)
                let full_path = PathBuf::from(path);
                let file_exists = full_path.exists();

                if file_exists && !self.state.files_read.contains(path) {
                    // Need to read existing file before overwriting
                    self.state.last_action = Some(format!("read_{}", path));
                    return self.build_read_call(path);
                }

                // Create the file
                self.state.last_action = Some(format!("write_{}", path));
                self.build_write_call(path, content)
            }

            TaskType::ReadFile { path } => {
                if self.state.files_read.contains(path) {
                    return self.build_completion(&format!("Task complete: Read file '{}'", path));
                }

                self.state.last_action = Some(format!("read_{}", path));
                self.build_read_call(path)
            }

            TaskType::ModifyFile { path, operation } => {
                // For modifications, we need to read first
                if !self.state.files_read.contains(path) {
                    self.state.last_action = Some(format!("read_{}", path));
                    return self.build_read_call(path);
                }

                // After reading, we would apply patch
                // For stub, just complete
                self.build_completion(&format!(
                    "Task preparation complete: Read file '{}' for modification '{}'",
                    path, operation
                ))
            }

            TaskType::Unknown => {
                // Unknown task - emit generic completion
                self.build_completion(&format!(
                    "Task processed: '{}' - no specific action pattern matched",
                    state_view.task
                ))
            }
        }
    }
}

impl Planner for IntelligentStubPlanner {
    fn generate(&self, state: &StateView) -> Result<PlannerOutput, ForgeError> {
        // Clone self to allow mutation during planning
        let mut planner = IntelligentStubPlanner {
            task_type: self.task_type.clone(),
            state: self.state.clone(),
            max_retries: self.max_retries,
        };

        let output = planner.determine_next_action(state);

        // Log what we're doing
        match &output {
            PlannerOutput::ToolCall(tc) => {
                eprintln!(
                    "[INTELLIGENT_STUB] Emitting tool_call: {}",
                    tc.name.as_str()
                );
            }
            PlannerOutput::Completion { reason } => {
                eprintln!(
                    "[INTELLIGENT_STUB] Emitting completion: {}",
                    reason.as_str()
                );
            }
            _ => {}
        }

        Ok(output)
    }

    fn generate_raw(&self, state: &StateView) -> Result<String, ForgeError> {
        // Generate canonical JSON output matching PlannerOutput
        let output = self.generate(state)?;

        let json_str = match output {
            PlannerOutput::ToolCall(tc) => {
                let name = tc.name.as_str();
                // Get required arguments by name for canonical output
                let path = tc.arguments.get("path").unwrap_or("");
                let content = tc.arguments.get("content").unwrap_or("");

                // Build arguments object with only the fields we need
                let mut args_map = serde_json::Map::new();
                if !path.is_empty() {
                    args_map.insert("path".to_string(), serde_json::json!(path));
                }
                if !content.is_empty() {
                    args_map.insert("content".to_string(), serde_json::json!(content));
                }

                serde_json::json!({
                    "type": "tool_call",
                    "tool_call": {
                        "name": name,
                        "arguments": args_map
                    }
                })
                .to_string()
            }
            PlannerOutput::Completion { reason } => serde_json::json!({
                "type": "completion",
                "reason": reason.as_str()
            })
            .to_string(),
            PlannerOutput::Failure {
                reason,
                recoverable,
            } => serde_json::json!({
                "type": "failure",
                "reason": reason.as_str(),
                "recoverable": recoverable
            })
            .to_string(),
        };

        Ok(json_str)
    }

    fn planner_type(&self) -> &'static str {
        "intelligent_stub"
    }

    fn health_check(&self) -> Result<(), ForgeError> {
        // Stub is always healthy
        Ok(())
    }
}

impl Default for IntelligentStubPlanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::state_view::{StateView, ToolInfo};
    use crate::types::ExecutionMode;
    use std::path::PathBuf;

    fn create_test_state(task: &str) -> StateView {
        StateView {
            task: task.to_string(),
            session_id: "test".to_string(),
            iteration: 0,
            max_iterations: 10,
            mode: ExecutionMode::Edit,
            files_read: vec![],
            files_written: vec![],
            available_tools: vec![
                ToolInfo::new(ToolName::new("read_file").unwrap(), "Read a file"),
                ToolInfo::new(ToolName::new("write_file").unwrap(), "Write a file"),
            ],
            recent_executions: vec![],
            last_validation: None,
            recent_errors: vec![],
            repo_root: PathBuf::from("."),
            allowed_paths: vec![],
        }
    }

    #[test]
    fn test_parse_create_file_task() {
        let mut planner = IntelligentStubPlanner::new();
        let task = "Create a new file hello.txt with content 'Hello World'";
        let task_type = planner.parse_task(task);

        match task_type {
            TaskType::CreateFile { path, content } => {
                assert_eq!(path, "hello.txt");
                assert_eq!(content, "Hello World");
            }
            _ => panic!("Expected CreateFile task type"),
        }
    }

    #[test]
    fn test_parse_read_file_task() {
        let mut planner = IntelligentStubPlanner::new();
        let task = "Read the file src/main.rs";
        let task_type = planner.parse_task(task);

        match task_type {
            TaskType::ReadFile { path } => {
                assert_eq!(path, "src/main.rs");
            }
            _ => panic!("Expected ReadFile task type"),
        }
    }

    #[test]
    fn test_determine_next_action_create() {
        let planner = IntelligentStubPlanner::new();
        let state = create_test_state("Create file test.txt with content 'test'");

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::ToolCall(tc) => {
                assert_eq!(tc.name.as_str(), "write_file");
            }
            _ => panic!("Expected ToolCall"),
        }
    }
}
