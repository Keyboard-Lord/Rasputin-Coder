//! FORGE Planner Output Adapter - Policy B: Minimal Deterministic Normalization
//!
//! This adapter sits between the raw planner output and the validator.
//! It implements the four-layer output-compliance recovery stack:
//! - Layer A: Canonical Output Adapter (minimal normalization)
//! - Layer B: Validator Rejection/Repair Loop
//! - Layer C: Few-Shot Schema Lock (enforced in prompts)
//! - Layer D: Deterministic Sampling (enforced in backend config)
//!
//! Per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 14.

use crate::planner::protocol_validator::{
    PlannerProtocolValidator, ValidationContext, ValidationDecision, ValidationFailureClass,
};
use crate::types::{
    CompletionReason, ForgeError, PlannerOutput, ToolArguments, ToolCall, ToolName,
};
use serde_json::Value;

/// ============================================================================
/// POLICY B: EXPLICIT NORMALIZATION ALLOWLIST
/// ============================================================================
/// Per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 14.0 and 14.1.3
///
/// ONLY these narrow patterns may be normalized. Everything else MUST reject.
/// This enum exists to make the allowlist explicit and auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum NormalizationAllowlist {
    /// Class 1: Shorthand Completion
    /// Input:  {"action": "complete", "justification": "..."}
    /// Output: {"type": "completion", "reason": "..."}
    ShorthandCompletion,

    /// Class 2: Shorthand Tool Call
    /// Input:  {"action": "read_file", "path": "..."}
    /// Output: {"type": "tool_call", "tool_call": {"name": "...", "arguments": {...}}}
    ShorthandToolCall,

    /// Class 3: Single-Level Wrapper Flatten
    /// Input:  {"status": "ready", "payload": {"type": "tool_call", ...}}
    /// Output: {"type": "tool_call", ...}  (inner canonical form)
    /// NOTE: Only flattens when inner payload is valid canonical JSON
    WrapperFlatten,

    /// Class 4: Direct Tool Type Drift
    /// Input:  {"type": "write_file", "path": "...", "content": "..."}
    /// Output: {"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {...}}}
    DirectToolType,

    /// Class 5: Flattened Tool Call Fields
    /// Input:  {"type": "tool_call", "name": "write_file", "arguments": {...}}
    /// Output: {"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {...}}}
    FlattenedToolCall,

    /// Class 6: Tool Field Alias
    /// Input:  {"tool": "read_file", "arguments": {...}}
    /// Output: {"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {...}}}
    ToolFieldAlias,
}

impl NormalizationAllowlist {
    /// All allowed normalization classes
    #[allow(dead_code)]
    pub const ALL: [NormalizationAllowlist; 6] = [
        NormalizationAllowlist::ShorthandCompletion,
        NormalizationAllowlist::ShorthandToolCall,
        NormalizationAllowlist::WrapperFlatten,
        NormalizationAllowlist::DirectToolType,
        NormalizationAllowlist::FlattenedToolCall,
        NormalizationAllowlist::ToolFieldAlias,
    ];

    /// Get drift pattern identifier for logging
    pub fn drift_pattern(&self) -> &'static str {
        match self {
            Self::ShorthandCompletion => "shorthand_completion",
            Self::ShorthandToolCall => "shorthand_tool_call",
            Self::WrapperFlatten => "wrapper_flatten",
            Self::DirectToolType => "direct_tool_type",
            Self::FlattenedToolCall => "flattened_tool_call",
            Self::ToolFieldAlias => "tool_field_alias",
        }
    }
}

/// Normalization action with audit trail
#[derive(Debug, Clone)]
pub struct NormalizationAction {
    pub drift_pattern: String,
    #[allow(dead_code)]
    pub raw_input: String,
    pub canonical_output: String,
    pub reason: String,
}

/// Result of adapter processing
#[derive(Debug, Clone)]
pub enum AdapterResult {
    Success(PlannerOutput),
    Normalized {
        output: PlannerOutput,
        action: NormalizationAction,
    },
    Reject {
        reason: String,
        tier: u8,
        failure_class: ValidationFailureClass,
        #[allow(dead_code)]
        raw_output: String,
    },
    Escalate {
        reason: String,
        violation: String,
        failure_class: ValidationFailureClass,
        #[allow(dead_code)]
        raw_output: String,
    },
}

