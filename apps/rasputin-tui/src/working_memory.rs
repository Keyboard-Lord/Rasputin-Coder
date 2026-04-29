//! Working Memory - Active session context for Codex-like continuity
//!
//! Maintains compact, bounded session memory that carries intent across turns.
//! This enables follow-up prompts like "continue", "fix that", "do the rest"
//! without requiring the user to restate the entire task.

use crate::persistence::{ExecutionOutcome, PersistentChain, PersistentState};
use crate::state::ArtifactCompletionContract;
use chrono::{DateTime, Local};
use std::collections::HashSet;
use std::path::PathBuf;

/// Compact working memory for the current session/run
#[derive(Debug, Clone, Default)]
pub struct WorkingMemory {
    /// Original raw prompt that started the current task
    pub original_intent: String,
    /// Summarized objective (for display/quick reference)
    pub objective_summary: String,
    /// Active artifact contract if any
    pub artifact_contract: Option<ArtifactCompletionContract>,
    /// Files recently modified in this session
    pub recent_files_changed: Vec<PathBuf>,
    /// Most recent validation result (pass/fail/message)
    pub last_validation_result: Option<ValidationSnapshot>,
    /// Last blocker or failure that stopped progress
    pub last_blocker: Option<BlockerRecord>,
    /// Current recommended next action
    pub next_recommended_action: Option<String>,
    /// Unresolved TODOs or missing deliverables
    pub unresolved_deliverables: Vec<String>,
    /// When this working memory was last updated
    pub updated_at: DateTime<Local>,
    /// Chain ID this memory is associated with
    pub active_chain_id: Option<String>,
    /// Whether the current task is complete
    pub is_complete: bool,
    /// Session turn counter for continuity tracking
    pub session_turn: u32,
}

/// Snapshot of validation state
#[derive(Debug, Clone)]
pub struct ValidationSnapshot {
    pub passed: bool,
    pub message: String,
    pub timestamp: DateTime<Local>,
}

/// Record of what blocked execution
#[derive(Debug, Clone)]
pub struct BlockerRecord {
    pub reason: String,
    pub step_id: Option<String>,
    pub recoverable: bool,
    pub timestamp: DateTime<Local>,
}

/// Follow-up intent types that can be resolved against working memory
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpIntent {
    /// "continue" - resume current work
    Continue,
    /// "fix that" / "fix it" - address last failure
    FixThat,
    /// "do the rest" / "finish" - complete remaining work
    DoTheRest,
    /// "make it cleaner" / "improve it" - quality pass
    Improve,
    /// "now validate it" / "check it" - validation request
    Validate,
    /// "finish the remaining docs" - specific deliverable completion
    FinishRemaining,
    /// Not a follow-up, requires full interpretation
    NewTask,
}

impl WorkingMemory {
    /// Create fresh working memory from a new task
    pub fn from_intent(intent: &str, objective_summary: String) -> Self {
        Self {
            original_intent: intent.to_string(),
            objective_summary,
            updated_at: Local::now(),
            session_turn: 1,
            ..Default::default()
        }
    }

    /// Build working memory from an active chain
    pub fn from_chain(chain: &PersistentChain) -> Self {
        let contract = chain.objective_satisfaction.artifact_contract.clone();
        let unresolved = Self::extract_unresolved_deliverables(chain, &contract);

        Self {
            original_intent: chain.raw_prompt_text().to_string(),
            objective_summary: chain.objective.clone(),
            artifact_contract: contract,
            recent_files_changed: chain.recorded_affected_paths()
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            last_validation_result: Self::extract_last_validation(chain),
            last_blocker: Self::extract_last_blocker(chain),
            next_recommended_action: None, // Computed on-demand
            unresolved_deliverables: unresolved,
            updated_at: Local::now(),
            active_chain_id: Some(chain.id.clone()),
            is_complete: chain.get_outcome()
                .map(|o| matches!(o, ExecutionOutcome::Success | ExecutionOutcome::SuccessWithWarnings))
                .unwrap_or(false),
            session_turn: 0,
        }
    }

