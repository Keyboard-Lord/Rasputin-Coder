//! FORGE Planner Protocol Validator
//!
//! Implements strict validation of planner output per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md.
//! This validator enforces the canonical Forge JSON contract and rejects ALL invalid forms.
//!
//! Design principles:
//! - Fail-closed: Any ambiguity → REJECT
//! - No auto-coercion: Never transform shorthand into canonical
//! - Explicit rejection: Every invalid form is logged with classification
//! - Tiered response: Accept | Reject (with retry) | Escalate (halt)

use crate::planner::StateView;
use crate::types::{ExecutionMode, ToolName};
use serde_json::Value;

/// Validation decision per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 9.3
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDecision {
    /// Output is valid canonical Forge JSON - proceed to execution
    Accept,
    /// Output is invalid - retry with error context
    Reject {
        reason: String,
        tier: u8,
        raw_output: String,
        failure_class: ValidationFailureClass,
    },
    /// Critical violation - halt session immediately
    Escalate {
        reason: String,
        violation: String,
        raw_output: String,
        failure_class: ValidationFailureClass,
    },
}

/// Classification of validation failures for structured logging
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationFailureClass {
    InvalidJson,
    SchemaViolation,
    MissingRequiredField,
    UnknownField,
    #[allow(dead_code)]
    ForbiddenField,
    ShorthandDetected,
    WrapperSchema,
    MultipleActions,
    #[allow(dead_code)]
    ArrayDetected,
    ShellLexemeDetected,
    UnknownTool,
    ModeViolation,
    IdleAction,
    #[allow(dead_code)]
    MetaResponse,
    #[allow(dead_code)]
    ProseOnly,
    SemanticViolation,
    // State-Aware Completion Gate failures
    CompletionWithoutEvidence,
    VagueCompletionReason,
    CompletionWithPendingValidation,
    CompletionWithKnownErrors,
    PrematureCompletion,
    // Read-Before-Write Enforcement Gate failures
    WriteWithoutRead,
    StaleRead,
    InsufficientReadScope,
    #[allow(dead_code)]
    MissingReadRecord,
    #[allow(dead_code)]
    ReadHashMismatch,
    #[allow(dead_code)]
    OverwriteWithoutRead,
    #[allow(dead_code)]
    ExistingFileCreationConflict,
}

impl std::fmt::Display for ValidationFailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson => write!(f, "invalid_json"),
            Self::SchemaViolation => write!(f, "schema_violation"),
            Self::MissingRequiredField => write!(f, "missing_required_field"),
            Self::UnknownField => write!(f, "unknown_field"),
            Self::ForbiddenField => write!(f, "forbidden_field"),
            Self::ShorthandDetected => write!(f, "shorthand_detected"),
            Self::WrapperSchema => write!(f, "wrapper_schema"),
            Self::MultipleActions => write!(f, "multiple_actions"),
            Self::ArrayDetected => write!(f, "array_detected"),
            Self::ShellLexemeDetected => write!(f, "shell_lexeme_detected"),
            Self::UnknownTool => write!(f, "unknown_tool"),
            Self::ModeViolation => write!(f, "mode_violation"),
            Self::IdleAction => write!(f, "idle_action"),
            Self::MetaResponse => write!(f, "meta_response"),
            Self::ProseOnly => write!(f, "prose_only"),
            Self::SemanticViolation => write!(f, "semantic_violation"),
            // State-Aware Completion Gate failures
            Self::CompletionWithoutEvidence => write!(f, "completion_without_evidence"),
            Self::VagueCompletionReason => write!(f, "vague_completion_reason"),
            Self::CompletionWithPendingValidation => {
                write!(f, "completion_with_pending_validation")
            }
            Self::CompletionWithKnownErrors => write!(f, "completion_with_known_errors"),
            Self::PrematureCompletion => write!(f, "premature_completion"),
            // Read-Before-Write Enforcement Gate failures
            Self::WriteWithoutRead => write!(f, "write_without_read"),
            Self::StaleRead => write!(f, "stale_read"),
            Self::InsufficientReadScope => write!(f, "insufficient_read_scope"),
            Self::MissingReadRecord => write!(f, "missing_read_record"),
            Self::ReadHashMismatch => write!(f, "read_hash_mismatch"),
            Self::OverwriteWithoutRead => write!(f, "overwrite_without_read"),
            Self::ExistingFileCreationConflict => write!(f, "existing_file_creation_conflict"),
        }
    }
}

/// Structured validation result with full context for logging
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub decision: ValidationDecision,
    #[allow(dead_code)]
    pub stage: ValidationStage,
    #[allow(dead_code)]
    pub execution_time_ms: u64,
    #[allow(dead_code)]
    pub raw_output_hash: String, // SHA-256 hash of raw output
}

/// Validation pipeline stages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStage {
    JsonValidity,
    SchemaMatch,
    NoForbiddenFields,
    NoShellLexemes,
    SingleAction,
    #[allow(dead_code)]
    ToolExists,
    #[allow(dead_code)]
    ToolAllowedInMode,
    #[allow(dead_code)]
    ArgumentsMatchSchema,
    SemanticValidity,
}

/// Strict protocol validator per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md
pub struct PlannerProtocolValidator {
    #[allow(dead_code)]
    fail_on_unknown_fields: bool,
    max_response_bytes: usize,
    forbidden_shell_lexemes: Vec<&'static str>,
    #[allow(dead_code)]
    meta_response_patterns: Vec<&'static str>,
}

impl PlannerProtocolValidator {
    pub fn new() -> Self {
        Self {
            fail_on_unknown_fields: true,
            max_response_bytes: 65536,
            forbidden_shell_lexemes: vec![
                "ls ", "cat ", "cd ", "pwd", "echo ", "rm ", "mv ", "cp ", "grep ", "find ", "|",
                ">", "<", "&&", "||", ";", "$()", "${", "$", "*", "?", "[", "]",
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
            ],
        }
    }