/// Policy B: Minimal Deterministic Normalization Adapter
///
/// Only normalizes a small, explicitly allowlisted set of near-miss patterns.
/// Everything else is rejected or escalated.
pub struct CanonicalOutputAdapter {
    validator: PlannerProtocolValidator,
    enable_normalization: bool,
}

impl CanonicalOutputAdapter {
    pub fn new() -> Self {
        Self {
            validator: PlannerProtocolValidator::new(),
            enable_normalization: true, // Policy B enabled
        }
    }

    #[allow(dead_code)]
    pub fn with_normalization_disabled(mut self) -> Self {
        self.enable_normalization = false;
        self
    }

    /// Main entry point: process raw planner output
    ///
    /// Flow:
    /// 1. Try strict validation first (fast path)
    /// 2. If strict fails with allowlisted pattern, attempt normalization
    /// 3. Re-validate normalized output
    /// 4. Return result with full audit trail
    pub fn process(&self, raw_output: &str, context: &ValidationContext) -> AdapterResult {
        // Step 1: Attempt strict validation
        let validation_result = self.validator.validate(raw_output, context);

        match &validation_result.decision {
            ValidationDecision::Accept => {
                // Fast path: already valid canonical JSON
                match self.parse_validated_output(raw_output) {
                    Ok(output) => AdapterResult::Success(output),
                    Err(e) => AdapterResult::Escalate {
                        reason: format!("Parse error after validation: {}", e),
                        violation: "internal_parse_error".to_string(),
                        failure_class: ValidationFailureClass::InvalidJson,
                        raw_output: raw_output.to_string(),
                    },
                }
            }
            ValidationDecision::Reject {
                reason,
                tier,
                failure_class,
                ..
            } => {
                // Step 2: Check if this is a normalizable pattern (Policy B)
                if self.enable_normalization
                    && *tier == 1
                    && let Some(action) =
                        self.attempt_normalization(raw_output, failure_class, context)
                {
                    // Step 3: Re-validate normalized output
                    let normalized_str = &action.canonical_output;
                    let revalidation = self.validator.validate(normalized_str, context);

                    if let ValidationDecision::Accept = revalidation.decision {
                        match self.parse_validated_output(normalized_str) {
                            Ok(output) => {
                                return AdapterResult::Normalized { output, action };
                            }
                            Err(_) => {
                                // Normalization produced unparseable output - reject
                            }
                        }
                    } else if let ValidationDecision::Escalate {
                        reason,
                        violation,
                        failure_class,
                        ..
                    } = revalidation.decision
                    {
                        return AdapterResult::Escalate {
                            reason,
                            violation,
                            failure_class,
                            raw_output: raw_output.to_string(),
                        };
                    } else if let ValidationDecision::Reject {
                        reason,
                        tier,
                        failure_class,
                        ..
                    } = revalidation.decision
                    {
                        return AdapterResult::Reject {
                            reason,
                            tier,
                            failure_class,
                            raw_output: raw_output.to_string(),
                        };
                    }
                }

                // Step 4: Reject (non-normalizable or normalization failed)
                AdapterResult::Reject {
                    reason: reason.clone(),
                    tier: *tier,
                    failure_class: failure_class.clone(),
                    raw_output: raw_output.to_string(),
                }
            }
            ValidationDecision::Escalate {
                reason,
                violation,
                failure_class,
                ..
            } => {
                if self.enable_normalization
                    && matches!(failure_class, ValidationFailureClass::ShellLexemeDetected)
                    && let Some(action) =
                        self.attempt_normalization(raw_output, failure_class, context)
                {
                    let normalized_str = &action.canonical_output;
                    let revalidation = self.validator.validate(normalized_str, context);
                    if let ValidationDecision::Accept = revalidation.decision
                        && let Ok(output) = self.parse_validated_output(normalized_str)
                    {
                        return AdapterResult::Normalized { output, action };
                    }
                }

                AdapterResult::Escalate {
                    reason: reason.clone(),
                    violation: violation.clone(),
                    failure_class: failure_class.clone(),
                    raw_output: raw_output.to_string(),
                }
            }
        }
    }