    /// Refresh working memory from current chain state
    pub fn refresh(&mut self, chain: &PersistentChain) {
        self.artifact_contract = chain.objective_satisfaction.artifact_contract.clone();
        self.recent_files_changed = chain.recorded_affected_paths()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        self.last_validation_result = Self::extract_last_validation(chain);
        self.last_blocker = Self::extract_last_blocker(chain);
        self.unresolved_deliverables = Self::extract_unresolved_deliverables(chain, &self.artifact_contract);
        self.is_complete = chain.get_outcome()
            .map(|o| matches!(o, ExecutionOutcome::Success | ExecutionOutcome::SuccessWithWarnings))
            .unwrap_or(false);
        self.updated_at = Local::now();
    }

    /// Increment turn counter for session tracking
    pub fn increment_turn(&mut self) {
        self.session_turn += 1;
    }

    /// Detect if input is a follow-up command that can use working memory
    pub fn detect_follow_up_intent(input: &str) -> FollowUpIntent {
        let lower = input.trim().to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();

        // Direct single-word commands
        match lower.as_str() {
            "continue" | "resume" | "proceed" => return FollowUpIntent::Continue,
            "finish" | "complete" => return FollowUpIntent::DoTheRest,
            "validate" | "check" => return FollowUpIntent::Validate,
            _ => {}
        }

        // Phrase patterns
        if lower.starts_with("fix that") || lower.starts_with("fix it") {
            return FollowUpIntent::FixThat;
        }
        if lower.starts_with("try again") || lower.starts_with("retry") {
            return FollowUpIntent::FixThat;
        }
        if lower.starts_with("do the rest") 
            || lower.starts_with("finish the rest")
            || lower.starts_with("complete the rest")
            || lower.starts_with("handle the rest") {
            return FollowUpIntent::DoTheRest;
        }
        if lower.starts_with("make it cleaner")
            || lower.starts_with("clean it up")
            || lower.starts_with("improve it")
            || lower.starts_with("polish it")
            || lower.starts_with("refine it") {
            return FollowUpIntent::Improve;
        }
        if lower.starts_with("now validate")
            || lower.starts_with("validate it")
            || lower.starts_with("check it")
            || lower.starts_with("test it") {
            return FollowUpIntent::Validate;
        }
        if lower.starts_with("finish the remaining")
            || lower.starts_with("complete the remaining")
            || lower.starts_with("do the remaining") {
            return FollowUpIntent::FinishRemaining;
        }

        // Context-dependent "continue" patterns
        if words.len() >= 2 && words[0] == "continue" {
            return FollowUpIntent::Continue;
        }

        FollowUpIntent::NewTask
    }

