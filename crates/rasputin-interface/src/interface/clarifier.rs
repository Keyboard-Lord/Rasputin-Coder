//! Ambiguity detection and clarification question generation
//!
//! Phase 4 implementation: Full ambiguity detection with contextual suggestions

use crate::types::{AmbiguityType, SessionContext};

/// Generates clarification questions for ambiguous intents
#[derive(Debug, Clone)]
pub struct Clarifier {
    /// Minimum confidence threshold before asking for clarification
    confidence_threshold: f32,
}

impl Clarifier {
    pub fn new() -> Self {
        Self {
            confidence_threshold: 0.7,
        }
    }

    /// Set confidence threshold
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.confidence_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Detect ambiguity in user intent
    ///
    /// Returns Some(AmbiguityType) if clarification is needed, None if clear
    pub fn detect_ambiguity(
        &self,
        content: &str,
        context: &SessionContext,
    ) -> Option<AmbiguityType> {
        let lower = content.to_lowercase();

        // Check for explicit vague terms
        if self.is_explicitly_vague(&lower) {
            // Determine what kind of vagueness
            if self.has_anaphora_without_context(&lower, context) {
                return Some(AmbiguityType::UnclearTarget);
            }

            if self.has_multiple_candidates(content, context) {
                return Some(AmbiguityType::MultipleCandidates);
            }

            if self.has_missing_context(content, context) {
                return Some(AmbiguityType::MissingContext);
            }
        }

        // Check for vague quality requests without target
        if self.is_vague_quality_request(&lower) && context.recent_files.is_empty() {
            return Some(AmbiguityType::VagueConstraint);
        }

        // Check for under-specified modifications
        if self.is_underspecified_change(&lower) {
            // If multiple files were recently touched, need to specify which
            if context.recent_files.len() > 1 {
                return Some(AmbiguityType::MultipleCandidates);
            }
            // If no context at all, missing context
            if context.recent_files.is_empty() && context.conversation.is_empty() {
                return Some(AmbiguityType::MissingContext);
            }
        }

        None
    }

