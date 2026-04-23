//! Planner Output Adapter
//!
//! Normalizes and validates planner model output.
//! Fail-closed schema enforcement with one-action-per-turn guarantee.

use crate::planner::state_view::StateView;
use crate::types::{
    CompletionReason, ForgeError, PlannerOutput, ToolArguments, ToolCall, ToolName,
};
use serde::{Deserialize, Serialize};

/// Errors specific to planner output normalization
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PlannerNormalizationError {
    EmptyResponse,
    JsonExtractionFailed(String),
    MultipleJsonObjects,
    SchemaViolation(String),
    UnknownActionType(String),
    MultipleActionsDetected,
    UnknownTool { tool_name: String },
    MissingRequiredField { field: String },
    InvalidArgumentFormat { field: String, reason: String },
    ProseExtractionFailed,
    NestedStructureDetected,
    ArrayOfActionsDetected,
}

impl std::fmt::Display for PlannerNormalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlannerNormalizationError::EmptyResponse => {
                write!(f, "Planner returned empty response")
            }
            PlannerNormalizationError::JsonExtractionFailed(e) => {
                write!(f, "Failed to extract JSON from response: {}", e)
            }
            PlannerNormalizationError::MultipleJsonObjects => {
                write!(f, "Multiple JSON objects detected - only one allowed")
            }
            PlannerNormalizationError::SchemaViolation(e) => {
                write!(f, "Output schema violation: {}", e)
            }
            PlannerNormalizationError::UnknownActionType(t) => {
                write!(f, "Unknown planner action type: {}", t)
            }
            PlannerNormalizationError::MultipleActionsDetected => {
                write!(f, "Multiple actions detected - exactly one required")
            }
            PlannerNormalizationError::UnknownTool { tool_name } => {
                write!(f, "Tool '{}' not available in current mode", tool_name)
            }
            PlannerNormalizationError::MissingRequiredField { field } => {
                write!(f, "Missing required field: {}", field)
            }
            PlannerNormalizationError::InvalidArgumentFormat { field, reason } => {
                write!(f, "Invalid argument format for {}: {}", field, reason)
            }
            PlannerNormalizationError::ProseExtractionFailed => {
                write!(f, "Failed to extract valid JSON from prose response")
            }
            PlannerNormalizationError::NestedStructureDetected => {
                write!(f, "Nested structures not allowed - flat output required")
            }
            PlannerNormalizationError::ArrayOfActionsDetected => {
                write!(f, "Array of actions not allowed - single action required")
            }
        }
    }
}

impl From<PlannerNormalizationError> for ForgeError {
    fn from(e: PlannerNormalizationError) -> Self {
        ForgeError::PlannerNormalizationError(e.to_string())
    }
}

