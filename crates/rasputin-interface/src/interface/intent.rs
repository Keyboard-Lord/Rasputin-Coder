//! Intent refinement - expands vague requests into concrete specifications
//!
//! Phase 2 implementation: Follow-up reference resolution

use crate::types::{
    AmbiguityType, FollowUpReference, IntentSpec, ResolutionFailureReason, SessionContext,
    UserFacingEvent, UserMessage,
};
use crate::Result;

/// Refines user messages into executable intent specifications
#[derive(Debug, Clone)]
pub struct IntentRefiner;

impl IntentRefiner {
    pub fn new() -> Self {
        Self
    }

    /// Refine a user message into an intent spec
    ///
    /// Phase 2: Resolve "that", "continue", "fix that" references
    pub fn refine(&self, message: &UserMessage, context: &SessionContext) -> Result<IntentSpec> {
        let content = message.content.trim();
        let lower = content.to_lowercase();

        // Check for follow-up commands
        if lower == "continue" || lower.starts_with("continue ") {
            return self.resolve_continue(context);
        }

        if lower.starts_with("fix that") || lower.starts_with("fix it") {
            return self.resolve_fix_that(content, context);
        }

        if lower.starts_with("try again") || lower.starts_with("retry") {
            return self.resolve_retry(content, context);
        }

        if lower.starts_with("what changed") || lower.starts_with("what did you do") {
            return self.resolve_what_changed(context);
        }

        if lower.starts_with("undo") || lower.starts_with("revert") {
            return self.resolve_undo(context);
        }

        // Check for anaphoric references ("that", "it", "this")
        if self.contains_anaphora(&lower) {
            return self.resolve_anaphora(content, context);
        }

        // Default: direct task
        Ok(IntentSpec::Concrete {
            task: content.to_string(),
            target_files: Vec::new(),
            constraints: Vec::new(),
            references: Vec::new(),
        })
    }

    /// Check if content contains vague references
    pub fn is_vague(&self, content: &str) -> bool {
        let vague_terms = [
            "that",
            "it",
            "this",
            "continue",
            "fix that",
            "try again",
            "what changed",
            "undo",
        ];
        let lower = content.to_lowercase();
        vague_terms.iter().any(|term| lower.contains(term))
    }

    /// Detect anaphoric references
    fn contains_anaphora(&self, content: &str) -> bool {
        let patterns = [
            " that ",
            " it ",
            " this ",
            "that file",
            "this file",
            "it there",
        ];
        patterns.iter().any(|p| content.contains(p))
    }

    /// Resolve "continue" - resume from last uncommitted work or re-run last intent
    fn resolve_continue(&self, context: &SessionContext) -> Result<IntentSpec> {
        // First check for uncommitted work
        if let Some(ref uncommitted) = context.uncommitted_work {
            return Ok(IntentSpec::Concrete {
                task: format!(
                    "Continue from paused work: {}",
                    uncommitted
                        .intent_spec
                        .task_description()
                        .unwrap_or("unknown task")
                ),
                target_files: Vec::new(),
                constraints: vec!["resume_from_checkpoint".to_string()],
                references: vec![FollowUpReference::UncommittedWorkRef],
            });
        }

        // Otherwise, re-run last completed turn's intent
        if let Some(last_turn) = context.conversation.last() {
            if let Some(ref intent) = last_turn.intent_spec {
                return Ok(IntentSpec::Concrete {
                    task: format!(
                        "Continue: {}",
                        intent.task_description().unwrap_or("previous task")
                    ),
                    target_files: Vec::new(),
                    constraints: vec!["continue_previous".to_string()],
                    references: vec![FollowUpReference::IntentRef(last_turn.turn_id)],
                });
            }
        }

        // No context to continue from
        Ok(IntentSpec::ResolutionFailed {
            reference: "continue".to_string(),
            reason: ResolutionFailureReason::NoRecentContext,
        })
    }

    /// Resolve "fix that" - fix the most recent error
    fn resolve_fix_that(&self, content: &str, context: &SessionContext) -> Result<IntentSpec> {
        // Get most recent error
        if let Some(error) = context.recent_errors.first() {
            let task = if content.len() > 8 {
                // User added more context: "fix that by adding a parameter"
                format!(
                    "Fix the error in turn {}: {}. {}",
                    error.turn_id,
                    error.summary,
                    &content[8..].trim() // Remove "fix that" prefix
                )
            } else {
                format!("Fix the error: {}", error.summary)
            };

            let mut target_files = Vec::new();
            if let Some(ref file) = error.file {
                target_files.push(file.clone());
            }

            return Ok(IntentSpec::Concrete {
                task,
                target_files,
                constraints: vec!["fix_error".to_string()],
                references: vec![FollowUpReference::ErrorRef {
                    turn_id: error.turn_id,
                    error_summary: error.summary.clone(),
                }],
            });
        }

        // No recent error - try to resolve "that" to a file
        self.resolve_anaphora(content, context)
    }