    /// Resolve a follow-up intent into a concrete task using working memory
    pub fn resolve_follow_up(&self, intent: FollowUpIntent, _user_input: &str) -> Option<String> {
        match intent {
            FollowUpIntent::Continue => {
                if self.is_complete {
                    Some(format!(
                        "Review completed work: {}. Original task: {}",
                        self.objective_summary,
                        self.original_intent
                    ))
                } else if !self.unresolved_deliverables.is_empty() {
                    let remaining = self.unresolved_deliverables.join(", ");
                    Some(format!(
                        "Continue working on: {}. Remaining deliverables: {}",
                        self.objective_summary, remaining
                    ))
                } else {
                    Some(format!(
                        "Continue working on: {}. Original task: {}",
                        self.objective_summary,
                        self.original_intent
                    ))
                }
            }
            FollowUpIntent::FixThat => {
                if let Some(ref blocker) = self.last_blocker {
                    Some(format!(
                        "Fix the blocker: {}. In context of: {}",
                        blocker.reason, self.objective_summary
                    ))
                } else {
                    Some(format!(
                        "Fix issues in: {}. Original task: {}",
                        self.objective_summary,
                        self.original_intent
                    ))
                }
            }
            FollowUpIntent::DoTheRest => {
                if !self.unresolved_deliverables.is_empty() {
                    let remaining = self.unresolved_deliverables.join(", ");
                    Some(format!(
                        "Complete the remaining work for: {}. Specifically finish: {}",
                        self.objective_summary, remaining
                    ))
                } else if let Some(ref contract) = self.artifact_contract {
                    let missing: Vec<_> = contract.required_filenames.iter()
                        .filter(|f| !contract.created_filenames.contains(f))
                        .cloned()
                        .collect();
                    if !missing.is_empty() {
                        let remaining = missing.join(", ");
                        Some(format!(
                            "Create the remaining required files: {} for task: {}",
                            remaining, self.objective_summary
                        ))
                    } else {
                        Some(format!(
                            "Complete any remaining work for: {}",
                            self.objective_summary
                        ))
                    }
                } else {
                    Some(format!(
                        "Complete the remaining work for: {}",
                        self.objective_summary
                    ))
                }
            }
            FollowUpIntent::Improve => {
                if !self.recent_files_changed.is_empty() {
                    let files = self.recent_files_changed.iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    Some(format!(
                        "Improve quality of recently modified files: {}. Context: {}",
                        files, self.objective_summary
                    ))
                } else {
                    Some(format!(
                        "Improve quality of work on: {}",
                        self.objective_summary
                    ))
                }
            }
            FollowUpIntent::Validate => {
                if let Some(ref validation) = self.last_validation_result {
                    if validation.passed {
                        Some(format!(
                            "Re-validate the completed work: {}. Last check passed but verify again.",
                            self.objective_summary
                        ))
                    } else {
                        Some(format!(
                            "Validate and fix issues in: {}. Previous validation failed with: {}",
                            self.objective_summary, validation.message
                        ))
                    }
                } else {
                    Some(format!(
                        "Validate the current state of: {}",
                        self.objective_summary
                    ))
                }
            }
            FollowUpIntent::FinishRemaining => {
                // Similar to DoTheRest but more specific about remaining items
                let mut deliverables = Vec::new();
                if let Some(ref contract) = self.artifact_contract {
                    let missing: Vec<_> = contract.required_filenames.iter()
                        .filter(|f| !contract.created_filenames.contains(f))
                        .cloned()
                        .collect();
                    deliverables.extend(missing);
                }
                deliverables.extend(self.unresolved_deliverables.clone());
                deliverables = deliverables.into_iter()
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();

                if !deliverables.is_empty() {
                    let remaining = deliverables.join(", ");
                    Some(format!(
                        "Finish the remaining deliverables: {} for task: {}",
                        remaining, self.objective_summary
                    ))
                } else {
                    Some(format!(
                        "Finish any remaining work for: {}",
                        self.objective_summary
                    ))
                }
            }
            FollowUpIntent::NewTask => None,
        }
    }

    /// Format working memory as a concise context block for planner prompts
    pub fn format_context_block(&self) -> String {
        let mut lines = vec![
            "=== WORKING CONTEXT ===".to_string(),
            format!("Objective: {}", self.objective_summary),
        ];

        if !self.original_intent.is_empty() && self.original_intent != self.objective_summary {
            let truncated = if self.original_intent.len() > 200 {
                format!("{}...", &self.original_intent[..200])
            } else {
                self.original_intent.clone()
            };
            lines.push(format!("Original Task: {}", truncated));
        }

        // Artifact contract status
        if let Some(ref contract) = self.artifact_contract {
            if contract.has_requirements() {
                let total = contract.required_filenames.len();
                let created = contract.created_filenames.len();
                let missing = total.saturating_sub(created);
                lines.push(format!(
                    "Deliverables: {}/{} required artifacts created",
                    created, total
                ));
                if missing > 0 && missing <= 5 {
                    let remaining: Vec<_> = contract.required_filenames.iter()
                        .filter(|f| !contract.created_filenames.contains(f))
                        .cloned()
                        .collect();
                    lines.push(format!("Still needed: {}", remaining.join(", ")));
                }
            }
        }

        // Recent files
        if !self.recent_files_changed.is_empty() {
            let recent: Vec<_> = self.recent_files_changed.iter()
                .rev()
                .take(5)
                .map(|p| p.display().to_string())
                .collect();
            lines.push(format!("Recently modified: {}", recent.join(", ")));
        }

        // Last validation
        if let Some(ref validation) = self.last_validation_result {
            let status = if validation.passed { "✓" } else { "✗" };
            lines.push(format!("Last validation: {} {}", status, validation.message));
        }

        // Blocker
        if let Some(ref blocker) = self.last_blocker {
            let recovery = if blocker.recoverable { "(recoverable)" } else { "(blocked)" };
            lines.push(format!("Current blocker: {} {}", blocker.reason, recovery));
        }

        // Unresolved deliverables
        if !self.unresolved_deliverables.is_empty() {
            lines.push(format!(
                "Unresolved: {}",
                self.unresolved_deliverables.join(", ")
            ));
        }

        lines.push("=======================".to_string());
        lines.join("\n")
    }

