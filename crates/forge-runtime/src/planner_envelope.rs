//! Planner Contract Firewall
//!
//! Implements strict classification of planner output before it can pollute runtime semantics.
//! Per DELIVERABLE 3: Planner Contract Firewall.

use crate::planner::protocol_validator::{ValidationDecision, ValidationFailureClass};
use crate::types::{PlannerOutput, ToolCall, ToolName};

/// Classification of planner envelope before runtime processing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerEnvelopeDecision {
    /// Valid tool call, proceed to execution
    AcceptedToolCall { tool: ToolName },
    /// Valid completion claim, evaluate completion gate
    AcceptedCompletion { reason: String },
    /// Invalid but potentially recoverable, enter repair loop
    RepairableProtocolViolation {
        reason: String,
        failure_class: ValidationFailureClass,
    },
    /// Invalid and unrecoverable, halt immediately
    TerminalProtocolViolation {
        reason: String,
        failure_class: ValidationFailureClass,
    },
}

/// Explicit terminal failure classes for structured logging
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TerminalFailureClass {
    /// Unparseable JSON output
    InvalidJson,
    /// Valid JSON but violates schema contract
    InvalidSchema,
    /// Tool name not in registry
    UnknownTool,
    /// Required arguments missing
    MissingArguments,
    /// Invalid completion envelope (e.g., completion with pending work)
    InvalidCompletionEnvelope,
    /// Tool call after terminal condition was reached
    ToolAfterTerminal,
    /// Repeated schema drift after repair budget exhausted
    RepairBudgetExhausted,
}

impl TerminalFailureClass {
    /// Check if this failure class is repairable (can enter repair loop)
    pub fn is_repairable(&self) -> bool {
        !matches!(
            self,
            Self::UnknownTool | Self::ToolAfterTerminal | Self::RepairBudgetExhausted
        )
    }

    /// Get human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            Self::InvalidJson => "Planner output is not valid JSON",
            Self::InvalidSchema => "Planner output violates schema contract",
            Self::UnknownTool => "Planner requested unknown tool not in registry",
            Self::MissingArguments => "Tool call missing required arguments",
            Self::InvalidCompletionEnvelope => "Completion claim in invalid state",
            Self::ToolAfterTerminal => "Tool call emitted after terminal condition",
            Self::RepairBudgetExhausted => "Max repair attempts exceeded",
        }
    }
}

impl std::fmt::Display for TerminalFailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::InvalidJson => "invalid_json",
                Self::InvalidSchema => "invalid_schema",
                Self::UnknownTool => "unknown_tool",
                Self::MissingArguments => "missing_arguments",
                Self::InvalidCompletionEnvelope => "invalid_completion_envelope",
                Self::ToolAfterTerminal => "tool_after_terminal",
                Self::RepairBudgetExhausted => "repair_budget_exhausted",
            }
        )
    }
}

/// Planner envelope classifier - deterministic classification of planner output
pub struct PlannerEnvelopeClassifier;

impl PlannerEnvelopeClassifier {
    /// Classify planner output into envelope decision
    pub fn classify(
        output: &PlannerOutput,
        validation_result: &ValidationDecision,
        available_tools: &[String],
        repair_count: u32,
        max_repair: u32,
    ) -> PlannerEnvelopeDecision {
        // Check repair budget first
        if repair_count >= max_repair {
            return PlannerEnvelopeDecision::TerminalProtocolViolation {
                reason: format!("Repair budget exhausted: {} attempts", repair_count),
                failure_class: ValidationFailureClass::SemanticViolation,
            };
        }

        match validation_result {
            ValidationDecision::Accept => {
                // Valid output - determine if tool call or completion
                match output {
                    PlannerOutput::ToolCall(ToolCall { name, .. }) => {
                        // Check if tool is known
                        if available_tools.contains(&name.as_str().to_string()) {
                            PlannerEnvelopeDecision::AcceptedToolCall { tool: name.clone() }
                        } else {
                            PlannerEnvelopeDecision::TerminalProtocolViolation {
                                reason: format!("Unknown tool: {}", name),
                                failure_class: ValidationFailureClass::UnknownTool,
                            }
                        }
                    }
                    PlannerOutput::Completion { reason } => {
                        PlannerEnvelopeDecision::AcceptedCompletion {
                            reason: reason.as_str().to_string(),
                        }
                    }
                    PlannerOutput::Failure {
                        reason,
                        recoverable,
                    } => {
                        if *recoverable {
                            PlannerEnvelopeDecision::RepairableProtocolViolation {
                                reason: reason.clone(),
                                failure_class: ValidationFailureClass::SemanticViolation,
                            }
                        } else {
                            PlannerEnvelopeDecision::TerminalProtocolViolation {
                                reason: reason.clone(),
                                failure_class: ValidationFailureClass::SemanticViolation,
                            }
                        }
                    }
                }
            }
            ValidationDecision::Reject {
                reason,
                failure_class,
                ..
            } => {
                // Check if rejection is repairable
                if Self::is_repairable_failure(failure_class) {
                    PlannerEnvelopeDecision::RepairableProtocolViolation {
                        reason: reason.clone(),
                        failure_class: failure_class.clone(),
                    }
                } else {
                    PlannerEnvelopeDecision::TerminalProtocolViolation {
                        reason: reason.clone(),
                        failure_class: failure_class.clone(),
                    }
                }
            }
            ValidationDecision::Escalate {
                reason,
                failure_class,
                ..
            } => {
                // Escalation is always terminal
                PlannerEnvelopeDecision::TerminalProtocolViolation {
                    reason: reason.clone(),
                    failure_class: failure_class.clone(),
                }
            }
        }
    }

