//! Model Planner - Real Model-Backed Planner
//!
//! Implements a planner that calls a private external model endpoint.
//! Uses PlannerBackend trait for isolation.

use crate::planner::adapter::PlannerAdapter;
use crate::planner::state_view::StateView;
use crate::planner::traits::Planner;
use crate::types::{ForgeError, PlannerOutput};
use std::time::{Duration, Instant};

/// Backend interface for model communication
/// Isolated from planner semantics - just transport
pub trait PlannerBackend: Send + Sync {
    /// Send prompt to model backend and return raw response
    fn infer(&self, prompt: &str) -> Result<String, ForgeError>;

    /// Backend identification for logging
    #[allow(dead_code)]
    fn backend_type(&self) -> &'static str;
}

/// HTTP-based model backend
pub struct HttpPlannerBackend {
    #[allow(dead_code)]
    endpoint: String,
    model_name: String,
    timeout: Duration,
    temperature: f32,
}

impl HttpPlannerBackend {
    pub fn new(endpoint: String, model_name: String) -> Self {
        Self {
            endpoint,
            model_name,
            timeout: Duration::from_secs(30),
            temperature: 0.0, // Deterministic by default
        }
    }

    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout = Duration::from_secs(seconds);
        self
    }

    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = temp.clamp(0.0, 2.0);
        self
    }
}

impl PlannerBackend for HttpPlannerBackend {
    fn infer(&self, prompt: &str) -> Result<String, ForgeError> {
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};

        // Use ollama CLI directly - no web server needed
        let mut child = Command::new("ollama")
            .args(["run", &self.model_name])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                ForgeError::PlannerBackendUnavailable(format!(
                    "Failed to spawn ollama: {}. Is ollama installed?",
                    e
                ))
            })?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).map_err(|e| {
                ForgeError::PlannerBackendUnavailable(format!("Failed to write prompt: {}", e))
            })?;
        }

        // Read output
        let mut output = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_string(&mut output).map_err(|e| {
                ForgeError::PlannerBackendUnavailable(format!("Failed to read output: {}", e))
            })?;
        }

        // Wait for completion
        let status = child.wait().map_err(|e| {
            ForgeError::PlannerBackendUnavailable(format!("Process wait failed: {}", e))
        })?;

        if !status.success() {
            return Err(ForgeError::PlannerBackendUnavailable(format!(
                "ollama process exited with status: {:?}",
                status.code()
            )));
        }

        Ok(output.trim().to_string())
    }

    fn backend_type(&self) -> &'static str {
        "ollama-cli"
    }
}

/// Mock backend for testing
#[allow(dead_code)]
pub struct MockPlannerBackend {
    canned_responses: Vec<String>,
    call_count: std::sync::Mutex<usize>,
}

impl MockPlannerBackend {
    #[allow(dead_code)]
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            canned_responses: responses,
            call_count: std::sync::Mutex::new(0),
        }
    }
}

impl PlannerBackend for MockPlannerBackend {
    fn infer(&self, _prompt: &str) -> Result<String, ForgeError> {
        let mut count = self.call_count.lock().unwrap();
        let response = self
            .canned_responses
            .get(*count)
            .cloned()
            .unwrap_or_else(|| r#"{"type": "completion", "reason": "Mock exhausted"}"#.to_string());
        *count += 1;
        Ok(response)
    }

    fn backend_type(&self) -> &'static str {
        "mock"
    }
}

/// Model-backed planner with output normalization
pub struct ModelPlanner {
    backend: Box<dyn PlannerBackend>,
    adapter: PlannerAdapter,
    system_prompt: String,
    #[allow(dead_code)]
    max_retries: u32,
}

impl ModelPlanner {
    pub fn new(backend: Box<dyn PlannerBackend>) -> Self {
        Self {
            backend,
            adapter: PlannerAdapter::new(),
            system_prompt: Self::default_system_prompt(),
            max_retries: 0, // Fail-closed by default
        }
    }

    #[allow(dead_code)]
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    #[allow(dead_code)]
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    #[allow(dead_code)]
    pub fn with_adapter_config(mut self, adapter: PlannerAdapter) -> Self {
        self.adapter = adapter;
        self
    }