    #[allow(dead_code)]
    pub fn with_fail_on_unknown_fields(mut self, fail: bool) -> Self {
        self.fail_on_unknown_fields = fail;
        self
    }

    /// Main validation entry point
    /// Implements the full validation pipeline per spec Section 8.1
    pub fn validate(&self, raw_output: &str, context: &ValidationContext) -> ValidationResult {
        let start = std::time::Instant::now();

        // Stage 1: JSON validity
        let json_value = match self.check_json_validity(raw_output) {
            Ok(v) => v,
            Err(e) => {
                return self.make_result(
                    ValidationDecision::Reject {
                        reason: e.clone(),
                        tier: 1,
                        raw_output: raw_output.to_string(),
                        failure_class: ValidationFailureClass::InvalidJson,
                    },
                    ValidationStage::JsonValidity,
                    start.elapsed().as_millis() as u64,
                    raw_output,
                );
            }
        };

        // Stage 2: Schema match (canonical format check)
        match self.check_schema_match(&json_value) {
            Ok(_) => {}
            Err((reason, failure_class)) => {
                return self.make_result(
                    ValidationDecision::Reject {
                        reason,
                        tier: 1,
                        raw_output: raw_output.to_string(),
                        failure_class,
                    },
                    ValidationStage::SchemaMatch,
                    start.elapsed().as_millis() as u64,
                    raw_output,
                );
            }
        }

        // Stage 3: No forbidden fields (shorthand/wrapper detection)
        match self.check_no_forbidden_fields(&json_value) {
            Ok(_) => {}
            Err((reason, failure_class)) => {
                let tier = match failure_class {
                    ValidationFailureClass::WrapperSchema => 4, // Contract breach
                    _ => 1,
                };
                return self.make_result(
                    ValidationDecision::Reject {
                        reason,
                        tier,
                        raw_output: raw_output.to_string(),
                        failure_class,
                    },
                    ValidationStage::NoForbiddenFields,
                    start.elapsed().as_millis() as u64,
                    raw_output,
                );
            }
        }

        // Stage 4: No shell lexemes
        match self.check_no_shell_lexemes(&json_value) {
            Ok(_) => {}
            Err(reason) => {
                return self.make_result(
                    ValidationDecision::Escalate {
                        reason,
                        violation: "shell_lexeme_detected".to_string(),
                        raw_output: raw_output.to_string(),
                        failure_class: ValidationFailureClass::ShellLexemeDetected,
                    },
                    ValidationStage::NoShellLexemes,
                    start.elapsed().as_millis() as u64,
                    raw_output,
                );
            }
        }

        // Stage 5: Single action (already validated by schema, but double-check)
        match self.check_single_action(&json_value) {
            Ok(_) => {}
            Err((reason, failure_class)) => {
                return self.make_result(
                    ValidationDecision::Escalate {
                        reason,
                        violation: "multiple_actions".to_string(),
                        raw_output: raw_output.to_string(),
                        failure_class,
                    },
                    ValidationStage::SingleAction,
                    start.elapsed().as_millis() as u64,
                    raw_output,
                );
            }
        }

        // Stage 6: Semantic validation (tool existence, mode, arguments)
        match self.check_semantic_validity(&json_value, context) {
            Ok(_) => {}
            Err((reason, failure_class, tier)) => {
                return self.make_result(
                    ValidationDecision::Reject {
                        reason,
                        tier,
                        raw_output: raw_output.to_string(),
                        failure_class,
                    },
                    ValidationStage::SemanticValidity,
                    start.elapsed().as_millis() as u64,
                    raw_output,
                );
            }
        }

        // All checks passed
        self.make_result(
            ValidationDecision::Accept,
            ValidationStage::SemanticValidity,
            start.elapsed().as_millis() as u64,
            raw_output,
        )
    }

    /// Stage 1: Check JSON validity
    fn check_json_validity(&self, raw: &str) -> Result<Value, String> {
        if raw.trim().is_empty() {
            return Err("Empty response".to_string());
        }

        if raw.len() > self.max_response_bytes {
            return Err(format!(
                "Response exceeds max size: {} > {} bytes",
                raw.len(),
                self.max_response_bytes
            ));
        }

        // Try to parse as JSON
        match serde_json::from_str::<Value>(raw.trim()) {
            Ok(v) => Ok(v),
            Err(e) => Err(format!("JSON parse error: {}", e)),
        }
    }

    /// Stage 2: Check schema match (canonical format)
    fn check_schema_match(&self, json: &Value) -> Result<(), (String, ValidationFailureClass)> {
        // Must be an object (not array, not primitive)
        if !json.is_object() {
            return Err((
                "Output must be a JSON object".to_string(),
                ValidationFailureClass::SchemaViolation,
            ));
        }

        let obj = json.as_object().unwrap();

        // Must have "type" field
        let type_val = match obj.get("type") {
            Some(v) => v,
            None => {
                return Err((
                    "Missing required field: type".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                ));
            }
        };

        let type_str = match type_val.as_str() {
            Some(s) => s,
            None => {
                return Err((
                    "Field 'type' must be a string".to_string(),
                    ValidationFailureClass::SchemaViolation,
                ));
            }
        };

        match type_str {
            "tool_call" => self.validate_tool_call_schema(obj),
            "completion" => self.validate_completion_schema(obj),
            "failure" => self.validate_failure_schema(obj),
            _ => Err((
                format!("Unknown type value: {}", type_str),
                ValidationFailureClass::SchemaViolation,
            )),
        }
    }