    /// Resolve "try again" / "retry" - same goal, potentially different approach
    fn resolve_retry(&self, content: &str, context: &SessionContext) -> Result<IntentSpec> {
        if let Some(last_turn) = context.conversation.last() {
            if let Some(ref intent) = last_turn.intent_spec {
                let additional_instruction = if content.len() > 9 {
                    format!(" Additional instruction: {}", &content[9..].trim())
                } else {
                    String::new()
                };

                return Ok(IntentSpec::Concrete {
                    task: format!(
                        "Retry: {}{}",
                        intent.task_description().unwrap_or("previous task"),
                        additional_instruction
                    ),
                    target_files: Vec::new(),
                    constraints: vec!["retry_with_variation".to_string()],
                    references: vec![FollowUpReference::IntentRef(last_turn.turn_id)],
                });
            }
        }

        Ok(IntentSpec::ResolutionFailed {
            reference: "retry".to_string(),
            reason: ResolutionFailureReason::NoRecentContext,
        })
    }

    /// Resolve "what changed?" - summarize recent mutations
    fn resolve_what_changed(&self, context: &SessionContext) -> Result<IntentSpec> {
        // Collect files from recent completed turns
        let mut changed_files: Vec<std::path::PathBuf> = Vec::new();

        for turn in context.conversation.iter().rev().take(5) {
            for event in &turn.response {
                if let UserFacingEvent::WorkCompleted { files_changed, .. } = event {
                    for file in files_changed {
                        if !changed_files.contains(file) {
                            changed_files.push(file.clone());
                        }
                    }
                }
                if let UserFacingEvent::ActionCompleted { description } = event {
                    if description.contains("Created") || description.contains("Updated") {
                        if let Some(path) = extract_path_from_desc(description) {
                            if !changed_files.contains(&path) {
                                changed_files.push(path);
                            }
                        }
                    }
                }
            }
        }

        if changed_files.is_empty() {
            return Ok(IntentSpec::Concrete {
                task: "Summarize: No files have been modified in recent turns.".to_string(),
                target_files: Vec::new(),
                constraints: vec!["summarize_only".to_string()],
                references: Vec::new(),
            });
        }

        let file_list = changed_files
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");

        Ok(IntentSpec::Concrete {
            task: format!("Summarize recent changes to: {}", file_list),
            target_files: changed_files,
            constraints: vec!["summarize_changes".to_string()],
            references: Vec::new(),
        })
    }

    /// Resolve "undo" / "revert"
    fn resolve_undo(&self, context: &SessionContext) -> Result<IntentSpec> {
        if let Some(last_turn) = context.conversation.last() {
            return Ok(IntentSpec::Concrete {
                task: format!("Undo changes from turn {}", last_turn.turn_id),
                target_files: Vec::new(),
                constraints: vec!["undo_last_turn".to_string()],
                references: vec![FollowUpReference::TurnRef(last_turn.turn_id)],
            });
        }

        Ok(IntentSpec::ResolutionFailed {
            reference: "undo".to_string(),
            reason: ResolutionFailureReason::NoRecentContext,
        })
    }

    /// Resolve anaphoric references ("that", "it", "this")
    fn resolve_anaphora(&self, content: &str, context: &SessionContext) -> Result<IntentSpec> {
        // Priority: most recent error > most recent file > most recent action

        // 1. Check for recent error
        if let Some(error) = context.recent_errors.first() {
            let task = format!(
                "Address the error in turn {}: {}",
                error.turn_id, error.summary
            );

            let mut references = vec![FollowUpReference::ErrorRef {
                turn_id: error.turn_id,
                error_summary: error.summary.clone(),
            }];

            let mut target_files = Vec::new();
            if let Some(ref file) = error.file {
                target_files.push(file.clone());
                references.push(FollowUpReference::FileRef(file.clone()));
            }

            return Ok(IntentSpec::Concrete {
                task,
                target_files,
                constraints: Vec::new(),
                references,
            });
        }

        // 2. Check for recent files
        if let Some(file) = context.recent_files.first() {
            // Extract what user wants to do from content
            let intent = infer_intent_from_context(content);

            return Ok(IntentSpec::Concrete {
                task: format!("{} {} (referencing recent work)", intent, file.display()),
                target_files: vec![file.clone()],
                constraints: Vec::new(),
                references: vec![FollowUpReference::FileRef(file.clone())],
            });
        }

        // 3. Fall back to recent turn
        if let Some(last_turn) = context.conversation.last() {
            return Ok(IntentSpec::Concrete {
                task: format!(
                    "Continue work from turn {}: {}",
                    last_turn.turn_id, last_turn.user_message.content
                ),
                target_files: Vec::new(),
                constraints: Vec::new(),
                references: vec![FollowUpReference::TurnRef(last_turn.turn_id)],
            });
        }

        // Nothing to reference
        Ok(IntentSpec::ClarificationNeeded {
            question: "What are you referring to? I don't see recent context to work with."
                .to_string(),
            options: Vec::new(),
            ambiguity_type: AmbiguityType::UnclearTarget,
        })
    }
}

