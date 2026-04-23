use super::policy::{MessageTemplates, RenderingPolicy};
use crate::types::{MessageLevel, OrchestratorEvent, UserFacingEvent};
use std::path::Path;

/// Maps internal orchestrator events to user-facing messages
#[derive(Debug, Clone)]
pub struct TransparencyMapper {
    policy: RenderingPolicy,
}

impl TransparencyMapper {
    pub fn new() -> Self {
        Self {
            policy: RenderingPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: RenderingPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Map an internal event to a user-facing event
    pub fn map(&self, event: &OrchestratorEvent) -> Option<UserFacingEvent> {
        match event {
            // ─── File Operations ─────────────────────────────
            OrchestratorEvent::ToolExecuting { name, arguments } => {
                self.map_tool_executing(name, arguments)
            }

            OrchestratorEvent::ToolCompleted { name, result } => {
                self.map_tool_completed(name, result)
            }

            // ─── Validation ───────────────────────────────────
            OrchestratorEvent::ValidationStarted { .. } => {
                Some(UserFacingEvent::ValidationRunning { stage: None })
            }

            OrchestratorEvent::ValidationStageRunning { stage } => {
                Some(UserFacingEvent::ValidationRunning {
                    stage: Some(stage.clone()),
                })
            }

            OrchestratorEvent::ValidationStageCompleted { stage, passed } => {
                if *passed && stage == "final" {
                    Some(UserFacingEvent::ValidationPassed)
                } else if !passed {
                    Some(UserFacingEvent::ValidationFailed {
                        reason: format!("{} validation failed", stage),
                        reverted: false,
                    })
                } else {
                    // Intermediate stage passed - don't show
                    None
                }
            }

            // ─── Completion ───────────────────────────────────
            OrchestratorEvent::ExecutionCompleted { success, summary } => {
                if *success {
                    Some(UserFacingEvent::SystemMessage {
                        content: summary.clone(),
                        level: MessageLevel::Success,
                    })
                } else {
                    Some(UserFacingEvent::WorkFailed {
                        reason: summary.clone(),
                        suggestion: Some("You can try again or modify the request".to_string()),
                    })
                }
            }

            OrchestratorEvent::ExecutionFailed {
                reason,
                recoverable,
            } => {
                let suggestion = if *recoverable {
                    Some("The system will retry automatically".to_string())
                } else {
                    Some("This error requires manual intervention".to_string())
                };

                Some(UserFacingEvent::WorkFailed {
                    reason: reason.clone(),
                    suggestion,
                })
            }

            // ─── Approval ────────────────────────────────────
            OrchestratorEvent::ApprovalRequired { request } => {
                Some(UserFacingEvent::ApprovalRequested {
                    request: request.clone(),
                })
            }

            // ─── Intent/Clarification ────────────────────────
            OrchestratorEvent::ClarificationRequested { question } => {
                Some(UserFacingEvent::ClarificationQuestion {
                    question: question.clone(),
                    context: None,
                })
            }

            // ─── Internal (not shown to user) ─────────────────
            OrchestratorEvent::IntentRefinementStarted { .. } => None,
            OrchestratorEvent::IntentRefinementCompleted { .. } => None,
            OrchestratorEvent::ExecutionStarted { .. } => None,
            OrchestratorEvent::PlannerInvoked { .. } => None,
            OrchestratorEvent::MutationsCommitted { .. } => None,
        }
    }

    fn map_tool_executing(
        &self,
        name: &str,
        args: &super::super::types::events::ToolArguments,
    ) -> Option<UserFacingEvent> {
        let path = args
            .path
            .as_ref()
            .or(args.file_path.as_ref())
            .map(|p| self.format_path(p));

        let description = match name {
            "read_file" => path
                .map(|p| MessageTemplates::reading_file(&p))
                .unwrap_or_else(|| "Reading file...".to_string()),
            "write_file" => path
                .map(|p| MessageTemplates::writing_file(&p))
                .unwrap_or_else(|| "Writing file...".to_string()),
            "apply_patch" => path
                .map(|p| MessageTemplates::updating_file(&p))
                .unwrap_or_else(|| "Applying changes...".to_string()),
            "delete_file" => path
                .map(|p| MessageTemplates::deleting_file(&p))
                .unwrap_or_else(|| "Deleting file...".to_string()),
            "search" => "Searching...".to_string(),
            "execute_command" => "Running command...".to_string(),
            _ => format!("Executing {}...", name),
        };

        Some(UserFacingEvent::ActionStarted { description })
    }

    fn map_tool_completed(
        &self,
        name: &str,
        result: &super::super::types::events::ToolResultSummary,
    ) -> Option<UserFacingEvent> {
        if !result.success {
            return Some(UserFacingEvent::ActionFailed {
                description: format!("{} failed", name),
                error: result
                    .error_preview
                    .clone()
                    .unwrap_or_else(|| "Unknown error".to_string()),
            });
        }

        let description = match name {
            "read_file" => result
                .line_count
                .map(|n| MessageTemplates::file_read("file", n))
                .unwrap_or_else(|| "File read completed".to_string()),
            "write_file" => MessageTemplates::file_written("file"),
            "apply_patch" => MessageTemplates::file_updated("file"),
            "delete_file" => MessageTemplates::file_deleted("file"),
            "search" => "Search completed".to_string(),
            "execute_command" => "Command completed".to_string(),
            _ => format!("{} completed", name),
        };

        Some(UserFacingEvent::ActionCompleted { description })
    }

    /// Format path for display
    fn format_path(&self, path: &Path) -> String {
        let s = path.display().to_string();

        if s.len() > self.policy.max_path_length {
            // Truncate from the start, keeping the filename
            if let Some(filename) = path.file_name() {
                let fname = filename.to_string_lossy();
                format!(".../{}", fname)
            } else {
                format!("...{}", &s[s.len() - self.policy.max_path_length..])
            }
        } else {
            s
        }
    }
}

impl Default for TransparencyMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::events::ToolArguments;
    use std::path::PathBuf;

    #[test]
    fn test_map_read_file_executing() {
        let mapper = TransparencyMapper::new();
        let args = ToolArguments {
            path: Some(PathBuf::from("src/main.rs")),
            file_path: None,
            raw: Default::default(),
        };

        let event = OrchestratorEvent::ToolExecuting {
            name: "read_file".to_string(),
            arguments: args,
        };

        let result = mapper.map(&event);
        assert!(matches!(
            result,
            Some(UserFacingEvent::ActionStarted { .. })
        ));
    }

    #[test]
    fn test_format_path_truncation() {
        let mapper = TransparencyMapper::new().with_policy(RenderingPolicy {
            max_path_length: 20,
            ..Default::default()
        });

        let long_path = Path::new("/very/long/path/to/the/file.rs");
        let formatted = mapper.format_path(long_path);

        assert!(formatted.len() <= 25); // Allow for ".../" prefix
        assert!(formatted.contains("file.rs"));
    }
}
