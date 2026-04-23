//! StateView - Constrained Planner State Boundary
//!
//! Provides a filtered, read-only view of runtime state for planner consumption.
//! Prevents planners from accessing internal runtime handles or mutable state.

use crate::state::AgentState;
use crate::types::{ExecutionMode, FileRecord, ToolName, ValidationReport};
use std::path::PathBuf;

const MAX_CONTENT_EXCERPT_CHARS: usize = 4_000;
const MAX_CONTENT_EXCERPT_LINES: usize = 80;

/// Information about available tools for planner context
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: ToolName,
    #[allow(dead_code)]
    pub description: String,
    #[allow(dead_code)]
    pub required_args: Vec<String>,
    #[allow(dead_code)]
    pub optional_args: Vec<String>,
}

impl ToolInfo {
    pub fn new(name: ToolName, description: &str) -> Self {
        Self {
            name,
            description: description.to_string(),
            required_args: vec![],
            optional_args: vec![],
        }
    }

    pub fn with_required_args(mut self, args: Vec<&str>) -> Self {
        self.required_args = args.into_iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn with_optional_args(mut self, args: Vec<&str>) -> Self {
        self.optional_args = args.into_iter().map(|s| s.to_string()).collect();
        self
    }
}

/// Metadata about a file the planner has read
#[derive(Debug, Clone)]
pub struct FileReadInfo {
    pub path: PathBuf,
    pub content_hash: String,
    #[allow(dead_code)]
    pub size_bytes: u64,
    pub total_lines: usize,
    pub is_full_read: bool,
    #[allow(dead_code)]
    pub read_at_iteration: u32,
    pub content_excerpt: Option<String>,
}

/// Record of a recent tool execution for planner context
#[derive(Debug, Clone)]
pub struct ToolExecutionRecord {
    #[allow(dead_code)]
    pub iteration: u32,
    #[allow(dead_code)]
    pub tool_name: String,
    #[allow(dead_code)]
    pub success: bool,
    #[allow(dead_code)]
    pub summary: String, // Brief description, not full output
}

/// Constrained view of runtime state for planner consumption
///
/// This is NOT the full AgentState. It is a deliberately filtered
/// projection containing only what planners are allowed to see.
#[derive(Debug, Clone)]
pub struct StateView {
    // Task context
    pub task: String,
    pub session_id: String,

    // Execution bounds
    pub iteration: u32,
    pub max_iterations: u32,
    #[allow(dead_code)]
    pub mode: ExecutionMode,

    // File state (read-only metadata)
    pub files_read: Vec<FileReadInfo>,
    pub files_written: Vec<PathBuf>,

    // Tool context
    pub available_tools: Vec<ToolInfo>,

    // Recent history (limited window)
    #[allow(dead_code)]
    pub recent_executions: Vec<ToolExecutionRecord>,
    #[allow(dead_code)]
    pub last_validation: Option<ValidationReport>,

    // Error state (for recovery context)
    pub recent_errors: Vec<String>,

    // Repository boundaries
    #[allow(dead_code)]
    pub repo_root: PathBuf,
    #[allow(dead_code)]
    pub allowed_paths: Vec<PathBuf>,
}

impl StateView {
    /// Create StateView from full AgentState
    /// Filters and constrains what the planner can see
    pub fn from_agent_state(
        state: &AgentState,
        available_tools: Vec<ToolInfo>,
        recent_executions: Vec<ToolExecutionRecord>,
    ) -> Self {
        // Convert files_read to FileReadInfo
        let files_read: Vec<FileReadInfo> = state
            .files_read
            .values()
            .map(|record| FileReadInfo {
                path: record.path.clone(),
                content_hash: record.content_hash.clone(),
                size_bytes: record.size_bytes,
                total_lines: record.total_lines,
                is_full_read: record.is_full_read,
                read_at_iteration: record.read_at_iteration,
                content_excerpt: bounded_excerpt(record),
            })
            .collect();

        // Extract recent errors from change history (rejections)
        let recent_errors: Vec<String> = state
            .change_history
            .iter()
            .rev()
            .take(5)
            .filter(|r| r.validation_report.decision == crate::types::ValidationDecision::Reject)
            .map(|r| r.validation_report.message.clone())
            .collect();

        // Get last validation from most recent change record
        let last_validation = state
            .change_history
            .last()
            .map(|r| r.validation_report.clone());

        Self {
            task: state.task.clone(),
            session_id: state.session_id.to_string(),
            iteration: state.iteration,
            max_iterations: state.max_iterations,
            mode: state.mode,
            files_read,
            files_written: state.files_written.iter().cloned().collect(),
            available_tools,
            recent_executions,
            last_validation,
            recent_errors,
            repo_root: std::path::PathBuf::from("."),
            allowed_paths: vec![std::path::PathBuf::from(".")],
        }
    }

    /// Check if a file has been fully read (for read-before-write checks)
    #[allow(dead_code)]
    pub fn is_file_fully_read(&self, path: &PathBuf) -> bool {
        self.files_read
            .iter()
            .any(|f| f.path == *path && f.is_full_read)
    }

    /// Get file read info if available
    #[allow(dead_code)]
    pub fn get_file_info(&self, path: &PathBuf) -> Option<&FileReadInfo> {
        self.files_read.iter().find(|f| f.path == *path)
    }