/// Raw planner response types for deserialization
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
enum RawPlannerOutput {
    #[serde(rename = "tool_call")]
    ToolCall {
        #[serde(rename = "tool_call")]
        tool_call: RawToolCall,
    },
    #[serde(rename = "completion")]
    Completion { reason: String },
    #[serde(rename = "failure")]
    Failure {
        reason: String,
        #[serde(default)]
        recoverable: bool,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct RawToolCall {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

/// Adapter for normalizing planner output
pub struct PlannerAdapter {
    #[allow(dead_code)]
    fail_on_unknown_fields: bool,
    max_response_bytes: usize,
}

impl PlannerAdapter {
    pub fn new() -> Self {
        Self {
            fail_on_unknown_fields: true,
            max_response_bytes: 65536,
        }
    }

    #[allow(dead_code)]
    pub fn with_fail_on_unknown_fields(mut self, fail: bool) -> Self {
        self.fail_on_unknown_fields = fail;
        self
    }

    #[allow(dead_code)]
    pub fn with_max_response_bytes(mut self, max: usize) -> Self {
        self.max_response_bytes = max;
        self
    }

    /// Normalize raw planner response into typed PlannerOutput
    ///
    /// Steps:
    /// 1. Strip markdown fences if present
    /// 2. Extract single JSON object
    /// 3. Validate against schema
    /// 4. Check tool availability
    /// 5. Return typed PlannerOutput
    pub fn normalize(
        &self,
        raw_response: &str,
        state_view: &StateView,
    ) -> Result<PlannerOutput, PlannerNormalizationError> {
        // Check empty
        if raw_response.trim().is_empty() {
            return Err(PlannerNormalizationError::EmptyResponse);
        }

        // Check size limit
        if raw_response.len() > self.max_response_bytes {
            return Err(PlannerNormalizationError::JsonExtractionFailed(format!(
                "Response exceeds max size: {} > {} bytes",
                raw_response.len(),
                self.max_response_bytes
            )));
        }

        // Step 1: Extract JSON from potential prose
        let json_str = self.extract_json(raw_response)?;

        // Step 2: Validate single JSON object (not array)
        let json_value: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| PlannerNormalizationError::JsonExtractionFailed(e.to_string()))?;

        // Reject arrays (multiple actions)
        if json_value.is_array() {
            return Err(PlannerNormalizationError::ArrayOfActionsDetected);
        }

        // Step 3: Deserialize to raw type
        let raw_output: RawPlannerOutput = serde_json::from_value(json_value)
            .map_err(|e| PlannerNormalizationError::SchemaViolation(e.to_string()))?;

        // Step 4: Convert to typed PlannerOutput with validation
        self.convert_to_typed(raw_output, state_view)
    }

    /// Extract JSON from raw response, stripping markdown and prose
    fn extract_json(&self, raw: &str) -> Result<String, PlannerNormalizationError> {
        let trimmed = raw.trim();

        // Check for markdown code fences
        if trimmed.starts_with("```") {
            // Extract content between fences
            let lines: Vec<&str> = trimmed.lines().collect();
            let mut json_lines = vec![];
            let mut in_json = false;

            for line in lines {
                if line.trim().starts_with("```") {
                    if in_json {
                        // End of code block
                        break;
                    } else {
                        // Start of code block - may have language specifier
                        in_json = true;
                        continue;
                    }
                }
                if in_json {
                    json_lines.push(line);
                }
            }

            if !json_lines.is_empty() {
                return Ok(json_lines.join("\n"));
            }
        }

        // Try to find JSON object boundaries
        if let Some(start) = trimmed.find('{')
            && let Some(end) = trimmed.rfind('}')
            && start < end
        {
            let json_candidate = &trimmed[start..=end];
            // Validate it's parseable JSON
            if serde_json::from_str::<serde_json::Value>(json_candidate).is_ok() {
                return Ok(json_candidate.to_string());
            }
        }

        // If no extraction worked, try the whole trimmed string
        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
            return Ok(trimmed.to_string());
        }

        Err(PlannerNormalizationError::ProseExtractionFailed)
    }

    /// Convert raw output to typed PlannerOutput with validation
    fn convert_to_typed(
        &self,
        raw: RawPlannerOutput,
        state_view: &StateView,
    ) -> Result<PlannerOutput, PlannerNormalizationError> {
        match raw {
            RawPlannerOutput::ToolCall { tool_call } => {
                // Validate tool name
                let tool_name = ToolName::new(&tool_call.name).map_err(|_| {
                    PlannerNormalizationError::SchemaViolation(format!(
                        "Invalid tool name: {}",
                        tool_call.name
                    ))
                })?;

                // Check tool availability
                if !state_view.is_tool_available(&tool_name) {
                    return Err(PlannerNormalizationError::UnknownTool {
                        tool_name: tool_call.name,
                    });
                }

                // Convert arguments
                let mut arguments = ToolArguments::new();
                if let serde_json::Value::Object(map) = tool_call.arguments {
                    for (key, value) in map {
                        let value_str = match value {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        };
                        arguments.set(&key, &value_str);
                    }
                }

                Ok(PlannerOutput::ToolCall(ToolCall {
                    name: tool_name,
                    arguments,
                }))
            }
            RawPlannerOutput::Completion { reason } => Ok(PlannerOutput::Completion {
                reason: CompletionReason::new(&reason),
            }),
            RawPlannerOutput::Failure {
                reason,
                recoverable,
            } => Ok(PlannerOutput::Failure {
                reason,
                recoverable,
            }),
        }
    }

    /// Validate that exactly one action is present (fail-closed)
    #[allow(dead_code)]
    pub fn validate_single_action(
        &self,
        outputs: &[PlannerOutput],
    ) -> Result<PlannerOutput, PlannerNormalizationError> {
        if outputs.is_empty() {
            return Err(PlannerNormalizationError::EmptyResponse);
        }
        if outputs.len() > 1 {
            return Err(PlannerNormalizationError::MultipleActionsDetected);
        }
        Ok(outputs[0].clone())
    }
}

impl Default for PlannerAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::state_view::ToolInfo;