    /// Determine if a failure class can enter repair loop
    fn is_repairable_failure(failure_class: &ValidationFailureClass) -> bool {
        use ValidationFailureClass::*;
        match failure_class {
            // These can potentially be repaired with retry
            InvalidJson => true,
            SchemaViolation => true,
            MissingRequiredField => true,
            ShorthandDetected => true,
            WrapperSchema => true,
            IdleAction => true,
            SemanticViolation => true,
            CompletionWithoutEvidence => true,
            VagueCompletionReason => true,
            CompletionWithPendingValidation => true,
            CompletionWithKnownErrors => true,
            PrematureCompletion => true,

            // These are terminal (no point in retrying)
            UnknownTool => false,
            UnknownField => false,
            MultipleActions => false,
            ShellLexemeDetected => false,
            ModeViolation => false,
            WriteWithoutRead => false,
            StaleRead => false,
            InsufficientReadScope => false,
            _ => false,
        }
    }
}

/// Event types for planner protocol transparency
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PlannerProtocolEvent {
    /// Protocol violation detected
    Violation {
        failure_class: ValidationFailureClass,
        decision: PlannerEnvelopeDecision,
        raw_output_hash: String,
    },
    /// Repair attempt initiated
    RepairAttempt {
        attempt: u32,
        max_attempts: u32,
        repair_prompt: String,
    },
    /// Terminal failure reached
    TerminalFailure {
        failure_class: TerminalFailureClass,
        reason: String,
        final_output_hash: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompletionReason, ToolArguments};

    fn test_tools() -> Vec<String> {
        vec![
            "read_file".to_string(),
            "write_file".to_string(),
            "apply_patch".to_string(),
        ]
    }

    #[test]
    fn classify_valid_tool_call() {
        let output = PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("read_file").unwrap(),
            arguments: ToolArguments::new(),
        });
        let validation = ValidationDecision::Accept;

        let decision =
            PlannerEnvelopeClassifier::classify(&output, &validation, &test_tools(), 0, 3);

        match decision {
            PlannerEnvelopeDecision::AcceptedToolCall { tool } => {
                assert_eq!(tool.as_str(), "read_file");
            }
            _ => panic!("Expected AcceptedToolCall, got {:?}", decision),
        }
    }

    #[test]
    fn classify_unknown_tool_is_terminal() {
        let output = PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("unknown_tool").unwrap(),
            arguments: ToolArguments::new(),
        });
        let validation = ValidationDecision::Accept;

        let decision =
            PlannerEnvelopeClassifier::classify(&output, &validation, &test_tools(), 0, 3);

        match decision {
            PlannerEnvelopeDecision::TerminalProtocolViolation { failure_class, .. } => {
                assert!(matches!(failure_class, ValidationFailureClass::UnknownTool));
            }
            _ => panic!("Expected TerminalProtocolViolation for unknown tool"),
        }
    }

    #[test]
    fn classify_repair_budget_exhausted_is_terminal() {
        let output = PlannerOutput::Completion {
            reason: CompletionReason::new("Done"),
        };
        let validation = ValidationDecision::Accept;

        let decision = PlannerEnvelopeClassifier::classify(
            &output,
            &validation,
            &test_tools(),
            3, // Already at max
            3,
        );

        match decision {
            PlannerEnvelopeDecision::TerminalProtocolViolation { reason, .. } => {
                assert!(reason.contains("Repair budget exhausted"));
            }
            _ => panic!("Expected TerminalProtocolViolation for exhausted budget"),
        }
    }

    #[test]
    fn classify_invalid_json_is_repairable() {
        let output = PlannerOutput::Failure {
            reason: "Invalid JSON".to_string(),
            recoverable: true,
        };
        let validation = ValidationDecision::Reject {
            reason: "Invalid JSON".to_string(),
            tier: 1,
            raw_output: "not json".to_string(),
            failure_class: ValidationFailureClass::InvalidJson,
        };

        let decision =
            PlannerEnvelopeClassifier::classify(&output, &validation, &test_tools(), 0, 3);

        match decision {
            PlannerEnvelopeDecision::RepairableProtocolViolation { failure_class, .. } => {
                assert!(matches!(failure_class, ValidationFailureClass::InvalidJson));
            }
            _ => panic!("Expected RepairableProtocolViolation for invalid JSON"),
        }
    }

    #[test]
    fn terminal_failure_class_descriptions() {
        assert_eq!(
            TerminalFailureClass::InvalidJson.description(),
            "Planner output is not valid JSON"
        );
        assert_eq!(
            TerminalFailureClass::UnknownTool.description(),
            "Planner requested unknown tool not in registry"
        );
    }

    #[test]
    fn terminal_failure_class_repairability() {
        assert!(TerminalFailureClass::InvalidJson.is_repairable());
        assert!(TerminalFailureClass::InvalidSchema.is_repairable());
        assert!(TerminalFailureClass::MissingArguments.is_repairable());
        assert!(!TerminalFailureClass::UnknownTool.is_repairable());
        assert!(!TerminalFailureClass::ToolAfterTerminal.is_repairable());
        assert!(!TerminalFailureClass::RepairBudgetExhausted.is_repairable());
    }
}