    /// Generate clarification question for detected ambiguity
    pub fn generate_question(&self, ambiguity: AmbiguityType, context: &SessionContext) -> String {
        let base_question = match ambiguity {
            AmbiguityType::UnclearTarget => {
                "What are you referring to? I don't see a clear reference.".to_string()
            }
            AmbiguityType::MultipleCandidates => {
                "Which one? I see multiple possibilities.".to_string()
            }
            AmbiguityType::MissingContext => {
                "I'm not sure what you're referring to. Could you clarify?".to_string()
            }
            AmbiguityType::VagueConstraint => {
                "Could you be more specific about what you'd like me to do?".to_string()
            }
        };

        // Add suggestions if available
        let suggestions = self.suggest_options(context);
        if suggestions.is_empty() {
            base_question
        } else {
            format!(
                "{}\n\nSuggestions:\n{}",
                base_question,
                suggestions
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("  {}. {}", i + 1, s))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    }

    /// Suggest options based on context
    pub fn suggest_options(&self, context: &SessionContext) -> Vec<String> {
        let mut options = Vec::new();

        // Suggest recent files
        for file in context.recent_files.iter().take(3) {
            options.push(format!("{}", file.display()));
        }

        // Suggest recent errors
        for error in context.recent_errors.iter().take(2) {
            options.push(format!(
                "Error in turn {}: {}",
                error.turn_id, error.summary
            ));
        }

        // Suggest actions from last turn
        if let Some(last_turn) = context.conversation.last() {
            if !last_turn.user_message.content.is_empty() {
                options.push(format!(
                    "Continue: {}",
                    truncate(&last_turn.user_message.content, 40)
                ));
            }
        }

        options
    }

    /// Check if the user explicitly used vague terms
    fn is_explicitly_vague(&self, lower: &str) -> bool {
        let vague_terms = [
            "that", "it", "this", "thing", "stuff", "here", "there", "those", "these",
        ];
        vague_terms.iter().any(|term| {
            lower.contains(&format!(" {} ", term))
                || lower.starts_with(&format!("{} ", term))
                || lower.ends_with(&format!(" {}", term))
        })
    }

    /// Check for anaphora without sufficient context
    fn has_anaphora_without_context(&self, lower: &str, context: &SessionContext) -> bool {
        let anaphora = ["that", "it", "this"];
        let has_anaphora = anaphora.iter().any(|a| lower.contains(a));

        // If we have recent context, it's probably fine
        if has_anaphora && context.recent_files.is_empty() && context.recent_errors.is_empty() {
            return true;
        }

        false
    }

    /// Check if there are multiple candidates for a reference
    fn has_multiple_candidates(&self, _content: &str, context: &SessionContext) -> bool {
        // Multiple recent files = ambiguous which one
        context.recent_files.len() > 1
    }

    /// Check if context is completely missing
    fn has_missing_context(&self, content: &str, context: &SessionContext) -> bool {
        let follow_up_commands = ["continue", "fix that", "try again", "what changed"];
        let lower = content.to_lowercase();

        // If it's a follow-up command but no history
        follow_up_commands.iter().any(|cmd| lower.contains(cmd)) && context.conversation.is_empty()
    }

    /// Check for vague quality improvement requests
    fn is_vague_quality_request(&self, lower: &str) -> bool {
        let quality_terms = [
            "better",
            "cleaner",
            "nicer",
            "improve",
            "optimize",
            "refactor",
            "enhance",
            "polish",
            "fix it up",
        ];
        quality_terms.iter().any(|term| lower.contains(term))
    }

    /// Check for underspecified change requests
    fn is_underspecified_change(&self, lower: &str) -> bool {
        let change_terms = [
            "change", "update", "modify", "edit", "adjust", "tweak", "fix", "correct",
        ];
        change_terms.iter().any(|term| lower.contains(term)) &&
            !lower.contains(" in ") &&  // No target specified
            !lower.contains(" to ") // No destination specified
    }

    /// Calculate confidence score for an intent
    pub fn calculate_confidence(&self, content: &str, context: &SessionContext) -> f32 {
        let lower = content.to_lowercase();
        let mut score = 1.0f32;

        // Reduce score for vague terms
        if self.is_explicitly_vague(&lower) {
            score -= 0.3;
        }

        // Reduce score for missing context
        if context.recent_files.is_empty() && (lower.contains("that") || lower.contains("it")) {
            score -= 0.4;
        }

        // Reduce score for multiple candidates
        if context.recent_files.len() > 1 {
            score -= 0.2;
        }

        // Boost score for clear file mentions
        if lower.contains("file") || lower.contains(".") {
            score += 0.1;
        }

        // Boost score for clear action verbs at start
        let clear_actions = ["create", "write", "add", "delete", "read", "show"];
        for action in &clear_actions {
            if lower.starts_with(action) {
                score += 0.1;
                break;
            }
        }

        score.clamp(0.0, 1.0)
    }

    /// Should we ask for clarification?
    pub fn should_clarify(&self, content: &str, context: &SessionContext) -> bool {
        let confidence = self.calculate_confidence(content, context);
        confidence < self.confidence_threshold || self.detect_ambiguity(content, context).is_some()
    }
}

impl Default for Clarifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate string with ellipsis
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_context_with_files() -> SessionContext {
        let mut context = SessionContext::new();
        context
            .recent_files
            .push(std::path::PathBuf::from("src/main.rs"));
        context
            .recent_files
            .push(std::path::PathBuf::from("src/lib.rs"));
        context
    }

    fn create_empty_context() -> SessionContext {
        SessionContext::new()
    }

    #[test]
    fn test_detect_unclear_target() {
        let clarifier = Clarifier::new();
        let context = create_empty_context();

        // "Fix that" with no context should be unclear
        let result = clarifier.detect_ambiguity("fix that", &context);
        assert!(matches!(
            result,
            Some(AmbiguityType::UnclearTarget) | Some(AmbiguityType::MissingContext)
        ));
    }

    #[test]
    fn test_detect_multiple_candidates() {
        let clarifier = Clarifier::new();
        let context = create_context_with_files();

        // "Update it" with multiple files should be ambiguous
        let result = clarifier.detect_ambiguity("update it", &context);
        assert!(matches!(
            result,
            Some(AmbiguityType::MultipleCandidates) | Some(AmbiguityType::UnclearTarget)
        ));
    }

    #[test]
    fn test_vague_quality_request() {
        let clarifier = Clarifier::new();
        let context = create_empty_context();

        // "Make code better" with no files (avoiding 'it' anaphora)
        let result = clarifier.detect_ambiguity("make code better", &context);
        assert!(matches!(result, Some(AmbiguityType::VagueConstraint)));
    }

    #[test]
    fn test_suggest_options() {
        let clarifier = Clarifier::new();
        let context = create_context_with_files();

        let options = clarifier.suggest_options(&context);
        assert!(!options.is_empty());
        assert!(options.iter().any(|o| o.contains("main.rs")));
    }

    #[test]
    fn test_confidence_calculation() {
        let clarifier = Clarifier::new();

        // Clear direct request should have high confidence
        let context = create_empty_context();
        let confidence =
            clarifier.calculate_confidence("create a new file called test.txt", &context);
        assert!(confidence > 0.7);

        // Vague request should have low confidence
        let confidence = clarifier.calculate_confidence("fix that thing", &context);
        assert!(confidence < 0.7);
    }
}