    /// Attempt Policy B normalization for EXPLICITLY ALLOWLISTED patterns only
    ///
    /// Per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 14.1.3:
    /// ONLY these 3 patterns may be normalized:
    /// - NormalizationAllowlist::ShorthandCompletion
    /// - NormalizationAllowlist::ShorthandToolCall
    /// - NormalizationAllowlist::WrapperFlatten
    ///
    /// Everything else MUST return None (caller will reject).
    fn attempt_normalization(
        &self,
        raw_output: &str,
        _failure_class: &ValidationFailureClass,
        context: &ValidationContext,
    ) -> Option<NormalizationAction> {
        // Parse the raw output
        let json: Value = serde_json::from_str(raw_output).ok()?;
        let obj = json.as_object()?;

        // === CLASS 1: SHORTHAND COMPLETION ===
        // Pattern: {"action": "complete", "justification": "..."}
        // Output: {"type": "completion", "reason": "..."}
        if let (Some(action), Some(justification)) = (
            obj.get("action").and_then(|v| v.as_str()),
            obj.get("justification").and_then(|v| v.as_str()),
        ) {
            // STRICT: Must be exactly "complete" with "justification" field
            if action == "complete" && !justification.is_empty() {
                let canonical = format!(
                    "{{\"type\":\"completion\",\"reason\":\"{}\"}}",
                    escape_json_string(justification)
                );
                return Some(NormalizationAction {
                    drift_pattern: NormalizationAllowlist::ShorthandCompletion
                        .drift_pattern()
                        .to_string(),
                    raw_input: raw_output.to_string(),
                    canonical_output: canonical,
                    reason: format!(
                        "[Policy B Allowlist: {:?}] Mapped 'action:complete' to 'type:completion'",
                        NormalizationAllowlist::ShorthandCompletion
                    ),
                });
            }
        }

        // === CLASS 2: SHORTHAND TOOL CALL ===
        // Pattern: {"action": "read_file", "path": "...", ...}
        // Output: {"type": "tool_call", "tool_call": {"name": "...", "arguments": {...}}}
        if let Some(action) = obj.get("action").and_then(|v| v.as_str()) {
            // STRICT: Must have "action" field AND additional argument fields
            // Do NOT normalize if only "action" is present (ambiguous)
            if obj.len() >= 2 {
                let mut args = serde_json::Map::new();
                for (key, value) in obj.iter() {
                    if key != "action" {
                        args.insert(key.clone(), value.clone());
                    }
                }

                if !args.is_empty() {
                    let args_json = serde_json::Value::Object(args).to_string();
                    let canonical = format!(
                        "{{\"type\":\"tool_call\",\"tool_call\":{{\"name\":\"{}\",\"arguments\":{}}}}}",
                        escape_json_string(action),
                        args_json
                    );
                    return Some(NormalizationAction {
                        drift_pattern: NormalizationAllowlist::ShorthandToolCall
                            .drift_pattern()
                            .to_string(),
                        raw_input: raw_output.to_string(),
                        canonical_output: canonical,
                        reason: format!(
                            "[Policy B Allowlist: {:?}] Mapped shorthand to canonical tool_call",
                            NormalizationAllowlist::ShorthandToolCall
                        ),
                    });
                }
            }
        }

        // === CLASS 4: DIRECT TOOL TYPE DRIFT ===
        // Pattern: {"type": "write_file", "path": "...", "content": "..."}
        // Output: {"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {...}}}
        if let Some(type_value) = obj.get("type").and_then(|v| v.as_str())
            && !matches!(type_value, "tool_call" | "completion" | "failure")
            && context.is_tool_available(type_value)
            && obj.len() >= 2
        {
            let args = collect_argument_fields(obj, &["type"]);

            if args.as_object().is_some_and(|args| !args.is_empty())
                && let Some(repaired_args) = repair_normalized_tool_arguments(type_value, args)
            {
                let canonical = canonical_tool_call_json(type_value, repaired_args);
                return Some(NormalizationAction {
                    drift_pattern: NormalizationAllowlist::DirectToolType
                        .drift_pattern()
                        .to_string(),
                    raw_input: raw_output.to_string(),
                    canonical_output: canonical,
                    reason: normalization_reason(
                        NormalizationAllowlist::DirectToolType,
                        "Mapped direct tool type to canonical tool_call",
                    ),
                });
            }
        }

        // === CLASS 5: FLATTENED TOOL_CALL FIELDS ===
        // Pattern: {"type": "tool_call", "name": "write_file", "arguments": {...}}
        // Output: canonical tool_call wrapper.
        if obj.get("type").and_then(|v| v.as_str()) == Some("tool_call")
            && let Some(name) = obj.get("name").and_then(|v| v.as_str())
            && context.is_tool_available(name)
        {
            let arguments = obj
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| collect_argument_fields(obj, &["type", "name"]));
            if arguments.is_object()
                && let Some(repaired_args) = repair_normalized_tool_arguments(name, arguments)
            {
                return Some(NormalizationAction {
                    drift_pattern: NormalizationAllowlist::FlattenedToolCall
                        .drift_pattern()
                        .to_string(),
                    raw_input: raw_output.to_string(),
                    canonical_output: canonical_tool_call_json(name, repaired_args),
                    reason: normalization_reason(
                        NormalizationAllowlist::FlattenedToolCall,
                        "Wrapped flattened tool_call fields",
                    ),
                });
            }
        }

