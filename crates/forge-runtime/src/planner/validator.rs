//! FORGE Planner Protocol Validator - 13-Rule Enforcement Bundle
//!
//! Implements the complete Planner Output Enforcement Bundle per
//! FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 14.0
//!
//! The 13 Rules:
//! 1. Single-Payload Rule
//! 2. Exact-Schema Rule
//! 3. No Shorthand Rule
//! 4. No Wrapper Rule
//! 5. No Idle Actions Rule
//! 6. No Shell Lexemes Rule
//! 7. State-Aware Completion Rule
//! 8. Tool Authorization Rule
//! 9. Semantic Validity Rule
//! 10. Narration Prohibition Rule
//! 11. No-Op Replacement Rule
//! 12. Read-Before-Write Rule
//! 13. Hash Discipline Rule

use crate::crypto_hash::compute_content_hash;
use crate::planner::state_view::StateView;
use crate::types::{ExecutionMode, ToolName};
use serde_json::Value;
use std::collections::HashMap;

/// Validation decision with full audit context
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDecision {
    Accept,
    Reject {
        reason: String,
        tier: u8,
        failure_class: ValidationFailureClass,
        rule_broken: &'static str,
    },
    Escalate {
        reason: String,
        violation: String,
        failure_class: ValidationFailureClass,
        rule_broken: &'static str,
    },
}

/// Classification of validation failures
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ValidationFailureClass {
    InvalidJson,
    SchemaViolation,
    MissingRequiredField,
    UnknownField,
    ShorthandDetected,
    WrapperSchema,
    MultipleActions,
    ArrayDetected,
    ShellLexemeDetected,
    UnknownTool,
    ModeViolation,
    IdleAction,
    MetaResponse,
    ProseOnly,
    SemanticViolation,
    CompletionWithoutEvidence,
    VagueCompletionReason,
    CompletionWithPendingValidation,
    CompletionWithKnownErrors,
    PrematureCompletion,
    WriteWithoutRead,
    StaleRead,
    InsufficientReadScope,
}

impl std::fmt::Display for ValidationFailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::InvalidJson => "invalid_json",
            Self::SchemaViolation => "schema_violation",
            Self::MissingRequiredField => "missing_required_field",
            Self::UnknownField => "unknown_field",
            Self::ShorthandDetected => "shorthand_detected",
            Self::WrapperSchema => "wrapper_schema",
            Self::MultipleActions => "multiple_actions",
            Self::ArrayDetected => "array_detected",
            Self::ShellLexemeDetected => "shell_lexeme_detected",
            Self::UnknownTool => "unknown_tool",
            Self::ModeViolation => "mode_violation",
            Self::IdleAction => "idle_action",
            Self::MetaResponse => "meta_response",
            Self::ProseOnly => "prose_only",
            Self::SemanticViolation => "semantic_violation",
            Self::CompletionWithoutEvidence => "completion_without_evidence",
            Self::VagueCompletionReason => "vague_completion_reason",
            Self::CompletionWithPendingValidation => "completion_with_pending_validation",
            Self::CompletionWithKnownErrors => "completion_with_known_errors",
            Self::PrematureCompletion => "premature_completion",
            Self::WriteWithoutRead => "write_without_read",
            Self::StaleRead => "stale_read",
            Self::InsufficientReadScope => "insufficient_read_scope",
        };
        write!(f, "{}", s)
    }
}

