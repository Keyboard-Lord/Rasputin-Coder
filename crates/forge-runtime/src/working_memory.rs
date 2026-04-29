//! Working Memory for Forge Bootstrap Worker
//!
//! Provides Codex-like continuity by detecting follow-up intents and
//! resolving them against the persisted chain state.

use crate::state::AgentState;
use std::collections::HashSet;
use std::path::PathBuf;

/// Compact working memory for the current session/run
#[derive(Debug, Clone, Default)]
pub struct WorkingMemory {
    /// Original raw prompt that started the current task
    pub original_intent: String,
    /// Summarized objective
    pub objective_summary: String,
    /// Files recently modified
    pub recent_files_changed: Vec<PathBuf>,
    /// Whether the current task is complete
    pub is_complete: bool,
    /// Missing deliverables (extracted from task patterns)
    pub missing_deliverables: Vec<String>,
}

/// Follow-up intent types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpIntent {
    Continue,
    FixThat,
    DoTheRest,
    Improve,
    Validate,
    FinishRemaining,
    NewTask,
}

/// Extract potential deliverable filenames from task text using simple pattern matching
fn extract_deliverables_from_task(task: &str) -> Vec<String> {
    let mut deliverables = Vec::new();
    
    // Look for numbered filenames like "1. docs/file.md" or "2) config/app.toml"
    for line in task.lines() {
        let trimmed = line.trim();
        
        // Check for "N. path/to/file.ext" pattern
        if let Some(pos) = trimmed.find(".") {
            let prefix = &trimmed[..pos];
            if prefix.trim().parse::<u32>().is_ok() {
                // This line starts with a number, extract the path after the dot
                let after_number = &trimmed[pos+1..].trim();
                // Find first whitespace-separated token that looks like a path
                if let Some(first_token) = after_number.split_whitespace().next() {
                    if first_token.contains('/') && first_token.contains('.') {
                        deliverables.push(first_token.to_string());
                    }
                }
            }
        }
        
        // Also check for config/ docs/ src/ patterns anywhere in line
        for word in trimmed.split_whitespace() {
            let word = word.trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == ',');
            if (word.starts_with("docs/") || word.starts_with("config/") || word.starts_with("src/"))
                && word.contains('.')
                && !word.contains("http")
            {
                deliverables.push(word.to_string());
            }
        }
    }
    
    // Deduplicate
    let seen: HashSet<_> = deliverables.iter().cloned().collect();
    seen.into_iter().collect()
}

/// Check which deliverables exist
fn check_missing_deliverables(deliverables: &[String]) -> Vec<String> {
    deliverables
        .iter()
        .filter(|path| {
            let path_buf = PathBuf::from(path);
            !path_buf.exists()
        })
        .cloned()
        .collect()
}

impl WorkingMemory {
    /// Build working memory from agent state
    pub fn from_state(state: &AgentState) -> Option<Self> {
        eprintln!("[WORKING_MEMORY] from_state called with task='{}', files_written={}", state.task, state.files_written.len());
        if state.task.is_empty() {
            eprintln!("[WORKING_MEMORY] returning None: task is empty");
            return None;
        }

        // Extract potential deliverables from task
        let potential_deliverables = extract_deliverables_from_task(&state.task);
        let missing_deliverables = check_missing_deliverables(&potential_deliverables);

        // Get recent files from change history and files_written
        let mut recent_files: HashSet<PathBuf> = state
            .change_history
            .iter()
            .map(|record| record.mutation.path.clone())
            .collect();
        
        // Also include files_written
        for file in &state.files_written {
            recent_files.insert(file.clone());
        }

        let result = Some(Self {
            original_intent: state.task.clone(),
            objective_summary: state.task.split('\n').next().unwrap_or(&state.task).to_string(),
            recent_files_changed: recent_files.into_iter().collect(),
            is_complete: false, // Will be set by completion gate
            missing_deliverables,
        });
        eprintln!("[WORKING_MEMORY] from_state returning Some with {} recent files, {} missing deliverables", 
            result.as_ref().unwrap().recent_files_changed.len(),
            result.as_ref().unwrap().missing_deliverables.len());
        result
    }