impl Default for IntentRefiner {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract path from action description
fn extract_path_from_desc(desc: &str) -> Option<std::path::PathBuf> {
    let verbs = ["Created", "Updated", "Deleted", "Read"];
    for verb in &verbs {
        if let Some(pos) = desc.find(verb) {
            let after = &desc[pos + verb.len()..];
            let path_str = after
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(&['(', ')', '"', '\''][..]);
            if !path_str.is_empty() && !path_str.starts_with('(') {
                return Some(std::path::PathBuf::from(path_str));
            }
        }
    }
    None
}

/// Infer intent verb from context
fn infer_intent_from_context(content: &str) -> &'static str {
    let lower = content.to_lowercase();
    if lower.contains("fix") || lower.contains("correct") {
        "Fix issues in"
    } else if lower.contains("improve") || lower.contains("clean") || lower.contains("refactor") {
        "Improve"
    } else if lower.contains("add") || lower.contains("implement") {
        "Add to"
    } else if lower.contains("remove") || lower.contains("delete") {
        "Remove from"
    } else if lower.contains("test") || lower.contains("check") {
        "Test"
    } else if lower.contains("explain") || lower.contains("describe") {
        "Explain"
    } else {
        "Work on"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ConversationTurn, UserMessage};
    use chrono::Local;

    fn create_test_context() -> SessionContext {
        let mut context = SessionContext::new();

        // Add a completed turn
        let turn = ConversationTurn {
            turn_id: 1,
            user_message: UserMessage::new("create hello.txt"),
            intent_spec: Some(IntentSpec::Concrete {
                task: "create hello.txt".to_string(),
                target_files: vec![std::path::PathBuf::from("hello.txt")],
                constraints: Vec::new(),
                references: Vec::new(),
            }),
            execution_id: Some("exec-1".to_string()),
            response: vec![
                UserFacingEvent::ActionCompleted {
                    description: "Created hello.txt".to_string(),
                },
                UserFacingEvent::WorkCompleted {
                    summary: "Task completed".to_string(),
                    files_changed: vec![std::path::PathBuf::from("hello.txt")],
                },
            ],
            completed_at: Some(Local::now()),
        };

        context.add_turn(turn);
        context
            .recent_files
            .push(std::path::PathBuf::from("hello.txt"));

        context
    }

    #[test]
    fn test_direct_task() {
        let refiner = IntentRefiner::new();
        let context = SessionContext::new();
        let msg = UserMessage::new("create a new file");

        let result = refiner.refine(&msg, &context).unwrap();
        assert!(result.is_concrete());
        assert_eq!(result.task_description(), Some("create a new file"));
    }

    #[test]
    fn test_continue_no_context() {
        let refiner = IntentRefiner::new();
        let context = SessionContext::new();
        let msg = UserMessage::new("continue");

        let result = refiner.refine(&msg, &context).unwrap();
        assert!(!result.is_concrete());
    }

    #[test]
    fn test_continue_with_context() {
        let refiner = IntentRefiner::new();
        let context = create_test_context();
        let msg = UserMessage::new("continue");

        let result = refiner.refine(&msg, &context).unwrap();
        assert!(result.is_concrete());
        assert!(result.task_description().unwrap().contains("Continue"));
    }

    #[test]
    fn test_that_resolution() {
        let refiner = IntentRefiner::new();
        let context = create_test_context();
        let msg = UserMessage::new("fix that");

        let result = refiner.refine(&msg, &context).unwrap();
        // Should resolve to the file since there's no error
        assert!(result.is_concrete());
    }
}