    /// Check if there's meaningful context to continue from
    pub fn has_context_for_continuation(&self) -> bool {
        !self.objective_summary.is_empty()
            && (!self.is_complete || !self.unresolved_deliverables.is_empty())
    }

    // Helper methods

    fn extract_last_validation(chain: &PersistentChain) -> Option<ValidationSnapshot> {
        // Look through steps for validation results
        for step in chain.steps.iter().rev() {
            if let Some(ref outcome) = step.execution_outcome {
                let (passed, message) = match outcome {
                    ExecutionOutcome::Success => (true, "Step completed successfully".to_string()),
                    ExecutionOutcome::SuccessWithWarnings => (true, "Completed with warnings".to_string()),
                    ExecutionOutcome::Blocked => (false, "Step blocked".to_string()),
                    ExecutionOutcome::Failed => (false, "Step failed".to_string()),
                };
                return Some(ValidationSnapshot {
                    passed,
                    message,
                    timestamp: step.completed_at.unwrap_or_else(Local::now),
                });
            }
        }
        None
    }

    fn extract_last_blocker(chain: &PersistentChain) -> Option<BlockerRecord> {
        for step in chain.steps.iter().rev() {
            if let Some(ref failure) = step.failure_reason {
                return Some(BlockerRecord {
                    reason: failure.summary.clone(),
                    step_id: Some(step.id.clone()),
                    recoverable: step.recovery_step_kind.is_some(),
                    timestamp: step.completed_at.unwrap_or_else(Local::now),
                });
            }
        }
        // Check chain-level block
        if let Some(ExecutionOutcome::Blocked) = chain.get_outcome() {
            return Some(BlockerRecord {
                reason: "Chain execution blocked".to_string(),
                step_id: None,
                recoverable: true,
                timestamp: Local::now(),
            });
        }
        None
    }

    fn extract_unresolved_deliverables(
        chain: &PersistentChain,
        contract: &Option<ArtifactCompletionContract>,
    ) -> Vec<String> {
        let mut unresolved = Vec::new();

        // From artifact contract
        if let Some(c) = contract {
            for path in &c.required_filenames {
                if !c.created_filenames.contains(path) {
                    unresolved.push(path.clone());
                }
            }
        }

        // From step-level TODOs (if any recorded)
        for step in &chain.steps {
            if let Some(ref summary) = step.result_summary {
                if summary.contains("TODO") || summary.contains("todo") {
                    // Extract TODO items
                    for line in summary.lines() {
                        if line.contains("TODO") || line.contains("todo") {
                            unresolved.push(line.trim().to_string());
                        }
                    }
                }
            }
        }

        unresolved = unresolved.into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        unresolved.sort();
        unresolved
    }
}

/// Compute working memory from current app state
pub fn compute_working_memory(persistence: &PersistentState) -> Option<WorkingMemory> {
    if let Some(ref chain_id) = persistence.active_chain_id {
        if let Some(chain) = persistence.get_chain(chain_id) {
            return Some(WorkingMemory::from_chain(chain));
        }
    }
    None
}