    fn create_test_state_view() -> StateView {
        StateView {
            task: "test".to_string(),
            session_id: "test-session".to_string(),
            iteration: 0,
            max_iterations: 10,
            mode: crate::types::ExecutionMode::Edit,
            files_read: vec![],
            files_written: vec![],
            available_tools: vec![
                ToolInfo::new(ToolName::new("read_file").unwrap(), "Read a file"),
                ToolInfo::new(ToolName::new("write_file").unwrap(), "Write a file"),
            ],
            recent_executions: vec![],
            last_validation: None,
            recent_errors: vec![],
            repo_root: std::path::PathBuf::from("."),
            allowed_paths: vec![],
        }
    }

    #[test]
    fn test_normalize_tool_call() {
        let adapter = PlannerAdapter::new();
        let state = create_test_state_view();

        let raw = r#"{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {"path": "test.txt"}}}"#;

        let result = adapter.normalize(raw, &state);
        assert!(result.is_ok());

        match result.unwrap() {
            PlannerOutput::ToolCall(tc) => {
                assert_eq!(tc.name.as_str(), "read_file");
                assert_eq!(tc.arguments.get("path"), Some("test.txt"));
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    #[test]
    fn test_normalize_completion() {
        let adapter = PlannerAdapter::new();
        let state = create_test_state_view();

        let raw = r#"{"type": "completion", "reason": "Task done"}"#;

        let result = adapter.normalize(raw, &state);
        assert!(result.is_ok());

        match result.unwrap() {
            PlannerOutput::Completion { reason } => {
                assert_eq!(reason.as_str(), "Task done");
            }
            _ => panic!("Expected Completion"),
        }
    }

    #[test]
    fn test_normalize_with_markdown_fences() {
        let adapter = PlannerAdapter::new();
        let state = create_test_state_view();

        let raw = r#"Here's my plan:

```json
{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {"path": "test.txt"}}}
```

Hope this works!"#;

        let result = adapter.normalize(raw, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reject_unknown_tool() {
        let adapter = PlannerAdapter::new();
        let state = create_test_state_view();

        let raw =
            r#"{"type": "tool_call", "tool_call": {"name": "unknown_tool", "arguments": {}}}"#;

        let result = adapter.normalize(raw, &state);
        assert!(matches!(
            result,
            Err(PlannerNormalizationError::UnknownTool { .. })
        ));
    }

    #[test]
    fn test_reject_array_of_actions() {
        let adapter = PlannerAdapter::new();
        let state = create_test_state_view();

        let raw = r#"[{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {}}}, {"type": "tool_call", "tool_call": {"name": "write_file", "arguments": {}}}]"#;

        let result = adapter.normalize(raw, &state);
        assert!(matches!(
            result,
            Err(PlannerNormalizationError::ArrayOfActionsDetected)
        ));
    }
}