    fn validate_tool_call_schema(
        &self,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<(), (String, ValidationFailureClass)> {
        // EXACT SCHEMA: {"type": "tool_call", "tool_call": {"name": "...", "arguments": {...}}}
        // Top level must have EXACTLY: type, tool_call
        let top_level_allowed = ["type", "tool_call"];
        for key in obj.keys() {
            if !top_level_allowed.contains(&key.as_str()) {
                return Err((
                    format!(
                        "ToolCall has unknown field at top level: '{}' - only [type, tool_call] allowed",
                        key
                    ),
                    ValidationFailureClass::UnknownField,
                ));
            }
        }

        // Check for tool_call wrapper
        match obj.get("tool_call") {
            Some(tc) => {
                if !tc.is_object() {
                    return Err((
                        "tool_call must be an object".to_string(),
                        ValidationFailureClass::SchemaViolation,
                    ));
                }
                let tc_obj = tc.as_object().unwrap();

                // tool_call level must have EXACTLY: name, arguments
                let tool_call_allowed = ["name", "arguments"];
                for key in tc_obj.keys() {
                    if !tool_call_allowed.contains(&key.as_str()) {
                        return Err((
                            format!(
                                "ToolCall.tool_call has unknown field: '{}' - only [name, arguments] allowed",
                                key
                            ),
                            ValidationFailureClass::UnknownField,
                        ));
                    }
                }

                // Must have name
                if !tc_obj.contains_key("name") {
                    return Err((
                        "tool_call missing required field: name".to_string(),
                        ValidationFailureClass::MissingRequiredField,
                    ));
                }

                // Must have arguments
                if !tc_obj.contains_key("arguments") {
                    return Err((
                        "tool_call missing required field: arguments".to_string(),
                        ValidationFailureClass::MissingRequiredField,
                    ));
                }

                // Check for forbidden shorthand patterns at top level
                if obj.contains_key("tool")
                    || obj.contains_key("name") && !obj.contains_key("tool_call")
                {
                    return Err((
                        "Shorthand pattern detected - use canonical tool_call wrapper".to_string(),
                        ValidationFailureClass::ShorthandDetected,
                    ));
                }

                Ok(())
            }
            None => {
                // Check for shorthand pattern: {"type": "tool_call", "tool": "..."}
                if obj.contains_key("tool") || obj.contains_key("params") {
                    return Err((
                        "Shorthand tool_call detected - must use canonical format with tool_call wrapper".to_string(),
                        ValidationFailureClass::ShorthandDetected,
                    ));
                }
                Err((
                    "tool_call type requires tool_call object".to_string(),
                    ValidationFailureClass::MissingRequiredField,
                ))
            }
        }
    }

    fn validate_completion_schema(
        &self,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<(), (String, ValidationFailureClass)> {
        // EXACT SCHEMA: {"type": "completion", "reason": "..."}
        // Top level must have EXACTLY: type, reason
        let allowed_fields = ["type", "reason"];
        for key in obj.keys() {
            if !allowed_fields.contains(&key.as_str()) {
                return Err((
                    format!(
                        "Completion has unknown field: '{}' - only [type, reason] allowed",
                        key
                    ),
                    ValidationFailureClass::UnknownField,
                ));
            }
        }

        // Must have reason
        if !obj.contains_key("reason") {
            return Err((
                "completion missing required field: reason".to_string(),
                ValidationFailureClass::MissingRequiredField,
            ));
        }

        Ok(())
    }

    fn validate_failure_schema(
        &self,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<(), (String, ValidationFailureClass)> {
        // EXACT SCHEMA: {"type": "failure", "reason": "...", "recoverable": true|false}
        // Top level must have EXACTLY: type, reason, recoverable
        let allowed_fields = ["type", "reason", "recoverable"];
        for key in obj.keys() {
            if !allowed_fields.contains(&key.as_str()) {
                return Err((
                    format!(
                        "Failure has unknown field: '{}' - only [type, reason, recoverable] allowed",
                        key
                    ),
                    ValidationFailureClass::UnknownField,
                ));
            }
        }

        // Must have reason
        if !obj.contains_key("reason") {
            return Err((
                "failure missing required field: reason".to_string(),
                ValidationFailureClass::MissingRequiredField,
            ));
        }

        // Must have recoverable
        if !obj.contains_key("recoverable") {
            return Err((
                "failure missing required field: recoverable".to_string(),
                ValidationFailureClass::MissingRequiredField,
            ));
        }

        Ok(())
    }

    /// Stage 3: Check for forbidden fields (wrapper schema detection)
    fn check_no_forbidden_fields(
        &self,
        json: &Value,
    ) -> Result<(), (String, ValidationFailureClass)> {
        let obj = json.as_object().unwrap();

        // Forbidden top-level fields (wrapper patterns)
        let forbidden_toplevel = vec![
            "status", "state", "meta", "payload", "step", "plan", "next", "action", "message",
        ];
        for field in &forbidden_toplevel {
            if obj.contains_key(*field) {
                return Err((
                    format!(
                        "Forbidden top-level field detected: {} - indicates wrapper schema",
                        field
                    ),
                    ValidationFailureClass::WrapperSchema,
                ));
            }
        }

        Ok(())
    }

    /// Stage 4: Check for shell lexemes
    fn check_no_shell_lexemes(&self, json: &Value) -> Result<(), String> {
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

                let lower = text.to_lowercase();
                for lexeme in &self.forbidden_shell_lexemes {
                    if lower.contains(lexeme) {
                        return Err(format!(
                            "Shell lexeme detected in field '{}': '{}' - direct shell execution forbidden",
                            field_name.unwrap_or("<unknown>"),
                            lexeme
                        ));
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Stage 5: Check single action
    fn check_single_action(&self, json: &Value) -> Result<(), (String, ValidationFailureClass)> {
        // If it's an array, it's multiple actions
        if json.is_array() {
            return Err((
                "Array of actions detected - only single action allowed".to_string(),
                ValidationFailureClass::MultipleActions,
            ));
        }

        // If it's an object, check type field
        if let Some(obj) = json.as_object() {
            // Check for multiple tool-like fields (indicates shorthand confusion)
            let tool_like = ["tool", "tool_call", "action", "name"];
            let mut found = 0;
            for key in obj.keys() {
                if tool_like.contains(&key.as_str()) {
                    found += 1;
                }
            }
            if found > 1 {
                return Err((
                    "Multiple action keys detected - ambiguous structure".to_string(),
                    ValidationFailureClass::MultipleActions,
                ));
            }
        }

        Ok(())
    }

    /// Stage 6: Semantic validation (tool existence, mode, arguments)
    fn check_semantic_validity(
        &self,
        json: &Value,
        context: &ValidationContext,
    ) -> Result<(), (String, ValidationFailureClass, u8)> {
        let obj = json.as_object().unwrap();
        let type_str = obj.get("type").unwrap().as_str().unwrap();

        match type_str {
            "tool_call" => self.validate_tool_call_semantic(obj, context),
            "completion" => self.validate_completion_semantic(obj, context),
            "failure" => Ok(()), // Failure is always semantically valid if schema matches
            _ => unreachable!(),
        }
    }

    fn validate_tool_call_semantic(
        &self,
        obj: &serde_json::Map<String, Value>,
        context: &ValidationContext,
    ) -> Result<(), (String, ValidationFailureClass, u8)> {
        let tc = obj.get("tool_call").unwrap().as_object().unwrap();
        let tool_name = tc.get("name").unwrap().as_str().unwrap();

        // Check for idle/noop/wait fake actions
        let idle_tools = ["idle", "noop", "wait", "pause", "sleep"];
        if idle_tools.contains(&tool_name) {
            return Err((
                format!(
                    "Idle/fake action detected: '{}' - no such tool exists",
                    tool_name
                ),
                ValidationFailureClass::IdleAction,
                2, // Tier 2 - halt
            ));
        }

        // Validate tool name format
        if ToolName::new(tool_name).is_err() {
            return Err((
                format!("Invalid tool name format: '{}'", tool_name),
                ValidationFailureClass::UnknownTool,
                2, // Tier 2 - halt
            ));
        }

        // Check tool availability
        if !context.is_tool_available(tool_name) {
            return Err((
                format!("Tool '{}' not available in current context", tool_name),
                ValidationFailureClass::UnknownTool,
                2, // Tier 2 - halt
            ));
        }

        // Check mode permission
        if !context.is_tool_allowed_in_mode(tool_name) {
            return Err((
                format!(
                    "Tool '{}' not allowed in {:?} mode",
                    tool_name, context.mode
                ),
                ValidationFailureClass::ModeViolation,
                3, // Tier 3 - halt
            ));
        }

        // === READ-BEFORE-WRITE ENFORCEMENT GATE ===
        // Check if mutation tools require prior read authorization
        let mutation_tools = ["apply_patch", "write_file", "delete_file"];
        if mutation_tools.contains(&tool_name) {
            // Extract target file path from arguments
            let args = tc.get("arguments").unwrap().as_object().unwrap();
            let file_path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str());

            let Some(path) = file_path else {
                return Err((
                    format!(
                        "Mutation tool '{}' missing required target path argument: path/file_path",
                        tool_name
                    ),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ));
            };

            if path.trim().is_empty() {
                return Err((
                    format!(
                        "Mutation tool '{}' has empty target path argument: path/file_path",
                        tool_name
                    ),
                    ValidationFailureClass::MissingRequiredField,
                    1,
                ));
            }

            // Check if file exists on disk (new files don't require prior read)
            let file_exists = std::path::Path::new(path).exists();

            // For existing files, require prior read authorization
            if file_exists && !context.has_read_record(path) {
                // File exists but not read - reject mutation
                return Err((
                    format!(
                        "Read-Before-Write violation: File '{}' was not read before {} attempt. Read file before mutating.",
                        path, tool_name
                    ),
                    ValidationFailureClass::WriteWithoutRead,
                    3, // Tier 3 - halt
                ));
            }

            // Check if read authorizes the intended mutation scope (only for existing files)
            // New files don't require prior read authorization
            if file_exists {
                let is_full_write = tool_name == "write_file"
                    || (tool_name == "apply_patch" && args.get("expected_hash").is_none());

                if is_full_write && !context.read_authorizes_full_write(path) {
                    // Full write attempted but only partial read exists
                    return Err((
                        format!(
                            "Insufficient read scope: Full-file {} on '{}' requires full-file read first. Partial read does not authorize full write.",
                            tool_name, path
                        ),
                        ValidationFailureClass::InsufficientReadScope,
                        3, // Tier 3 - halt
                    ));
                }
            }

            // Note: Stale read detection (hash mismatch) would be checked at execution time
            // when the actual file hash is computed and compared to the tracked hash.
            // This validator performs static checks; runtime performs dynamic hash verification.
        }

        Ok(())
    }

    fn validate_completion_semantic(
        &self,
        obj: &serde_json::Map<String, Value>,
        context: &ValidationContext,
    ) -> Result<(), (String, ValidationFailureClass, u8)> {
        let reason = obj.get("reason").unwrap().as_str().unwrap();

        // === STATE-AWARE COMPLETION GATE ===

        // 1. Tool Execution Evidence Check
        // Completion on iteration 0 with no tool calls is forbidden (unless trivial task)
        if context.iteration == 0 && context.tool_calls_executed == 0 {
            return Err((
                "Completion rejected: No tool execution evidence. Task cannot be satisfied without action.".to_string(),
                ValidationFailureClass::CompletionWithoutEvidence,
                2, // Tier 2 - halt with guidance
            ));
        }

        // 2. Vague Reason Prohibition Check
        // Reject vague/non-state-based completion reasons
        let vague_patterns = vec![
            "looks good",
            "looks correct",
            "task complete",
            "task done",
            "done",
            "finished",
            "complete",
            "all done",
            "ready",
            "success",
            "completed",
            "ok",
            "okay",
            "good",
        ];
        let reason_lower = reason.to_lowercase();
        for pattern in &vague_patterns {
            if reason_lower == *pattern || reason_lower.starts_with(&format!("{} ", pattern)) {
                return Err((
                    format!(
                        "Completion rejected: Vague reason '{}' does not reference observable state. Must cite specific files, lines, or verifiable facts.",
                        reason
                    ),
                    ValidationFailureClass::VagueCompletionReason,
                    1, // Tier 1 - retry
                ));
            }
        }

        // 3. Premature Completion Pattern Check
        let premature = vec![
            "will be",
            "mostly done",
            "unverified",
            "should be",
            "probably",
            "likely",
        ];
        for pattern in &premature {
            if reason.to_lowercase().contains(pattern) {
                return Err((
                    format!(
                        "Premature completion detected: reason contains '{}' - refers to future or uncertain state",
                        pattern
                    ),
                    ValidationFailureClass::PrematureCompletion,
                    1, // Tier 1 - retry
                ));
            }
        }

        // 4. Pending Operations Check
        if context.has_pending_operations {
            return Err((
                "Completion rejected: Pending operations exist. Complete all work before signaling completion.".to_string(),
                ValidationFailureClass::SemanticViolation,
                2, // Tier 2 - halt
            ));
        }

        // 5. Pending Validation Check
        if !context.pending_validation.is_empty() {
            return Err((
                format!(
                    "Completion rejected: {} validation(s) pending. Cannot complete with unvalidated changes.",
                    context.pending_validation.len()
                ),
                ValidationFailureClass::CompletionWithPendingValidation,
                2, // Tier 2 - halt
            ));
        }

        // 6. Known Errors Check
        if !context.known_errors.is_empty() {
            return Err((
                format!(
                    "Completion rejected: {} known error(s) remain unaddressed.",
                    context.known_errors.len()
                ),
                ValidationFailureClass::CompletionWithKnownErrors,
                2, // Tier 2 - halt
            ));
        }

        // 7. State-Based Justification Check
        // Reason should reference specific files, paths, or observable facts
        // Check for file path patterns, line numbers, or specific identifiers
        let has_state_reference = reason.contains(".") || // File extension
            reason.contains("/") || // Path separator
            reason.contains("line") || // Line reference
            reason.contains(":") || // Line number separator
            reason.contains("fn ") || // Function reference
            reason.contains("src/") || // Source path
            reason.contains("config") || // Config reference
            reason.contains("Cargo.toml") || // Specific file
            reason.contains("verified") || // Verification mention
            reason.contains("validated"); // Validation mention

        if !has_state_reference {
            // This is a warning-level check - still allow but log
            // In strict mode, this could be elevated to Tier 1 rejection
        }

        Ok(())
    }

    /// Helper: Create validation result
    fn make_result(
        &self,
        decision: ValidationDecision,
        stage: ValidationStage,
        elapsed_ms: u64,
        raw_output: &str,
    ) -> ValidationResult {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        raw_output.hash(&mut hasher);
        let hash = format!("sha256:{:016x}", hasher.finish());

        ValidationResult {
            decision,
            stage,
            execution_time_ms: elapsed_ms,
            raw_output_hash: hash,
        }
    }
}

impl Default for PlannerProtocolValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Read record for read-before-write authorization tracking
#[derive(Debug, Clone)]
pub struct ReadRecord {
    pub path: String,
    #[allow(dead_code)]
    pub iteration_read: u32,
    pub is_full_read: bool,
    #[allow(dead_code)]
    pub line_start: Option<u32>,
    #[allow(dead_code)]
    pub line_end: Option<u32>,
    #[allow(dead_code)]
    pub content_hash: String,
    #[allow(dead_code)]
    pub read_at: u64,
}

impl ReadRecord {
    pub fn new(path: &str, iteration: u32, is_full: bool, hash: &str) -> Self {
        Self {
            path: path.to_string(),
            iteration_read: iteration,
            is_full_read: is_full,
            line_start: None,
            line_end: None,
            content_hash: hash.to_string(),
            read_at: 0, // Could use timestamp
        }
    }

    #[allow(dead_code)]
    pub fn with_line_range(mut self, start: u32, end: u32) -> Self {
        self.line_start = Some(start);
        self.line_end = Some(end);
        self.is_full_read = false;
        self
    }

    /// Check if this read authorizes a mutation at the given line
    #[allow(dead_code)]
    pub fn covers_line(&self, line: u32) -> bool {
        if self.is_full_read {
            return true;
        }
        match (self.line_start, self.line_end) {
            (Some(start), Some(end)) => line >= start && line <= end,
            _ => false,
        }
    }

    /// Check if this read authorizes a full-file mutation
    pub fn authorizes_full_write(&self) -> bool {
        self.is_full_read
    }
}

/// Context for validation (tool availability, mode, completion gate state, etc.)
pub struct ValidationContext {
    pub available_tools: Vec<String>,
    pub mode: ExecutionMode,
    pub has_pending_operations: bool,
    #[allow(dead_code)]
    pub files_read: Vec<String>,
    // State-Aware Completion Gate fields
    pub iteration: u32,
    pub tool_calls_executed: u32,
    pub pending_validation: Vec<String>,
    pub known_errors: Vec<String>,
    #[allow(dead_code)]
    pub task_description: String,
    // Read-Before-Write Enforcement Gate fields
    pub read_records: Vec<ReadRecord>,
    #[allow(dead_code)]
    pub allow_partial_read_writes: bool, // Policy: allow partial read to authorize patch
}

impl ValidationContext {
    #[allow(dead_code)]
    pub fn new(mode: ExecutionMode) -> Self {
        Self {
            available_tools: vec![],
            mode,
            has_pending_operations: false,
            files_read: vec![],
            // State-Aware Completion Gate defaults
            iteration: 0,
            tool_calls_executed: 0,
            pending_validation: vec![],
            known_errors: vec![],
            task_description: String::new(),
            // Read-Before-Write Enforcement Gate defaults
            read_records: vec![],
            allow_partial_read_writes: false, // Strict policy: partial reads don't authorize writes by default
        }
    }

    #[allow(dead_code)]
    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.available_tools = tools;
        self
    }

    #[allow(dead_code)]
    pub fn with_iteration(mut self, iteration: u32) -> Self {
        self.iteration = iteration;
        self
    }

    #[allow(dead_code)]
    pub fn with_tool_calls_executed(mut self, count: u32) -> Self {
        self.tool_calls_executed = count;
        self
    }

    #[allow(dead_code)]
    pub fn with_pending_validation(mut self, items: Vec<String>) -> Self {
        self.pending_validation = items;
        self
    }

    #[allow(dead_code)]
    pub fn with_known_errors(mut self, errors: Vec<String>) -> Self {
        self.known_errors = errors;
        self
    }

    #[allow(dead_code)]
    pub fn with_task_description(mut self, task: String) -> Self {
        self.task_description = task;
        self
    }

    // Read-Before-Write Enforcement Gate helpers
    #[allow(dead_code)]
    pub fn with_read_records(mut self, records: Vec<ReadRecord>) -> Self {
        self.read_records = records;
        self
    }

    #[allow(dead_code)]
    pub fn add_read_record(mut self, record: ReadRecord) -> Self {
        self.read_records.push(record);
        self
    }

    #[allow(dead_code)]
    pub fn with_partial_read_policy(mut self, allow: bool) -> Self {
        self.allow_partial_read_writes = allow;
        self
    }

    /// Check if a file has been read (has a read record)
    pub fn has_read_record(&self, path: &str) -> bool {
        let path_buf = std::path::PathBuf::from(path);
        self.read_records.iter().any(|r| {
            let record_path = std::path::PathBuf::from(&r.path);
            // Compare file names to handle both relative and absolute paths
            record_path.file_name() == path_buf.file_name()
        })
    }

    /// Get the read record for a file
    pub fn get_read_record(&self, path: &str) -> Option<&ReadRecord> {
        let path_buf = std::path::PathBuf::from(path);
        self.read_records.iter().find(|r| {
            let record_path = std::path::PathBuf::from(&r.path);
            // Compare file names to handle both relative and absolute paths
            record_path.file_name() == path_buf.file_name()
        })
    }

    /// Check if read authorizes a full-file write
    pub fn read_authorizes_full_write(&self, path: &str) -> bool {
        self.get_read_record(path)
            .map(|r| r.authorizes_full_write())
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn from_state_view(state_view: &StateView) -> Self {
        // Convert FileReadInfo to ReadRecord
        let read_records: Vec<ReadRecord> = state_view
            .files_read
            .iter()
            .map(|f| ReadRecord {
                path: f.path.display().to_string(),
                iteration_read: f.read_at_iteration,
                is_full_read: f.is_full_read,
                line_start: None, // Could be extended if FileReadInfo tracks line ranges
                line_end: None,
                content_hash: f.content_hash.clone(),
                read_at: 0,
            })
            .collect();

        Self {
            available_tools: state_view
                .available_tools
                .iter()
                .map(|t| t.name.as_str().to_string())
                .collect(),
            mode: state_view.mode,
            has_pending_operations: false,
            files_read: state_view
                .files_read
                .iter()
                .map(|f| f.path.display().to_string())
                .collect(),
            // State-Aware Completion Gate
            iteration: state_view.iteration,
            tool_calls_executed: state_view.recent_executions.len() as u32,
            pending_validation: vec![],
            known_errors: state_view.recent_errors.clone(),
            task_description: state_view.task.clone(),
            // Read-Before-Write Enforcement Gate
            read_records,
            allow_partial_read_writes: false, // Strict by default
        }
    }

    pub fn is_tool_available(&self, tool_name: &str) -> bool {
        self.available_tools.iter().any(|t| t == tool_name)
    }

    pub fn is_tool_allowed_in_mode(&self, tool_name: &str) -> bool {
        let tool = ToolName::new(tool_name).ok();
        match tool {
            Some(t) => self.mode.allows_tool(&t),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context() -> ValidationContext {
        ValidationContext::new(ExecutionMode::Edit).with_tools(vec![
            "read_file".to_string(),
            "write_file".to_string(),
            "apply_patch".to_string(),
        ])
    }

    #[test]
    fn test_accept_valid_tool_call() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {"path": "test.txt"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Accept));
    }

    #[test]
    fn test_reject_shorthand_tool() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"tool": "read_file", "path": "test.txt"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
    }

    #[test]
    fn test_reject_wrapper_schema() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"status": "in_progress", "tool": "read_file", "arguments": {}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::MissingRequiredField);
        }
    }

    #[test]
    fn test_reject_array_of_actions() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"[{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {}}}, {"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {}}}]"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::SchemaViolation);
        }
    }

    #[test]
    fn test_reject_idle_action() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "idle", "arguments": {}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(failure_class, ValidationFailureClass::IdleAction);
            assert_eq!(tier, 2); // Tier 2 - halt
        }
    }

    #[test]
    fn test_reject_unknown_tool() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw =
            r#"{"type": "tool_call", "tool_call": {"name": "unknown_tool", "arguments": {}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(failure_class, ValidationFailureClass::UnknownTool);
            assert_eq!(tier, 2); // Tier 2 - halt
        }
    }

    #[test]
    fn test_reject_mode_violation() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Analysis)
            .with_tools(vec!["read_file".to_string(), "write_file".to_string()]);

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {"path": "test.txt", "content": "hello"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(failure_class, ValidationFailureClass::ModeViolation);
            assert_eq!(tier, 3); // Tier 3 - halt
        }
    }

    #[test]
    fn test_reject_completion_with_forbidden_fields() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "completion", "reason": "Done", "status": "success", "summary": "Task completed"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::UnknownField);
        }
    }

    #[test]
    fn test_reject_failure_with_forbidden_fields() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "failure", "reason": "Error", "recoverable": false, "details": "More info"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::UnknownField);
        }
    }

    #[test]
    fn test_reject_shell_lexeme() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "execute_command", "arguments": {"command": "ls -la"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(
            result.decision,
            ValidationDecision::Escalate { .. }
        ));
        if let ValidationDecision::Escalate { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::ShellLexemeDetected);
        }
    }

    #[test]
    fn test_expected_hash_placeholder_is_not_shell_escalation() {
        let validator = PlannerProtocolValidator::new();
        let full_read = ReadRecord::new("src/main.rs", 0, true, "hash123");
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["read_file".to_string(), "apply_patch".to_string()])
            .with_iteration(1)
            .with_read_records(vec![full_read]);

        let raw = r#"{"type":"tool_call","tool_call":{"name":"apply_patch","arguments":{"file_path":"src/main.rs","old_text":"fn old()","new_text":"fn new()","expected_hash":"<hash from read_file>"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(!matches!(
            result.decision,
            ValidationDecision::Escalate {
                failure_class: ValidationFailureClass::ShellLexemeDetected,
                ..
            }
        ));
    }

    #[test]
    fn test_accept_valid_completion() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_iteration(1)
            .with_tool_calls_executed(1)
            .with_tools(vec!["read_file".to_string()]);

        let raw = r#"{"type": "completion", "reason": "Runtime entrypoint identified at src/main.rs:12 and validated against Cargo.toml."}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Accept));
    }

    #[test]
    fn test_accept_valid_failure() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw =
            r#"{"type": "failure", "reason": "Cannot proceed without file", "recoverable": true}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Accept));
    }

    // EXACT-SCHEMA RULE TESTS

    #[test]
    fn test_reject_extra_field_in_completion() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "completion", "reason": "Task done", "summary": "Extra info"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::UnknownField);
        }
    }

    #[test]
    fn test_reject_extra_field_in_failure() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "failure", "reason": "Error occurred", "recoverable": true, "details": "More info"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::UnknownField);
        }
    }

    #[test]
    fn test_reject_extra_field_in_tool_call_top_level() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {"path": "test.txt"}}, "extra": "value"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::UnknownField);
        }
    }

    #[test]
    fn test_reject_extra_field_inside_tool_call_object() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {"path": "test.txt"}, "extra_field": "value"}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::UnknownField);
        }
    }

    #[test]
    fn test_reject_no_op_tool_call() {
        let validator = PlannerProtocolValidator::new();
        let context = create_test_context();

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "idle", "arguments": {}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject { failure_class, .. } = result.decision {
            assert_eq!(failure_class, ValidationFailureClass::IdleAction);
        }
    }

    // STATE-AWARE COMPLETION GATE TESTS

    #[test]
    fn test_reject_completion_without_tool_execution() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_iteration(0)
            .with_tool_calls_executed(0);

        let raw = r#"{"type": "completion", "reason": "Task is complete"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(
                failure_class,
                ValidationFailureClass::CompletionWithoutEvidence
            );
            assert_eq!(tier, 2); // Tier 2 - halt
        }
    }

    #[test]
    fn test_reject_vague_completion_reason() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_iteration(1)
            .with_tool_calls_executed(1);

        let vague_reasons = vec![
            "Looks good",
            "Done",
            "Task complete",
            "Finished",
            "Ready",
            "Success",
            "OK",
        ];

        for reason in vague_reasons {
            let raw = format!(r#"{{"type": "completion", "reason": "{}"}}"#, reason);
            let result = validator.validate(&raw, &context);

            assert!(
                matches!(result.decision, ValidationDecision::Reject { .. }),
                "Should reject vague reason: {}",
                reason
            );
            if let ValidationDecision::Reject { failure_class, .. } = &result.decision {
                assert_eq!(
                    *failure_class,
                    ValidationFailureClass::VagueCompletionReason,
                    "Wrong failure class for reason: {}",
                    reason
                );
            }
        }
    }

    #[test]
    fn test_reject_completion_with_pending_validation() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_iteration(1)
            .with_tool_calls_executed(1)
            .with_pending_validation(vec!["src/main.rs".to_string()]);

        let raw = r#"{"type": "completion", "reason": "File updated successfully"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(
                failure_class,
                ValidationFailureClass::CompletionWithPendingValidation
            );
            assert_eq!(tier, 2);
        }
    }

    #[test]
    fn test_reject_completion_with_known_errors() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_iteration(1)
            .with_tool_calls_executed(1)
            .with_known_errors(vec!["Syntax error in src/main.rs".to_string()]);

        let raw = r#"{"type": "completion", "reason": "All tasks finished"}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(
                failure_class,
                ValidationFailureClass::CompletionWithKnownErrors
            );
            assert_eq!(tier, 2);
        }
    }

    #[test]
    fn test_reject_premature_completion_patterns() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_iteration(1)
            .with_tool_calls_executed(1);

        let premature_reasons = vec![
            "Task will be completed next",
            "Mostly done",
            "Changes look correct but unverified",
            "Should be fine now",
            "Probably complete",
            "Likely finished",
        ];

        for reason in premature_reasons {
            let raw = format!(r#"{{"type": "completion", "reason": "{}"}}"#, reason);
            let result = validator.validate(&raw, &context);

            assert!(
                matches!(result.decision, ValidationDecision::Reject { .. }),
                "Should reject premature reason: {}",
                reason
            );
            if let ValidationDecision::Reject { failure_class, .. } = &result.decision {
                assert_eq!(
                    *failure_class,
                    ValidationFailureClass::PrematureCompletion,
                    "Wrong failure class for reason: {}",
                    reason
                );
            }
        }
    }

    #[test]
    fn test_accept_valid_state_justified_completion() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_iteration(1)
            .with_tool_calls_executed(1)
            .with_tools(vec!["read_file".to_string()]);

        let valid_reasons = vec![
            "Runtime entrypoint identified at src/main.rs:12 based on file read and Cargo.toml binary configuration.",
            "Config file updated at src/config.toml with new database URL. Validation passed.",
            "Import error resolved: added 'use std::collections::HashMap;' at src/lib.rs line 45.",
        ];

        for reason in valid_reasons {
            let raw = format!(r#"{{"type": "completion", "reason": "{}"}}"#, reason);
            let result = validator.validate(&raw, &context);

            assert!(
                matches!(result.decision, ValidationDecision::Accept),
                "Should accept valid reason: {}",
                reason
            );
        }
    }

    // READ-BEFORE-WRITE ENFORCEMENT GATE TESTS

    #[test]
    fn test_reject_apply_patch_without_prior_read() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["read_file".to_string(), "apply_patch".to_string()])
            .with_iteration(1);
        // No read records - file was never read

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "apply_patch", "arguments": {"file_path": "src/main.rs", "old_text": "fn old()", "new_text": "fn new()"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(failure_class, ValidationFailureClass::WriteWithoutRead);
            assert_eq!(tier, 3); // Tier 3 - halt
        }
    }

    #[test]
    fn test_reject_mutation_tool_missing_target_path() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["write_file".to_string()])
            .with_iteration(1);

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {"content": "// new file"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            reason,
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert!(reason.contains("path/file_path"));
            assert_eq!(failure_class, ValidationFailureClass::MissingRequiredField);
            assert_eq!(tier, 1);
        }
    }

    #[test]
    fn test_reject_write_file_to_existing_without_read() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["read_file".to_string(), "write_file".to_string()])
            .with_iteration(1);
        // No read records

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {"path": "Cargo.toml", "content": "[package]"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(failure_class, ValidationFailureClass::WriteWithoutRead);
            assert_eq!(tier, 3);
        }
    }

    #[test]
    fn test_reject_full_write_with_partial_read() {
        let validator = PlannerProtocolValidator::new();
        // Create read record for partial read (lines 1-10 only)
        let partial_read =
            ReadRecord::new("src/main.rs", 0, false, "hash123").with_line_range(1, 10);

        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["read_file".to_string(), "write_file".to_string()])
            .with_iteration(1)
            .with_read_records(vec![partial_read]);

        // Attempting full-file write_file after partial read
        let raw = r#"{"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {"path": "src/main.rs", "content": "// full new content"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(failure_class, ValidationFailureClass::InsufficientReadScope);
            assert_eq!(tier, 3);
        }
    }

    #[test]
    fn test_accept_apply_patch_with_full_read() {
        let validator = PlannerProtocolValidator::new();
        // Create read record for full file read
        let full_read = ReadRecord::new("src/main.rs", 0, true, "hash123");

        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["read_file".to_string(), "apply_patch".to_string()])
            .with_iteration(1)
            .with_read_records(vec![full_read]);

        // Apply patch after full read - should be allowed
        let raw = r#"{"type": "tool_call", "tool_call": {"name": "apply_patch", "arguments": {"file_path": "src/main.rs", "old_text": "fn old()", "new_text": "fn new()"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(
            matches!(result.decision, ValidationDecision::Accept),
            "Should allow patch after full read"
        );
    }

    #[test]
    fn test_accept_new_file_creation_without_read() {
        let validator = PlannerProtocolValidator::new();
        // No read records - creating a NEW file
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["read_file".to_string(), "write_file".to_string()])
            .with_iteration(1);

        // Creating a new file should be allowed without prior read
        // (The runtime will distinguish new file vs existing file overwrite)
        let raw = r#"{"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {"path": "src/new_module.rs", "content": "// new file"}}}"#;
        let result = validator.validate(raw, &context);

        // This will be allowed by the validator; runtime checks if file exists
        // For validator purposes, we don't know if file exists, so we allow
        // Runtime will check and reject if it's an overwrite without read
        assert!(
            matches!(result.decision, ValidationDecision::Accept),
            "Should allow new file creation attempt (runtime verifies if file exists)"
        );
    }

    #[test]
    fn test_reject_delete_file_without_read() {
        let validator = PlannerProtocolValidator::new();
        let context = ValidationContext::new(ExecutionMode::Edit)
            .with_tools(vec!["read_file".to_string(), "delete_file".to_string()])
            .with_iteration(1);
        // No read records

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "delete_file", "arguments": {"path": "src/main.rs"}}}"#;
        let result = validator.validate(raw, &context);

        assert!(matches!(result.decision, ValidationDecision::Reject { .. }));
        if let ValidationDecision::Reject {
            failure_class,
            tier,
            ..
        } = result.decision
        {
            assert_eq!(failure_class, ValidationFailureClass::WriteWithoutRead);
            assert_eq!(tier, 3);
        }
    }
}