/// Check if input should be routed to active work instead of chat
pub fn should_route_to_active_work(input: &str, persistence: &PersistentState) -> bool {
    let intent = WorkingMemory::detect_follow_up_intent(input);
    
    // Only route to active work if there's a valid chain and it's a follow-up
    if matches!(intent, FollowUpIntent::NewTask) {
        return false;
    }

    // Check if we have context to continue from
    if let Some(ref memory) = compute_working_memory(persistence) {
        return memory.has_context_for_continuation();
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_continue_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("continue"),
            FollowUpIntent::Continue
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("Continue working on this"),
            FollowUpIntent::Continue
        ));
    }

    #[test]
    fn detects_fix_that_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("fix that"),
            FollowUpIntent::FixThat
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("fix it"),
            FollowUpIntent::FixThat
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("try again"),
            FollowUpIntent::FixThat
        ));
    }

    #[test]
    fn detects_do_the_rest_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("do the rest"),
            FollowUpIntent::DoTheRest
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("finish the rest"),
            FollowUpIntent::DoTheRest
        ));
    }

    #[test]
    fn detects_improve_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("make it cleaner"),
            FollowUpIntent::Improve
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("improve it"),
            FollowUpIntent::Improve
        ));
    }

    #[test]
    fn detects_validate_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("now validate it"),
            FollowUpIntent::Validate
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("check it"),
            FollowUpIntent::Validate
        ));
    }

    #[test]
    fn detects_finish_remaining_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("finish the remaining files"),
            FollowUpIntent::FinishRemaining
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("complete the remaining docs"),
            FollowUpIntent::FinishRemaining
        ));
    }

    #[test]
    fn new_task_returns_none() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("Create a new function"),
            FollowUpIntent::NewTask
        ));
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("What is the weather?"),
            FollowUpIntent::NewTask
        ));
    }

    #[test]
    fn resolve_continue_with_unresolved_deliverables() {
        let memory = WorkingMemory {
            objective_summary: "Create docs".to_string(),
            original_intent: "Create docs/01.md and docs/02.md".to_string(),
            unresolved_deliverables: vec!["docs/02.md".to_string()],
            is_complete: false,
            ..Default::default()
        };

        let resolved = memory.resolve_follow_up(FollowUpIntent::Continue, "continue");
        assert!(resolved.is_some());
        let task = resolved.unwrap();
        assert!(task.contains("Create docs"));
        assert!(task.contains("docs/02.md"));
    }

    #[test]
    fn resolve_fix_that_with_blocker() {
        let memory = WorkingMemory {
            objective_summary: "Fix parser".to_string(),
            last_blocker: Some(BlockerRecord {
                reason: "Compilation error in line 42".to_string(),
                step_id: Some("step-1".to_string()),
                recoverable: true,
                timestamp: Local::now(),
            }),
            ..Default::default()
        };

        let resolved = memory.resolve_follow_up(FollowUpIntent::FixThat, "fix that");
        assert!(resolved.is_some());
        let task = resolved.unwrap();
        assert!(task.contains("Compilation error"));
    }

    #[test]
    fn format_context_block_includes_key_info() {
        let memory = WorkingMemory {
            objective_summary: "Create docs".to_string(),
            original_intent: "Create 2 docs".to_string(),
            artifact_contract: Some(ArtifactCompletionContract {
                required_filenames: vec!["a.md".to_string(), "b.md".to_string()],
                created_filenames: vec!["a.md".to_string()],
                ..Default::default()
            }),
            recent_files_changed: vec![PathBuf::from("a.md")],
            unresolved_deliverables: vec!["b.md".to_string()],
            ..Default::default()
        };

        let block = memory.format_context_block();
        assert!(block.contains("WORKING CONTEXT"));
        assert!(block.contains("Create docs"));
        assert!(block.contains("1/2 required artifacts"));
        assert!(block.contains("b.md"));
    }
}