        // === CLASS 6: TOOL FIELD ALIAS ===
        // Pattern: {"tool": "read_file", "arguments": {"path": "src/lib.rs"}}
        // Output: canonical tool_call wrapper.
        if let Some(name) = obj
            .get("tool")
            .or_else(|| obj.get("tool_name"))
            .and_then(|v| v.as_str())
            && context.is_tool_available(name)
        {
            let arguments = obj
                .get("arguments")
                .or_else(|| obj.get("args"))
                .cloned()
                .unwrap_or_else(|| collect_argument_fields(obj, &["tool", "tool_name"]));
            if arguments.is_object()
                && let Some(repaired_args) = repair_normalized_tool_arguments(name, arguments)
            {
                return Some(NormalizationAction {
                    drift_pattern: NormalizationAllowlist::ToolFieldAlias
                        .drift_pattern()
                        .to_string(),
                    raw_input: raw_output.to_string(),
                    canonical_output: canonical_tool_call_json(name, repaired_args),
                    reason: normalization_reason(
                        NormalizationAllowlist::ToolFieldAlias,
                        "Mapped tool field alias to canonical tool_call",
                    ),
                });
            }
        }

        // === CLASS 3: WRAPPER FLATTEN ===
        // Pattern: {"status": "ready", "payload": {"type": "tool_call", ...}}
        // Output: {"type": "tool_call", ...} (inner canonical form)
        // STRICT RULES:
        // - Exactly 2 top-level fields
        // - One must be "status"
        // - Other field must contain valid canonical JSON with "type" field
        if obj.len() == 2 && obj.contains_key("status") {
            for (key, value) in obj.iter() {
                if key != "status" && value.is_object() {
                    let inner = value.as_object()?;
                    // STRICT: Inner must have "type" field with valid canonical value
                    if let Some(type_val) = inner.get("type").and_then(|v| v.as_str())
                        && matches!(type_val, "tool_call" | "completion" | "failure")
                    {
                        let canonical = value.to_string();
                        return Some(NormalizationAction {
                            drift_pattern: NormalizationAllowlist::WrapperFlatten
                                .drift_pattern()
                                .to_string(),
                            raw_input: raw_output.to_string(),
                            canonical_output: canonical,
                            reason: format!(
                                "[Policy B Allowlist: {:?}] Flattened wrapper to inner canonical JSON",
                                NormalizationAllowlist::WrapperFlatten
                            ),
                        });
                    }
                }
            }
        }

        // === NOT NORMALIZABLE ===
        // Pattern does not match any allowlisted class
        None
    }

    /// Parse validated canonical JSON into PlannerOutput
    fn parse_validated_output(&self, json_str: &str) -> Result<PlannerOutput, ForgeError> {
        let json: Value = serde_json::from_str(json_str)
            .map_err(|e| ForgeError::PlannerNormalizationError(e.to_string()))?;

        let obj = json
            .as_object()
            .ok_or_else(|| ForgeError::PlannerNormalizationError("Not an object".to_string()))?;

        let type_val = obj.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
            ForgeError::PlannerNormalizationError("Missing type field".to_string())
        })?;

        match type_val {
            "tool_call" => {
                let tool_call_obj = obj
                    .get("tool_call")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        ForgeError::PlannerNormalizationError("Missing tool_call".to_string())
                    })?;

                let name = tool_call_obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ForgeError::PlannerNormalizationError("Missing tool_call.name".to_string())
                    })?;

                let arguments = tool_call_obj
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));

                let tool_name = ToolName::new(name)
                    .map_err(|e| ForgeError::PlannerNormalizationError(e.to_string()))?;

                let mut args = ToolArguments::new();
                if let Some(arg_obj) = arguments.as_object() {
                    for (k, v) in arg_obj.iter() {
                        // Use as_str() for string values to avoid including JSON quotes
                        let value_str = v.as_str().unwrap_or(&v.to_string()).to_string();
                        args.set(k, &value_str);
                    }
                }

                Ok(PlannerOutput::ToolCall(ToolCall {
                    name: tool_name,
                    arguments: args,
                }))
            }
            "completion" => {
                let reason = obj.get("reason").and_then(|v| v.as_str()).ok_or_else(|| {
                    ForgeError::PlannerNormalizationError("Missing reason".to_string())
                })?;

                Ok(PlannerOutput::Completion {
                    reason: CompletionReason::new(reason),
                })
            }
            "failure" => {
                let reason = obj.get("reason").and_then(|v| v.as_str()).ok_or_else(|| {
                    ForgeError::PlannerNormalizationError("Missing reason".to_string())
                })?;

                let recoverable = obj
                    .get("recoverable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                Ok(PlannerOutput::Failure {
                    reason: reason.to_string(),
                    recoverable,
                })
            }
            _ => Err(ForgeError::PlannerNormalizationError(format!(
                "Unknown type: {}",
                type_val
            ))),
        }
    }
}

