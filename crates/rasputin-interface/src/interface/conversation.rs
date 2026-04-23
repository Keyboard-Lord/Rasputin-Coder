//! Conversation transcript management
//!
//! Phase 2 implementation: Full conversation history with context tracking

use crate::types::{ConversationTurn, SessionContext, UserFacingEvent};
use std::collections::VecDeque;

/// Maximum turns to retain in memory
const MAX_HISTORY_TURNS: usize = 50;

/// Manages conversation history and turn sequencing
#[derive(Debug, Clone)]
pub struct ConversationManager {
    next_turn_id: u32,
    turns: VecDeque<ConversationTurn>,
    current_turn: Option<ConversationTurn>,
}

impl ConversationManager {
    pub fn new() -> Self {
        Self {
            next_turn_id: 1,
            turns: VecDeque::with_capacity(MAX_HISTORY_TURNS),
            current_turn: None,
        }
    }

    /// Start a new conversation turn
    pub fn start_turn(&mut self, user_message: super::UserMessage) -> &ConversationTurn {
        let turn = ConversationTurn::new(self.next_turn_id, user_message);
        self.next_turn_id += 1;
        self.current_turn = Some(turn);
        self.current_turn.as_ref().unwrap()
    }

    /// Add an event to the current turn
    pub fn add_event(&mut self, event: UserFacingEvent) {
        if let Some(ref mut turn) = self.current_turn {
            turn.add_event(event);
        }
    }

    /// Complete the current turn and archive it
    pub fn complete_turn(
        &mut self,
        intent_spec: Option<crate::types::IntentSpec>,
        execution_id: Option<String>,
    ) {
        if let Some(mut turn) = self.current_turn.take() {
            turn.intent_spec = intent_spec;
            turn.execution_id = execution_id;
            turn.mark_completed();

            // Add to history
            self.turns.push_back(turn);

            // Trim old turns
            while self.turns.len() > MAX_HISTORY_TURNS {
                self.turns.pop_front();
            }
        }
    }

    /// Get the current in-progress turn
    pub fn current_turn(&self) -> Option<&ConversationTurn> {
        self.current_turn.as_ref()
    }

    /// Get recent completed turns (excluding current)
    pub fn recent_turns(&self, count: usize) -> Vec<&ConversationTurn> {
        self.turns.iter().rev().take(count).collect()
    }

    /// Get last N turns including current if present
    pub fn last_turns(&self, count: usize) -> Vec<&ConversationTurn> {
        let mut result: Vec<&ConversationTurn> = self.turns.iter().rev().take(count).collect();
        if let Some(ref current) = self.current_turn {
            result.insert(0, current);
        }
        result
    }

    /// Get a specific turn by ID
    pub fn get_turn(&self, turn_id: u32) -> Option<&ConversationTurn> {
        self.turns
            .iter()
            .find(|t| t.turn_id == turn_id)
            .or_else(|| self.current_turn.as_ref().filter(|t| t.turn_id == turn_id))
    }

    /// Get the most recent completed turn
    pub fn last_completed_turn(&self) -> Option<&ConversationTurn> {
        self.turns.back()
    }

    /// Check if there's an in-progress turn
    pub fn has_current_turn(&self) -> bool {
        self.current_turn.is_some()
    }

    /// Build SessionContext from conversation history
    pub fn build_context(&self) -> SessionContext {
        let mut context = SessionContext::new();

        // Extract recent files from all turns
        for turn in self.turns.iter().rev().take(10) {
            for event in &turn.response {
                if let UserFacingEvent::ActionCompleted { description } = event {
                    // Extract file paths from descriptions like "Read src/main.rs"
                    if let Some(path) = extract_path_from_description(description) {
                        if !context.recent_files.contains(&path) {
                            context.recent_files.push(path);
                        }
                    }
                }

                // Extract errors
                if let UserFacingEvent::ActionFailed { error, .. } = event {
                    context.recent_errors.push(crate::types::ErrorRecord {
                        turn_id: turn.turn_id,
                        timestamp: turn.user_message.timestamp,
                        summary: error.clone(),
                        file: None,
                    });
                }
            }
        }

        // Keep only most recent 10 files and 5 errors
        context.recent_files.truncate(10);
        context.recent_errors.truncate(5);

        context
    }

    /// Clear all history
    pub fn clear(&mut self) {
        self.turns.clear();
        self.current_turn = None;
        self.next_turn_id = 1;
    }

    /// Total turns in history
    pub fn history_count(&self) -> usize {
        self.turns.len()
    }
}

impl Default for ConversationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract a file path from an action description
/// e.g., "Read src/main.rs (142 lines)" -> "src/main.rs"
fn extract_path_from_description(description: &str) -> Option<std::path::PathBuf> {
    // Simple heuristic: find the word after the verb
    let verbs = [
        "Read", "Created", "Updated", "Deleted", "Writing", "Reading",
    ];

    for verb in &verbs {
        if let Some(pos) = description.find(verb) {
            let after_verb = &description[pos + verb.len()..];
            // Extract path (next word or quoted string)
            let path_str = after_verb
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UserMessage;

    #[test]
    fn test_turn_lifecycle() {
        let mut mgr = ConversationManager::new();

        let msg = UserMessage::new("create hello.txt");
        mgr.start_turn(msg);

        mgr.add_event(UserFacingEvent::ActionStarted {
            description: "Writing hello.txt...".to_string(),
        });

        mgr.complete_turn(None, None);

        assert_eq!(mgr.history_count(), 1);
        assert!(mgr.last_completed_turn().is_some());
    }

    #[test]
    fn test_extract_path() {
        assert_eq!(
            extract_path_from_description("Read src/main.rs (142 lines)"),
            Some(std::path::PathBuf::from("src/main.rs"))
        );

        assert_eq!(
            extract_path_from_description("Created hello.txt"),
            Some(std::path::PathBuf::from("hello.txt"))
        );

        assert_eq!(extract_path_from_description("Validation passed"), None);
    }
}