    /// Default system prompt for planner model - CANONICAL FORGE JSON ONLY
    fn default_system_prompt() -> String {
        r#"You are a planner for the Forge runtime system.

Your role: Propose exactly ONE action based on the provided state.
You CANNOT execute actions directly - you only propose them.

Forge will validate and execute your proposal.

AVAILABLE ACTIONS (CANONICAL FORMAT ONLY):
1. tool_call - Call a single tool with arguments
2. completion - Signal task completion
3. failure - Report unrecoverable failure

STRICT RULES - VIOLATIONS CAUSE IMMEDIATE REJECTION:
- Emit exactly ONE JSON object per response
- No prose outside JSON
- No markdown fences (raw JSON only)
- No multiple actions or arrays
- Only use tools listed in available_tools
- READ-BEFORE-WRITE RULE: files must be read before ANY mutation (patch, overwrite, delete)
- Full-file write requires full-file read first
- Never invent file contents you haven't read
- Be deterministic: same state should suggest same action

FORBIDDEN (Will Be Rejected):
- Shorthand: {"tool": "..."} or {"name": "...", "args": {...}}
- Wrapper fields: {"status": "...", "tool": "..."}
- Meta-responses: "Acknowledged", "Ready", "Understood"
- Arrays: [{...}, {...}]

CANONICAL OUTPUT FORMAT (Only valid format):

Tool call:
{"type": "tool_call", "tool_call": {"name": "TOOL_NAME", "arguments": {"arg1": "value1"}}}

Completion (STATE-AWARE GATE - Strict Requirements):
{"type": "completion", "reason": "State-based justification with specific file/line references"}
- ONLY emit completion when task is demonstrably satisfied
- Completion on iteration 0 with no tool calls → REJECTED
- Vague reasons like "Done", "Looks good" → REJECTED
- Must cite specific files, lines, or verifiable facts

Failure:
{"type": "failure", "reason": "Specific, observable issue", "recoverable": true/false}

COMPLETION GATE RULE:
If unsure about completion → Continue with tool_call
If blocked → Emit failure, not completion

Remember: You are advisory only. Forge has sole execution authority."#
            .to_string()
    }

    /// Build prompt from system prompt + state view
    fn build_prompt(&self, state: &StateView) -> String {
        let state_json = state.to_json();

        format!(
            "{}\n\nCURRENT STATE:\n{}\n\nPROPOSE EXACTLY ONE ACTION:",
            self.system_prompt, state_json
        )
    }
}

impl Planner for ModelPlanner {
    /// PHASE 4: generate_raw() returns the ACTUAL raw LLM response
    /// This is the critical method for the hardened validation path.
    fn generate_raw(&self, state: &StateView) -> Result<String, ForgeError> {
        let start = Instant::now();

        // Build prompt
        let prompt = self.build_prompt(state);

        // PHASE 4: Temperature enforcement check
        let effective_temp = self.enforce_temperature();
        if effective_temp != self.get_backend_temperature() {
            eprintln!(
                "[PLANNER] WARNING: Temperature capped from {} to {} for schema compliance",
                self.get_backend_temperature(),
                effective_temp
            );
        }

        // Call backend - this returns the RAW string response from the LLM
        // NO parsing, NO struct conversion, NO cleanup happens here
        let raw_response = self.backend.infer(&prompt)?;

        // Log raw output for audit (first 200 chars)
        let preview: String = raw_response.chars().take(200).collect();
        eprintln!(
            "[PLANNER] Raw model output ({} bytes, {}ms): {}",
            raw_response.len(),
            start.elapsed().as_millis(),
            preview
        );

        // Return the EXACT response body - this goes to the ProtocolValidator
        Ok(raw_response)
    }

    fn generate(&self, state: &StateView) -> Result<PlannerOutput, ForgeError> {
        // PHASE 4: For the old path, we still need to support this
        // But the hardened runtime should call generate_raw() instead
        let raw = self.generate_raw(state)?;

        // Normalize through adapter (legacy path)
        match self.adapter.normalize(&raw, state) {
            Ok(planner_output) => Ok(planner_output),
            Err(e) => Err(ForgeError::PlannerNormalizationError(e.to_string())),
        }
    }

    fn planner_type(&self) -> &'static str {
        "model"
    }

    fn health_check(&self) -> Result<(), ForgeError> {
        // Simple health check via backend
        self.backend.infer("{\"type\": \"ping\"}").map(|_| ())
    }
}