    /// Check if a tool is available in current mode
    pub fn is_tool_available(&self, tool_name: &ToolName) -> bool {
        self.available_tools.iter().any(|t| t.name == *tool_name)
    }

    /// Serialize to simplified JSON for model consumption
    pub fn to_json(&self) -> String {
        // Manual JSON construction to avoid serde complexity with core types
        let files_read_json: Vec<String> = self
            .files_read
            .iter()
            .map(|f| {
                let excerpt_json = f
                    .content_excerpt
                    .as_ref()
                    .map(|excerpt| format!(", \"excerpt\": \"{}\"", escape_json_string(excerpt)))
                    .unwrap_or_default();
                format!(
                    "{{\"path\": \"{}\", \"hash\": \"{}\", \"lines\": {}, \"full_read\": {}{}}}",
                    escape_json_string(&f.path.display().to_string()),
                    escape_json_string(&f.content_hash),
                    f.total_lines,
                    f.is_full_read,
                    excerpt_json
                )
            })
            .collect();

        let tools_json: Vec<String> = self
            .available_tools
            .iter()
            .map(|t| format!("\"{}\"", t.name.as_str().replace('"', "\\\"")))
            .collect();

        format!(
            "{{\n  \"session_id\": \"{}\",\n  \"iteration\": {}/{},\n  \"task\": \"{}\",\n  \"files_read\": [{}],\n  \"files_written\": [{}],\n  \"available_tools\": [{}],\n  \"recent_errors\": [{}]\n}}",
            escape_json_string(&self.session_id),
            self.iteration,
            self.max_iterations,
            escape_json_string(&self.task),
            files_read_json.join(", "),
            self.files_written
                .iter()
                .map(|p| format!("\"{}\"", escape_json_string(&p.display().to_string())))
                .collect::<Vec<_>>()
                .join(", "),
            tools_json.join(", "),
            self.recent_errors
                .iter()
                .map(|e| format!("\"{}\"", escape_json_string(e)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// Get content hash for a file if read
    #[allow(dead_code)]
    pub fn get_file_hash(&self, path: &PathBuf) -> Option<&String> {
        self.get_file_info(path).map(|f| &f.content_hash)
    }
}

fn bounded_excerpt(record: &FileRecord) -> Option<String> {
    let content = record.content.as_ref()?;
    let excerpt_line_count = content.lines().take(MAX_CONTENT_EXCERPT_LINES).count();
    let mut excerpt = content
        .lines()
        .take(MAX_CONTENT_EXCERPT_LINES)
        .collect::<Vec<_>>()
        .join("\n");

    let was_line_truncated = content.lines().count() > excerpt_line_count;
    if excerpt.len() > MAX_CONTENT_EXCERPT_CHARS {
        excerpt = excerpt.chars().take(MAX_CONTENT_EXCERPT_CHARS).collect();
        excerpt.push('\u{2026}');
    } else if was_line_truncated {
        excerpt.push_str("\n\u{2026}");
    }

    if excerpt.is_empty() && content.is_empty() {
        return Some(String::new());
    }

    Some(excerpt)
}

fn escape_json_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Builder for StateView to simplify construction
#[allow(dead_code)]
pub struct StateViewBuilder {
    state: Option<AgentState>,
    tools: Vec<ToolInfo>,
    executions: Vec<ToolExecutionRecord>,
}

impl StateViewBuilder {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            state: None,
            tools: vec![],
            executions: vec![],
        }
    }

    #[allow(dead_code)]
    pub fn with_state(mut self, state: &AgentState) -> Self {
        self.state = Some(state.clone());
        self
    }

    #[allow(dead_code)]
    pub fn with_tools(mut self, tools: Vec<ToolInfo>) -> Self {
        self.tools = tools;
        self
    }

    #[allow(dead_code)]
    pub fn with_executions(mut self, executions: Vec<ToolExecutionRecord>) -> Self {
        self.executions = executions;
        self
    }

    #[allow(dead_code)]
    pub fn build(self) -> Option<StateView> {
        self.state
            .map(|s| StateView::from_agent_state(&s, self.tools, self.executions))
    }
}

impl Default for StateViewBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AgentState;
    use crate::types::FileRecord;

    #[test]
    fn state_view_includes_bounded_file_excerpt() {
        let mut state = AgentState::new(4, "test".to_string(), crate::types::ExecutionMode::Edit);
        let content = (1..=120)
            .map(|n| format!("line {}", n))
            .collect::<Vec<_>>()
            .join("\n");

        let record =
            FileRecord::new("src/lib.rs", &content, None, 1).with_observed_content(content.clone());
        state.record_file_read(record);

        let state_view = StateView::from_agent_state(&state, vec![], vec![]);
        let read_info = state_view
            .get_file_info(&PathBuf::from("src/lib.rs"))
            .unwrap();
        let excerpt = read_info.content_excerpt.as_ref().unwrap();

        assert!(excerpt.contains("line 1"));
        assert!(excerpt.contains("line 80"));
        assert!(!excerpt.contains("line 81"));
        assert!(excerpt.ends_with('\u{2026}') || excerpt.ends_with("\n\u{2026}"));
        assert!(state_view.to_json().contains("\"excerpt\":"));
    }
}