    /// Detect if input is a follow-up command
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
        if lower.starts_with("fix that") || lower.starts_with("fix it") || lower.starts_with("try again") {
            return FollowUpIntent::FixThat;
        }
        if lower.starts_with("do the rest")
            || lower.starts_with("finish the rest")
            || lower.starts_with("complete the rest")
        {
            return FollowUpIntent::DoTheRest;
        }
        if lower.starts_with("make it cleaner")
            || lower.starts_with("clean it up")
            || lower.starts_with("improve it")
            || lower.starts_with("polish it")
        {
            return FollowUpIntent::Improve;
        }
        if lower.starts_with("now validate") || lower.starts_with("validate it") || lower.starts_with("check it") {
            return FollowUpIntent::Validate;
        }
        if lower.starts_with("finish the remaining") || lower.starts_with("complete the remaining") {
            return FollowUpIntent::FinishRemaining;
        }
        if words.len() >= 2 && words[0] == "continue" {
            return FollowUpIntent::Continue;
        }

        FollowUpIntent::NewTask
    }

    /// Resolve a follow-up intent into a concrete task
    pub fn resolve_follow_up(&self, intent: FollowUpIntent) -> Option<String> {
        eprintln!("[WORKING_MEMORY] resolve_follow_up called with intent {:?}", intent);
        let result = match intent {
            FollowUpIntent::Continue => {
                if !self.missing_deliverables.is_empty() {
                    let remaining = self.missing_deliverables.join(", ");
                    Some(format!(
                        "Continue working on: {}. Complete remaining: {}",
                        self.objective_summary, remaining
                    ))
                } else {
                    Some(format!(
                        "Continue working on: {}. Original: {}",
                        self.objective_summary, self.original_intent
                    ))
                }
            }
            FollowUpIntent::FixThat => {
                Some(format!(
                    "Fix issues in: {}. Original: {}",
                    self.objective_summary, self.original_intent
                ))
            }
            FollowUpIntent::DoTheRest | FollowUpIntent::FinishRemaining => {
                if !self.missing_deliverables.is_empty() {
                    let remaining = self.missing_deliverables.join(", ");
                    Some(format!(
                        "Complete the remaining deliverables: {} for task: {}",
                        remaining, self.objective_summary
                    ))
                } else {
                    Some(format!(
                        "Complete any remaining work for: {}",
                        self.objective_summary
                    ))
                }
            }
            FollowUpIntent::Improve => {
                if !self.recent_files_changed.is_empty() {
                    let files = self
                        .recent_files_changed
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    Some(format!(
                        "Improve quality of recently modified files: {}. Context: {}",
                        files, self.objective_summary
                    ))
                } else {
                    Some(format!("Improve quality of work on: {}", self.objective_summary))
                }
            }
            FollowUpIntent::Validate => {
                Some(format!(
                    "Validate the current state of: {}",
                    self.objective_summary
                ))
            }
            FollowUpIntent::NewTask => None,
        };
        eprintln!("[WORKING_MEMORY] resolve_follow_up returning {:?}", result.as_ref().map(|s| &s[..20.min(s.len())]));
        result
    }

    /// Check if there's meaningful context to continue from
    pub fn has_context(&self) -> bool {
        !self.original_intent.is_empty()
    }
}

/// Check if a task should be resolved as a follow-up
pub fn resolve_follow_up_task(state: &AgentState, input_task: &str) -> Option<String> {
    let intent = WorkingMemory::detect_follow_up_intent(input_task);
    if matches!(intent, FollowUpIntent::NewTask) {
        return None;
    }

    let memory = WorkingMemory::from_state(state)?;
    if !memory.has_context() {
        return None;
    }

    memory.resolve_follow_up(intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_continue_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("continue"),
            FollowUpIntent::Continue
        ));
    }

    #[test]
    fn detects_fix_that_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("fix that"),
            FollowUpIntent::FixThat
        ));
    }

    #[test]
    fn detects_do_the_rest_intent() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("do the rest"),
            FollowUpIntent::DoTheRest
        ));
    }

    #[test]
    fn new_task_returns_none() {
        assert!(matches!(
            WorkingMemory::detect_follow_up_intent("Create a new function"),
            FollowUpIntent::NewTask
        ));
    }

    #[test]
    fn resolve_continue_with_missing_deliverables() {
        let memory = WorkingMemory {
            objective_summary: "Create docs".to_string(),
            original_intent: "Create docs/01.md and docs/02.md".to_string(),
            missing_deliverables: vec!["docs/02.md".to_string()],
            is_complete: false,
            ..Default::default()
        };

        let resolved = memory.resolve_follow_up(FollowUpIntent::Continue);
        assert!(resolved.is_some());
        let task = resolved.unwrap();
        assert!(task.contains("Create docs"));
        assert!(task.contains("docs/02.md"));
    }
}