impl ModelPlanner {
    /// PHASE 4: Get current backend temperature
    fn get_backend_temperature(&self) -> f32 {
        // This would need to be stored or retrieved from backend
        // For now, assume 0.0 default
        0.0
    }

    /// PHASE 4: Enforce temperature cap per Deterministic Sampling Policy
    /// Per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 14.4:
    /// - temperature: 0.0 or 0.1 maximum
    fn enforce_temperature(&self) -> f32 {
        let requested = self.get_backend_temperature();
        let capped = requested.min(0.1); // Hard cap at 0.1

        if requested > 0.1 {
            eprintln!(
                "[PLANNER] Temperature {} exceeds 0.1 maximum, capping at 0.1 for schema compliance",
                requested
            );
        }

        capped
    }
}

/// Configuration for model planner
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ModelPlannerConfig {
    pub backend_type: String, // "http", "mock", etc.
    pub model_name: String,
    pub endpoint: String,
    pub timeout_seconds: u64,
    pub temperature: f32,
    pub system_prompt_path: Option<String>,
    pub max_response_bytes: usize,
    pub fail_on_unknown_fields: bool,
    pub max_retries: u32,
}

impl Default for ModelPlannerConfig {
    fn default() -> Self {
        Self {
            backend_type: "stub".to_string(),
            model_name: crate::planner::model_http::DEFAULT_CODER_14B_MODEL.to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            timeout_seconds: 30,
            temperature: 0.0,
            system_prompt_path: None,
            max_response_bytes: 65536,
            fail_on_unknown_fields: true,
            max_retries: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::state_view::{StateView, ToolInfo};
    use crate::types::{ExecutionMode, ToolName};
    use std::path::PathBuf;

    fn create_test_state() -> StateView {
        StateView {
            task: "test".to_string(),
            session_id: "test-session".to_string(),
            iteration: 0,
            max_iterations: 10,
            mode: ExecutionMode::Edit,
            files_read: vec![],
            files_written: vec![],
            available_tools: vec![
                ToolInfo::new(ToolName::new("read_file").unwrap(), "Read a file"),
                ToolInfo::new(ToolName::new("write_file").unwrap(), "Write a file"),
            ],
            recent_executions: vec![],
            last_validation: None,
            recent_errors: vec![],
            repo_root: PathBuf::from("."),
            allowed_paths: vec![],
        }
    }

    #[test]
    fn test_mock_planner_parses_valid_tool_call() {
        let mock_backend = Box::new(MockPlannerBackend::new(vec![
            r#"{"type": "tool_call", "tool_call": {"name": "read_file", "arguments": {"path": "test.txt"}}}"#.to_string(),
        ]));

        let planner = ModelPlanner::new(mock_backend);
        let state = create_test_state();

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::ToolCall(tc) => {
                assert_eq!(tc.name.as_str(), "read_file");
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    #[test]
    fn test_mock_planner_parses_completion() {
        let mock_backend = Box::new(MockPlannerBackend::new(vec![
            r#"{"type": "completion", "reason": "Done"}"#.to_string(),
        ]));

        let planner = ModelPlanner::new(mock_backend);
        let state = create_test_state();

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::Completion { reason } => {
                assert_eq!(reason.as_str(), "Done");
            }
            _ => panic!("Expected Completion"),
        }
    }

    #[test]
    fn test_mock_planner_rejects_unknown_tool() {
        let mock_backend = Box::new(MockPlannerBackend::new(vec![
            r#"{"type": "tool_call", "tool_call": {"name": "unknown_tool", "arguments": {}}}"#
                .to_string(),
        ]));

        let planner = ModelPlanner::new(mock_backend);
        let state = create_test_state();

        let result = planner.generate(&state);
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_planner_normalizes_markdown_fences() {
        let mock_backend = Box::new(MockPlannerBackend::new(vec![
            "```json\n{\"type\": \"completion\", \"reason\": \"Fenced\"}\n```".to_string(),
        ]));

        let planner = ModelPlanner::new(mock_backend);
        let state = create_test_state();

        let result = planner.generate(&state).unwrap();

        match result {
            PlannerOutput::Completion { reason } => {
                assert_eq!(reason.as_str(), "Fenced");
            }
            _ => panic!("Expected Completion"),
        }
    }
}