/// Validation audit log entry
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ValidationAuditLog {
    pub timestamp: u64,
    pub raw_output_hash: String,
    pub decision: ValidationDecision,
    pub rule_checks: Vec<RuleCheckResult>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RuleCheckResult {
    pub rule: &'static str,
    pub passed: bool,
    pub details: Option<String>,
}

/// Context for validation
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ValidationContext {
    pub mode: ExecutionMode,
    pub available_tools: Vec<String>,
    pub iteration: u32,
    pub files_read: Vec<ReadRecord>,
    pub task_description: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ReadRecord {
    pub path: String,
    pub iteration: u32,
    pub is_full_read: bool,
    pub content_hash: String,
}

/// 13-Rule Planner Protocol Validator
#[allow(dead_code)]
pub struct PlannerValidator {
    shell_lexeme_patterns: Vec<&'static str>,
    meta_response_patterns: Vec<&'static str>,
    vague_completion_patterns: Vec<&'static str>,
    max_response_bytes: usize,
    audit_log: Vec<ValidationAuditLog>,
}

impl PlannerValidator {
    pub fn new() -> Self {
        Self {
            shell_lexeme_patterns: vec![
                "ls ", "cat ", "cd ", "pwd", "echo ", "rm ", "mv ", "cp ", "grep ", "find ",
                "chmod ", "chown ", "sudo ", "apt ", "yum ", "|", ">", "<", "&&", "||", ";", "$()",
                "${", "`", "$", "*", "?", "[", "]", "{", "}", "~", "#", "!",
            ],
            meta_response_patterns: vec![
                "acknowledged",
                "understood",
                "ready",
                "prepared",
                "got it",
                "sure",
                "i will",
                "let me",
                "i'll",
                "ok",
                "okay",
                "waiting",
                "standing by",
                "at your service",
            ],
            vague_completion_patterns: vec![
                "done",
                "complete",
                "finished",
                "success",
                "ok",
                "looks good",
                "seems fine",
                "appears correct",
                "should work",
            ],
            max_response_bytes: 65536,
            audit_log: Vec::new(),
        }
    }

    /// Main validation entry - enforces all 13 rules
    pub fn validate(
        &mut self,
        raw_output: &str,
        context: &ValidationContext,
    ) -> ValidationDecision {
        let start_time = std::time::Instant::now();
        let mut rule_checks = Vec::new();

        // Rule 1: Single-Payload Rule (must be valid JSON, not array)
        let json_value = match self.check_rule_1_single_payload(raw_output) {
            Ok(v) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R1-Single-Payload",
                    passed: true,
                    details: None,
                });
                v
            }
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R1-Single-Payload",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 1,
                        failure_class: fc,
                        rule_broken: "R1-Single-Payload",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 2: Exact-Schema Rule
        match self.check_rule_2_exact_schema(&json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R2-Exact-Schema",
                passed: true,
                details: None,
            }),
            Err((reason, fc, tier)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R2-Exact-Schema",
                    passed: false,
                    details: Some(reason.clone()),
                });
                let decision = if tier >= 3 {
                    ValidationDecision::Escalate {
                        reason,
                        violation: "schema_violation".to_string(),
                        failure_class: fc,
                        rule_broken: "R2-Exact-Schema",
                    }
                } else {
                    ValidationDecision::Reject {
                        reason,
                        tier,
                        failure_class: fc,
                        rule_broken: "R2-Exact-Schema",
                    }
                };
                return self.log_and_return(raw_output, decision, rule_checks, start_time);
            }
        };

        // Rule 3: No Shorthand Rule
        match self.check_rule_3_no_shorthand(&json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R3-No-Shorthand",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R3-No-Shorthand",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 1,
                        failure_class: fc,
                        rule_broken: "R3-No-Shorthand",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 4: No Wrapper Rule
        match self.check_rule_4_no_wrapper(&json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R4-No-Wrapper",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R4-No-Wrapper",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Escalate {
                        reason,
                        violation: "wrapper_schema".to_string(),
                        failure_class: fc,
                        rule_broken: "R4-No-Wrapper",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 5: No Idle Actions Rule
        match self.check_rule_5_no_idle_actions(&json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R5-No-Idle-Actions",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R5-No-Idle-Actions",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 2,
                        failure_class: fc,
                        rule_broken: "R5-No-Idle-Actions",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 6: No Shell Lexemes Rule
        match self.check_rule_6_no_shell_lexemes(&json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R6-No-Shell-Lexemes",
                passed: true,
                details: None,
            }),
            Err(reason) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R6-No-Shell-Lexemes",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Escalate {
                        reason,
                        violation: "shell_lexeme".to_string(),
                        failure_class: ValidationFailureClass::ShellLexemeDetected,
                        rule_broken: "R6-No-Shell-Lexemes",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 7: State-Aware Completion Rule
        match self.check_rule_7_state_aware_completion(&json_value, context) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R7-State-Aware-Completion",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R7-State-Aware-Completion",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 2,
                        failure_class: fc,
                        rule_broken: "R7-State-Aware-Completion",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 8: Tool Authorization Rule
        match self.check_rule_8_tool_authorization(&json_value, context) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R8-Tool-Authorization",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R8-Tool-Authorization",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 2,
                        failure_class: fc,
                        rule_broken: "R8-Tool-Authorization",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 9: Semantic Validity Rule
        match self.check_rule_9_semantic_validity(&json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R9-Semantic-Validity",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R9-Semantic-Validity",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 1,
                        failure_class: fc,
                        rule_broken: "R9-Semantic-Validity",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 10: Narration Prohibition Rule
        match self.check_rule_10_narration_prohibition(raw_output, &json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R10-Narration-Prohibition",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R10-Narration-Prohibition",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 1,
                        failure_class: fc,
                        rule_broken: "R10-Narration-Prohibition",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rule 11: No-Op Replacement Rule
        match self.check_rule_11_no_op_replacement(&json_value) {
            Ok(_) => rule_checks.push(RuleCheckResult {
                rule: "R11-No-Op-Replacement",
                passed: true,
                details: None,
            }),
            Err((reason, fc)) => {
                rule_checks.push(RuleCheckResult {
                    rule: "R11-No-Op-Replacement",
                    passed: false,
                    details: Some(reason.clone()),
                });
                return self.log_and_return(
                    raw_output,
                    ValidationDecision::Reject {
                        reason,
                        tier: 2,
                        failure_class: fc,
                        rule_broken: "R11-No-Op-Replacement",
                    },
                    rule_checks,
                    start_time,
                );
            }
        };

        // Rules 12 & 13 require runtime state, checked separately in runtime
        rule_checks.push(RuleCheckResult {
            rule: "R12-Read-Before-Write",
            passed: true,
            details: Some("checked at runtime".to_string()),
        });
        rule_checks.push(RuleCheckResult {
            rule: "R13-Hash-Discipline",
            passed: true,
            details: Some("checked at runtime".to_string()),
        });

        // All checks passed
        self.log_and_return(
            raw_output,
            ValidationDecision::Accept,
            rule_checks,
            start_time,
        )
    }

    /// Rule 1: Single-Payload Rule - Must be single JSON object, not array
    fn check_rule_1_single_payload(
        &self,
        raw: &str,
    ) -> Result<Value, (String, ValidationFailureClass)> {
        let trimmed = raw.trim();

        if trimmed.is_empty() {
            return Err((
                "Empty response".to_string(),
                ValidationFailureClass::InvalidJson,
            ));
        }

        if trimmed.len() > self.max_response_bytes {
            return Err((
                format!("Response too large: {} bytes", trimmed.len()),
                ValidationFailureClass::InvalidJson,
            ));
        }

        // Check for multiple JSON objects (streaming artifacts)
        let first_brace = trimmed.find('{').unwrap_or(0);
        let last_brace = trimmed.rfind('}').unwrap_or(trimmed.len());

        if first_brace >= last_brace {
            return Err((
                "No JSON object found".to_string(),
                ValidationFailureClass::InvalidJson,
            ));
        }

        // Check for trailing content after valid JSON
        let after_json = &trimmed[last_brace + 1..].trim();
        if !after_json.is_empty() && !after_json.starts_with("<!--") {
            return Err((
                "Multiple JSON objects or trailing content detected".to_string(),
                ValidationFailureClass::MultipleActions,
            ));
        }

        match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Object(_)) => Ok(serde_json::from_str(trimmed).unwrap()),
            Ok(Value::Array(_)) => Err((
                "Arrays not allowed - must be single object".to_string(),
                ValidationFailureClass::ArrayDetected,
            )),
            Ok(_) => Err((
                "Must be JSON object, not primitive".to_string(),
                ValidationFailureClass::InvalidJson,
            )),
            Err(e) => Err((
                format!("JSON parse error: {}", e),
                ValidationFailureClass::InvalidJson,
            )),
        }
    }

    /// Rule 2: Exact-Schema Rule - Must match canonical format
    fn check_rule_2_exact_schema(
        &self,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass, u8)> {
        let obj = json.as_object().ok_or((
            "Must be JSON object".to_string(),
            ValidationFailureClass::SchemaViolation,
            3,
        ))?;

        // Must have type field
        let type_val = obj.get("type").ok_or((
            "Missing required field: type".to_string(),
            ValidationFailureClass::MissingRequiredField,
            1,
        ))?;

        let type_str = type_val.as_str().ok_or((
            "type must be a string".to_string(),
            ValidationFailureClass::SchemaViolation,
            1,
        ))?;

        match type_str {
            "tool_call" => {
                // EXACT: {"type": "tool_call", "tool_call": {"name": "...", "arguments": {...}}}
                if obj.len() != 2 {
                    return Err((
                        format!(
                            "tool_call must have exactly 2 fields (type, tool_call), found {}",
                            obj.len()
                        ),
                        ValidationFailureClass::SchemaViolation,
                        3,
                    ));
                }

                let tc = obj.get("tool_call").ok_or((
                    "Missing tool_call field".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ))?;

                let tc_obj = tc.as_object().ok_or((
                    "tool_call must be object".to_string(),
                    ValidationFailureClass::SchemaViolation,
                    2,
                ))?;

                if tc_obj.len() != 2 {
                    return Err((
                        "tool_call must have exactly 2 fields (name, arguments)".to_string(),
                        ValidationFailureClass::SchemaViolation,
                        2,
                    ));
                }

                tc_obj.get("name").ok_or((
                    "Missing name in tool_call".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ))?;
                tc_obj.get("arguments").ok_or((
                    "Missing arguments in tool_call".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ))?;
            }
            "completion" => {
                // EXACT: {"type": "completion", "reason": "..."}
                if obj.len() != 2 {
                    return Err((
                        format!(
                            "completion must have exactly 2 fields (type, reason), found {}",
                            obj.len()
                        ),
                        ValidationFailureClass::SchemaViolation,
                        3,
                    ));
                }
                obj.get("reason").ok_or((
                    "Missing reason field".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ))?;
            }
            "failure" => {
                // EXACT: {"type": "failure", "reason": "...", "recoverable": bool}
                if obj.len() != 3 {
                    return Err((
                        format!(
                            "failure must have exactly 3 fields (type, reason, recoverable), found {}",
                            obj.len()
                        ),
                        ValidationFailureClass::SchemaViolation,
                        3,
                    ));
                }
                obj.get("reason").ok_or((
                    "Missing reason field".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ))?;
                obj.get("recoverable").ok_or((
                    "Missing recoverable field".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ))?;
            }
            _ => {
                return Err((
                    format!("Unknown type: {}", type_str),
                    ValidationFailureClass::SchemaViolation,
                    2,
                ));
            }
        }

        Ok(())
    }

    /// Rule 3: No Shorthand Rule - Detect shorthand patterns
    fn check_rule_3_no_shorthand(
        &self,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        // Shorthand: {"action": "..."} instead of {"type": "..."}
        if obj.contains_key("action") && !obj.contains_key("type") {
            return Err((
                "Shorthand 'action' field detected - use 'type'".to_string(),
                ValidationFailureClass::ShorthandDetected,
            ));
        }

        // Shorthand: {"name": "...", "args": {...}} at top level
        if obj.contains_key("name") && obj.contains_key("args") {
            return Err((
                "Shorthand tool call at top level - use wrapped format".to_string(),
                ValidationFailureClass::ShorthandDetected,
            ));
        }

        // Check tool_call for shorthand
        if let Some(tc) = obj.get("tool_call")
            && let Some(tc_obj) = tc.as_object()
        {
            // Shorthand: "args" instead of "arguments"
            if tc_obj.contains_key("args") {
                return Err((
                    "Shorthand 'args' - use 'arguments'".to_string(),
                    ValidationFailureClass::ShorthandDetected,
                ));
            }
            // Shorthand: missing "arguments" wrapper
            for key in tc_obj.keys() {
                if key != "name" && key != "arguments" {
                    return Err((
                        format!("Unknown field in tool_call: '{}'", key),
                        ValidationFailureClass::UnknownField,
                    ));
                }
            }
        }

        Ok(())
    }

    /// Rule 4: No Wrapper Rule - Detect wrapper schemas
    fn check_rule_4_no_wrapper(
        &self,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        // Wrapper fields that indicate nested structure
        let wrapper_fields = [
            "status", "plan", "payload", "result", "data", "output", "response",
        ];
        for field in &wrapper_fields {
            if obj.contains_key(*field) {
                return Err((
                    format!(
                        "Wrapper field '{}' detected - emit canonical JSON only",
                        field
                    ),
                    ValidationFailureClass::WrapperSchema,
                ));
            }
        }

        Ok(())
    }

    /// Rule 5: No Idle Actions Rule - Reject meta-responses
    fn check_rule_5_no_idle_actions(
        &self,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        // Check for idle meta-responses disguised as tool calls
        if let Some(tc) = obj.get("tool_call")
            && let Some(tc_obj) = tc.as_object()
            && let Some(name) = tc_obj.get("name").and_then(|n| n.as_str())
        {
            let name_lower = name.to_lowercase();
            if name_lower.contains("wait")
                || name_lower.contains("idle")
                || name_lower.contains("standby")
            {
                return Err((
                    "Idle action detected - only emit when task requires action".to_string(),
                    ValidationFailureClass::IdleAction,
                ));
            }
        }

        // Check completion reasons for idle patterns
        if let Some(reason) = obj.get("reason").and_then(|r| r.as_str()) {
            let reason_lower = reason.to_lowercase();
            for pattern in &self.meta_response_patterns {
                if reason_lower.contains(pattern) {
                    return Err((
                        format!("Meta-response '{}' not allowed in reason", pattern),
                        ValidationFailureClass::MetaResponse,
                    ));
                }
            }
        }

        Ok(())
    }

    /// Rule 6: No Shell Lexemes Rule - Critical security check
    fn check_rule_6_no_shell_lexemes(&self, json: &Value) -> Result<(), String> {
        self.check_value_for_shell_lexemes(None, json)
    }

    fn check_value_for_shell_lexemes(
        &self,
        field_name: Option<&str>,
        value: &Value,
    ) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    self.check_value_for_shell_lexemes(Some(key.as_str()), nested)?;
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.check_value_for_shell_lexemes(field_name, item)?;
                }
            }
            Value::String(text) => {
                if matches!(
                    field_name,
                    Some("content" | "old_text" | "new_text" | "reason" | "expected_hash")
                ) {
                    return Ok(());
                }

                for pattern in &self.shell_lexeme_patterns {
                    if text.contains(pattern) {
                        return Err(format!(
                            "Forbidden shell lexeme '{}' detected in field '{}' - shell execution is never allowed",
                            pattern,
                            field_name.unwrap_or("<unknown>")
                        ));
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Rule 7: State-Aware Completion Rule
    fn check_rule_7_state_aware_completion(
        &self,
        json: &Value,
        context: &ValidationContext,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        if obj.get("type").and_then(|t| t.as_str()) == Some("completion") {
            let reason = obj.get("reason").and_then(|r| r.as_str()).unwrap_or("");

            // Reject early completion (iteration 0 with no actions)
            if context.iteration == 0 {
                return Err((
                    "Premature completion on iteration 0 - at least one action required"
                        .to_string(),
                    ValidationFailureClass::PrematureCompletion,
                ));
            }

            // Reject vague reasons
            let reason_lower = reason.to_lowercase();
            for pattern in &self.vague_completion_patterns {
                if reason_lower.trim() == *pattern {
                    return Err((
                        format!(
                            "Vague completion reason '{}' - provide specific evidence",
                            pattern
                        ),
                        ValidationFailureClass::VagueCompletionReason,
                    ));
                }
            }

            // Reason must have minimum specificity
            if reason.len() < 20 {
                return Err((
                    "Completion reason too short - must cite specific files/lines".to_string(),
                    ValidationFailureClass::VagueCompletionReason,
                ));
            }
        }

        Ok(())
    }

    /// Rule 8: Tool Authorization Rule
    fn check_rule_8_tool_authorization(
        &self,
        json: &Value,
        context: &ValidationContext,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        if let Some(tc) = obj.get("tool_call")
            && let Some(tc_obj) = tc.as_object()
        {
            let tool_name = tc_obj.get("name").and_then(|n| n.as_str()).unwrap_or("");

            // Check tool exists
            if !context.available_tools.iter().any(|t| t == tool_name) {
                return Err((
                    format!("Unknown tool '{}' - not in available_tools", tool_name),
                    ValidationFailureClass::UnknownTool,
                ));
            }

            // Analysis mode is read-only apart from search.
            if context.mode == ExecutionMode::Analysis
                && tool_name != "read_file"
                && tool_name != "search"
            {
                return Err((
                    format!("Tool '{}' not allowed in Analysis mode", tool_name),
                    ValidationFailureClass::ModeViolation,
                ));
            }
        }

        Ok(())
    }

    /// Rule 9: Semantic Validity Rule
    fn check_rule_9_semantic_validity(
        &self,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        if let Some(tc) = obj.get("tool_call")
            && let Some(tc_obj) = tc.as_object()
        {
            // Check arguments is an object, not array or primitive
            let args = tc_obj.get("arguments").ok_or((
                "Missing arguments".to_string(),
                ValidationFailureClass::SemanticViolation,
            ))?;

            if !args.is_object() {
                return Err((
                    "arguments must be an object".to_string(),
                    ValidationFailureClass::SemanticViolation,
                ));
            }
        }

        Ok(())
    }

    /// Rule 10: Narration Prohibition Rule
    fn check_rule_10_narration_prohibition(
        &self,
        raw: &str,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let trimmed = raw.trim();

        // Check for prose before JSON
        let first_brace = trimmed.find('{').unwrap_or(0);
        if first_brace > 0 {
            let prefix = &trimmed[..first_brace].trim();
            if !prefix.is_empty() && !prefix.starts_with("```") {
                return Err((
                    "Prose detected before JSON - emit raw JSON only".to_string(),
                    ValidationFailureClass::ProseOnly,
                ));
            }
        }

        // Check for prose after JSON
        let obj = json.as_object().unwrap();
        if obj.is_empty() {
            return Err((
                "Empty JSON object".to_string(),
                ValidationFailureClass::ProseOnly,
            ));
        }

        // Check for markdown fences (should be stripped before validation, but double-check)
        if trimmed.starts_with("```") || trimmed.ends_with("```") {
            return Err((
                "Markdown fences detected - remove ```json wrappers".to_string(),
                ValidationFailureClass::ProseOnly,
            ));
        }

        Ok(())
    }

    /// Rule 11: No-Op Replacement Rule
    fn check_rule_11_no_op_replacement(
        &self,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        if let Some(tc) = obj.get("tool_call")
            && let Some(tc_obj) = tc.as_object()
        {
            let tool_name = tc_obj.get("name").and_then(|n| n.as_str()).unwrap_or("");

            // Check for no-op search
            if tool_name == "search" || tool_name == "list_directory" {
                // These can be no-ops if not needed for task
                return Ok(());
            }

            // Check apply_patch for identical old/new text
            if tool_name == "apply_patch"
                && let Some(args) = tc_obj.get("arguments").and_then(|a| a.as_object())
            {
                let old_text = args.get("old_text").and_then(|o| o.as_str()).unwrap_or("");
                let new_text = args.get("new_text").and_then(|n| n.as_str()).unwrap_or("");
                if old_text == new_text {
                    return Err((
                        "No-op patch: old_text equals new_text".to_string(),
                        ValidationFailureClass::IdleAction,
                    ));
                }
            }
        }

        Ok(())
    }

    /// Log validation and return decision
    fn log_and_return(
        &mut self,
        raw_output: &str,
        decision: ValidationDecision,
        rule_checks: Vec<RuleCheckResult>,
        start_time: std::time::Instant,
    ) -> ValidationDecision {
        let log = ValidationAuditLog {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            raw_output_hash: compute_content_hash(raw_output),
            decision: decision.clone(),
            rule_checks,
        };

        // Emit structured audit log
        let decision_str = match &decision {
            ValidationDecision::Accept => "ACCEPT",
            ValidationDecision::Reject { .. } => "REJECT",
            ValidationDecision::Escalate { .. } => "ESCALATE",
        };

        eprintln!(
            "[VALIDATOR] {} ({}ms, hash: {}...)",
            decision_str,
            start_time.elapsed().as_millis(),
            &log.raw_output_hash[..16]
        );

        self.audit_log.push(log);
        decision
    }

    /// Get audit log for debugging
    #[allow(dead_code)]
    pub fn get_audit_log(&self) -> &[ValidationAuditLog] {
        &self.audit_log
    }

    /// Generate correction prompt for retry
    pub fn generate_correction_prompt(&self, failure: &ValidationDecision) -> String {
        let base_prompt = r#"Your previous output violated the Forge output contract.

CANONICAL SCHEMA (ONLY VALID FORMAT):
{"type": "tool_call", "tool_call": {"name": "TOOL_NAME", "arguments": {"arg1": "value1"}}}

{"type": "completion", "reason": "Specific justification with file/line references (min 20 chars)"}

{"type": "failure", "reason": "Specific issue", "recoverable": true/false}

FORBIDDEN PATTERNS:
- Shorthand: {"action": "..."} or {"name": "...", "args": {...}}
- Wrapper: {"status": "...", "tool": {...}}
- Arrays: [{...}, {...}]
- Prose before/after JSON
- Shell commands: ls, cat, rm, |, >, &&, etc.
- Meta-responses: "Acknowledged", "Ready", "Done"

EMIT EXACTLY ONE CANONICAL JSON OBJECT. NO PROSE. NO MARKDOWN."#;

        match failure {
            ValidationDecision::Reject {
                reason,
                rule_broken,
                ..
            } => {
                format!(
                    "{}\n\nVIOLATION: {}\nRULE BROKEN: {}\n\nRETRY WITH CORRECT FORMAT:",
                    base_prompt, reason, rule_broken
                )
            }
            ValidationDecision::Escalate {
                reason, violation, ..
            } => {
                format!(
                    "{}\n\nCRITICAL VIOLATION: {} ({}). DO NOT REPEAT. USE CANONICAL FORMAT ONLY.",
                    base_prompt, reason, violation
                )
            }
            _ => base_prompt.to_string(),
        }
    }
}

impl Default for PlannerValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> ValidationContext {
        ValidationContext {
            mode: ExecutionMode::Edit,
            available_tools: vec![
                "read_file".to_string(),
                "write_file".to_string(),
                "apply_patch".to_string(),
            ],
            iteration: 1,
            files_read: vec![],
            task_description: "test".to_string(),
        }
    }

    #[test]
    fn test_valid_tool_call() {
        let mut validator = PlannerValidator::new();
        let json = r#"{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {"path": "test.txt"}}}"#;
        let result = validator.validate(json, &test_context());
        assert!(matches!(result, ValidationDecision::Accept));
    }

    #[test]
    fn test_reject_shorthand() {
        let mut validator = PlannerValidator::new();
        let json = r#"{"action": "read_file", "path": "test.txt"}"#;
        let result = validator.validate(json, &test_context());
        assert!(matches!(result, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            rule_broken,
            failure_class,
            ..
        } = result
        {
            assert_eq!(rule_broken, "R2-Exact-Schema");
            assert_eq!(failure_class, ValidationFailureClass::MissingRequiredField);
        }
    }

    #[test]
    fn test_reject_wrapper() {
        let mut validator = PlannerValidator::new();
        let json = r#"{"status": "ready", "type": "tool_call", "tool_call": {"name": "read_file", "arguments": {}}}"#;
        let result = validator.validate(json, &test_context());
        assert!(matches!(result, ValidationDecision::Escalate { .. }));
        if let ValidationDecision::Escalate {
            rule_broken,
            failure_class,
            ..
        } = result
        {
            assert_eq!(rule_broken, "R2-Exact-Schema");
            assert_eq!(failure_class, ValidationFailureClass::SchemaViolation);
        }
    }

    #[test]
    fn test_reject_shell_lexeme() {
        let mut validator = PlannerValidator::new();
        let json = r#"{"type": "tool_call", "tool_call": {"name": "execute", "arguments": {"command": "ls -la"}}}"#;
        let result = validator.validate(json, &test_context());
        assert!(matches!(
            result,
            ValidationDecision::Escalate {
                rule_broken: "R6-No-Shell-Lexemes",
                ..
            }
        ));
    }

    #[test]
    fn test_reject_vague_completion() {
        let mut validator = PlannerValidator::new();
        let json = r#"{"type": "completion", "reason": "Done"}"#;
        let result = validator.validate(json, &test_context());
        assert!(matches!(
            result,
            ValidationDecision::Reject {
                rule_broken: "R7-State-Aware-Completion",
                ..
            }
        ));
    }

    #[test]
    fn test_reject_early_completion() {
        let mut validator = PlannerValidator::new();
        let ctx = ValidationContext {
            iteration: 0,
            ..test_context()
        };
        let json = r#"{"type": "completion", "reason": "Task is complete with all files modified as requested"}"#;
        let result = validator.validate(json, &ctx);
        assert!(matches!(
            result,
            ValidationDecision::Reject {
                rule_broken: "R7-State-Aware-Completion",
                ..
            }
        ));
    }
}