/// Escape a string for safe inclusion in JSON
fn escape_json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string())
        .to_string()
        .trim_matches('"')
        .to_string()
}

fn canonical_tool_call_json(name: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "type": "tool_call",
        "tool_call": {
            "name": name,
            "arguments": arguments
        }
    })
    .to_string()
}

fn collect_argument_fields(
    obj: &serde_json::Map<String, Value>,
    excluded: &[&str],
) -> serde_json::Value {
    let mut args = serde_json::Map::new();
    for (key, value) in obj {
        if !excluded.contains(&key.as_str()) {
            args.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(args)
}

fn normalization_reason(allowlist: NormalizationAllowlist, action: &str) -> String {
    format!(
        "[Policy B Allowlist: {:?}] {}; repaired normalized tool arguments where canonical aliases/wrappers were unambiguous",
        allowlist, action
    )
}

fn repair_normalized_tool_arguments(tool_name: &str, arguments: Value) -> Option<Value> {
    let mut args = flatten_argument_wrappers(arguments)?;

    if matches!(tool_name, "write_file" | "apply_patch" | "delete_file") {
        repair_mutation_path_argument(tool_name, &mut args)?;
    }

    Some(Value::Object(args))
}

fn flatten_argument_wrappers(arguments: Value) -> Option<serde_json::Map<String, Value>> {
    let original = arguments.as_object()?.clone();
    let mut flattened = serde_json::Map::new();

    merge_argument_object(&mut flattened, &original);

    for wrapper in ["arguments", "args", "input", "params", "payload"] {
        if let Some(nested) = original.get(wrapper).and_then(|value| value.as_object()) {
            merge_argument_object(&mut flattened, nested);
        }
    }

    if let Some(tool_call) = original
        .get("tool_call")
        .and_then(|value| value.as_object())
    {
        if let Some(nested) = tool_call
            .get("arguments")
            .and_then(|value| value.as_object())
        {
            merge_argument_object(&mut flattened, nested);
        } else {
            merge_argument_object(&mut flattened, tool_call);
        }
    }

    Some(flattened)
}

fn merge_argument_object(
    target: &mut serde_json::Map<String, Value>,
    source: &serde_json::Map<String, Value>,
) {
    for (key, value) in source {
        if matches!(
            key.as_str(),
            "arguments" | "args" | "input" | "params" | "payload" | "tool_call" | "name"
        ) && value.is_object()
        {
            continue;
        }
        target.entry(key.clone()).or_insert_with(|| value.clone());
    }
}

fn repair_mutation_path_argument(
    tool_name: &str,
    args: &mut serde_json::Map<String, Value>,
) -> Option<()> {
    let candidates = collect_path_candidates(args);
    let target_path = single_grounded_path(&candidates)?;

    match tool_name {
        "write_file" => {
            args.insert("path".to_string(), Value::String(target_path));
        }
        "apply_patch" | "delete_file" => {
            args.insert("file_path".to_string(), Value::String(target_path));
        }
        _ => {}
    }

    Some(())
}

fn collect_path_candidates(args: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut candidates = Vec::new();
    for key in ["path", "file_path", "target_path"] {
        if let Some(path) = args.get(key).and_then(|value| value.as_str())
            && !path.trim().is_empty()
        {
            candidates.push(path.to_string());
        }
    }
    candidates
}

fn single_grounded_path(candidates: &[String]) -> Option<String> {
    let mut unique = Vec::new();
    for candidate in candidates {
        if !unique.iter().any(|known| known == candidate) {
            unique.push(candidate.clone());
        }
    }

    if unique.len() == 1 {
        unique.pop()
    } else {
        None
    }
}

/// Repair loop handler for rejected outputs
pub struct RepairLoopHandler {
    max_retries: u32,
}

impl RepairLoopHandler {
    pub fn new(max_retries: u32) -> Self {
        Self { max_retries }
    }

    /// Generate repair prompt for rejected output
    pub fn generate_repair_prompt(
        &self,
        rejection_reason: &str,
        failure_class: &ValidationFailureClass,
    ) -> String {
        format!(
            r#"INVALID OUTPUT.

You did not return canonical Forge JSON.

Rejection reason: {}
Failure class: {}

Return EXACTLY one JSON object in one of these forms:

{{"type":"tool_call","tool_call":{{"name":"...","arguments":{{...}}}}}}
{{"type":"completion","reason":"..."}}
{{"type":"failure","reason":"...","recoverable":true}}

No additional fields.
No prose.
No wrapper schema.
No shorthand like {{"action":"..."}}.
"#,
            rejection_reason, failure_class
        )
    }

    /// Check if retry is allowed
    pub fn can_retry(&self, retry_count: u32) -> bool {
        retry_count < self.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ExecutionMode;

    fn edit_context() -> ValidationContext {
        ValidationContext::new(ExecutionMode::Edit).with_tools(vec![
            "read_file".to_string(),
            "write_file".to_string(),
            "apply_patch".to_string(),
            "grep_search".to_string(),
        ])
    }

    #[test]
    fn normalizes_direct_tool_type_schema_drift() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"write_file","path":"src/math.rs","content":"pub fn triple(x: i32) -> i32 {\n    x * 3\n}\n"}"#;

        let result = adapter.process(raw, &edit_context());

        match result {
            AdapterResult::Normalized { output, action } => {
                assert_eq!(action.drift_pattern, "direct_tool_type");
                match output {
                    PlannerOutput::ToolCall(tool_call) => {
                        assert_eq!(tool_call.name.as_str(), "write_file");
                        assert_eq!(tool_call.arguments.get("path"), Some("src/math.rs"));
                        assert!(
                            tool_call
                                .arguments
                                .get("content")
                                .is_some_and(|content| content.contains("triple"))
                        );
                    }
                    _ => panic!("expected tool call"),
                }
            }
            other => panic!("expected normalized output, got {:?}", other),
        }
    }

    #[test]
    fn repairs_write_file_file_path_alias() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"write_file","file_path":"src/math.rs","content":"pub fn triple(x: i32) -> i32 { x * 3 }\n"}"#;

        let result = adapter.process(raw, &edit_context());

        match result {
            AdapterResult::Normalized { output, action } => {
                assert_eq!(action.drift_pattern, "direct_tool_type");
                assert!(action.reason.contains("repaired normalized tool arguments"));
                match output {
                    PlannerOutput::ToolCall(tool_call) => {
                        assert_eq!(tool_call.name.as_str(), "write_file");
                        assert_eq!(tool_call.arguments.get("path"), Some("src/math.rs"));
                    }
                    _ => panic!("expected tool call"),
                }
            }
            other => panic!("expected normalized output, got {:?}", other),
        }
    }

    #[test]
    fn repairs_apply_patch_path_alias() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"apply_patch","path":"zz_missing_for_patch.rs","old_text":"pub fn old() {}","new_text":"pub fn new() {}","expected_hash":"sha256:abc"}"#;

        let result = adapter.process(raw, &edit_context());

        match result {
            AdapterResult::Normalized { output, action } => {
                assert_eq!(action.drift_pattern, "direct_tool_type");
                assert!(action.reason.contains("repaired normalized tool arguments"));
                match output {
                    PlannerOutput::ToolCall(tool_call) => {
                        assert_eq!(tool_call.name.as_str(), "apply_patch");
                        assert_eq!(
                            tool_call.arguments.get("file_path"),
                            Some("zz_missing_for_patch.rs")
                        );
                        assert_eq!(tool_call.arguments.get("expected_hash"), Some("sha256:abc"));
                    }
                    _ => panic!("expected tool call"),
                }
            }
            other => panic!("expected normalized output, got {:?}", other),
        }
    }

    #[test]
    fn lifts_nested_path_from_malformed_args_object() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"write_file","arguments":{"path":"src/math.rs","content":"pub fn triple(x: i32) -> i32 { x * 3 }\n"}}"#;

        let result = adapter.process(raw, &edit_context());

        match result {
            AdapterResult::Normalized { output, action } => {
                assert_eq!(action.drift_pattern, "direct_tool_type");
                match output {
                    PlannerOutput::ToolCall(tool_call) => {
                        assert_eq!(tool_call.name.as_str(), "write_file");
                        assert_eq!(tool_call.arguments.get("path"), Some("src/math.rs"));
                        assert!(
                            tool_call
                                .arguments
                                .get("content")
                                .is_some_and(|content| content.contains("triple"))
                        );
                    }
                    _ => panic!("expected tool call"),
                }
            }
            other => panic!("expected normalized output, got {:?}", other),
        }
    }

    #[test]
    fn lifts_path_from_malformed_direct_tool_call_wrapper() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"write_file","tool_call":{"name":"write_file","arguments":{"path":"src/math.rs","content":"pub fn triple(x: i32) -> i32 { x * 3 }\n"}}}"#;

        let result = adapter.process(raw, &edit_context());

        match result {
            AdapterResult::Normalized { output, action } => {
                assert_eq!(action.drift_pattern, "direct_tool_type");
                match output {
                    PlannerOutput::ToolCall(tool_call) => {
                        assert_eq!(tool_call.name.as_str(), "write_file");
                        assert_eq!(tool_call.arguments.get("path"), Some("src/math.rs"));
                        assert!(tool_call.arguments.get("tool_call").is_none());
                    }
                    _ => panic!("expected tool call"),
                }
            }
            other => panic!("expected normalized output, got {:?}", other),
        }
    }

    #[test]
    fn rejects_ambiguous_multi_path_argument_repair() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"write_file","path":"src/a.rs","file_path":"src/b.rs","content":"pub fn a() {}\n"}"#;

        let result = adapter.process(raw, &edit_context());

        assert!(matches!(result, AdapterResult::Reject { .. }));
    }

    #[test]
    fn normalizes_flattened_tool_call_structure() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"tool_call","name":"read_file","arguments":{"path":"src/lib.rs"}}"#;

        let result = adapter.process(raw, &edit_context());

        match result {
            AdapterResult::Normalized { output, action } => {
                assert_eq!(action.drift_pattern, "flattened_tool_call");
                match output {
                    PlannerOutput::ToolCall(tool_call) => {
                        assert_eq!(tool_call.name.as_str(), "read_file");
                        assert_eq!(tool_call.arguments.get("path"), Some("src/lib.rs"));
                    }
                    _ => panic!("expected tool call"),
                }
            }
            other => panic!("expected normalized output, got {:?}", other),
        }
    }

    #[test]
    fn normalizes_tool_field_alias() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"tool":"grep_search","arguments":{"query":"parse_setting","path":"src"}}"#;

        let result = adapter.process(raw, &edit_context());

        match result {
            AdapterResult::Normalized { output, action } => {
                assert_eq!(action.drift_pattern, "tool_field_alias");
                match output {
                    PlannerOutput::ToolCall(tool_call) => {
                        assert_eq!(tool_call.name.as_str(), "grep_search");
                        assert_eq!(tool_call.arguments.get("query"), Some("parse_setting"));
                    }
                    _ => panic!("expected tool call"),
                }
            }
            other => panic!("expected normalized output, got {:?}", other),
        }
    }

    #[test]
    fn rejects_unsafe_shell_lexeme_after_attempted_normalization() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"grep_search","query":"parse_setting","path":"src | cat"}"#;

        let result = adapter.process(raw, &edit_context());

        assert!(matches!(
            result,
            AdapterResult::Reject { .. } | AdapterResult::Escalate { .. }
        ));
        match result {
            AdapterResult::Reject { failure_class, .. }
            | AdapterResult::Escalate { failure_class, .. } => {
                assert_ne!(failure_class, ValidationFailureClass::UnknownTool);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn refuses_ambiguous_direct_type_without_arguments() {
        let adapter = CanonicalOutputAdapter::new();
        let raw = r#"{"type":"write_file"}"#;

        let result = adapter.process(raw, &edit_context());

        assert!(matches!(result, AdapterResult::Reject { .. }));
    }
}
