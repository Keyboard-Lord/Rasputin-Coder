//! FORGE PHASE 1.5: Hardened Runtime Loop
//!
//! Implements deterministic execution loop per FORGE_RUNTIME_IMPLEMENTATION_SPEC.md
//!
//! Key improvements:
//! - Uses strongly-typed contracts throughout
//! - Formal error handling with ForgeError
//! - Structured logging
//! - State integrity verification
//!
//! Loop stages:
//! 1. Preflight - State integrity checks
//! 2. Planner invocation - generate() -> PlannerOutput
//! 3. Tool call parsing - Extract ToolCall
//! 4. Tool validation - Check mode compatibility
//! 5. Tool execution - Execute with context
//! 6. Mutation detection - Record mutations
//! 7. Validation execution - Run validators
//! 8. State commit - Update state with hash
//! 9. Completion check - Check for completion signal

use crate::execution::{
    ValidationEngine, validation_engine::ValidationStage as EngineValidationStage,
};
use crate::governance::{
    AuditLogEntry, GovernanceEngine, GovernanceRuntimeSnapshot, init_governance,
};
use crate::planner::model_http::{
    DEFAULT_CODER_14B_MODEL, FALLBACK_PLANNER_MODEL, normalize_requested_model,
    preferred_model_candidates, should_enable_css_compression,
};
use crate::planner::state_view::{StateView, ToolExecutionRecord, ToolInfo};
use crate::planner::traits::BoxedPlanner;
use crate::planner::{
    AdapterResult, CanonicalOutputAdapter, HttpModelPlanner, HttpOllamaBackend, HttpPlannerBackend,
    IntelligentStubPlanner, ModelPlanner, RepairLoopHandler, StubPlanner, ValidationContext,
    ValidationFailureClass,
};
use crate::runtime_gates::{
    CompletionGate, CompletionGateResult, CompletionReadiness, ReadBeforeWriteGate,
    ReadBeforeWriteResult, normalize_path,
};
use crate::state::AgentState;
use crate::tool_registry::ToolExecutor;
use crate::types::{
    ChainExecutionResult, CompletionReason, ExecutionContext, ExecutionMode, ForgeError,
    JsonlLogEntry, LogSeverity, Mutation, MutationType, PlannerOutput, SessionStatus, ToolCall,
    ToolName, ToolResult,
};
use serde::Deserialize;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_TRACKED_READ_LINES: usize = 120;
const READ_ONLY_CHURN_THRESHOLD: u32 = 3;
const READ_ONLY_CHURN_HALT_THRESHOLD: u32 = 5;
const SMALL_FILE_FALLBACK_MAX_BYTES: usize = 16 * 1024;
const MAX_LIVE_VALIDATION_RECOVERY_ATTEMPTS: u32 = 2;

/// ===========================================================================
/// RUNTIME CONFIGURATION
/// ===========================================================================
///
/// Runtime configuration
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub max_iterations: u32,
    pub task: String,
    pub auto_revert: bool,
    pub mode: ExecutionMode,
    // PHASE 3: Planner configuration
    pub planner_type: String, // "stub", "intelligent", "model", or "http"
    pub planner_endpoint: String, // HTTP endpoint for model planner
    pub planner_model: String, // Model identifier
    pub planner_timeout_seconds: u64,
    pub planner_temperature: f32, // Clamped to 0.0-0.1 for determinism
    pub planner_seed: u64,        // Deterministic seed (default: 42)
    pub css_compression: bool,    // Enable CSS compression for 8B models
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            task: "Create a hello.txt file with 'hello world' content".to_string(),
            auto_revert: true,
            mode: ExecutionMode::Edit,
            // Default to the stronger local coder model when available.
            planner_type: "http".to_string(),
            planner_endpoint: "http://127.0.0.1:11434".to_string(),
            planner_model: DEFAULT_CODER_14B_MODEL.to_string(),
            planner_timeout_seconds: 30,
            planner_temperature: 0.0, // Strictly enforced
            planner_seed: 42,         // Deterministic
            css_compression: false,   // Auto-enabled for 14B+ models
        }
    }
}

fn registered_tool_infos() -> Vec<ToolInfo> {
    vec![
        ToolInfo::new(
            ToolName::new("write_file").unwrap(),
            "Write content to a file",
        )
        .with_required_args(vec!["path", "content"]),
        ToolInfo::new(ToolName::new("read_file").unwrap(), "Read file content")
            .with_required_args(vec!["path"])
            .with_optional_args(vec!["offset", "limit"]),
        ToolInfo::new(
            ToolName::new("apply_patch").unwrap(),
            "Apply a patch to a file",
        )
        .with_required_args(vec!["file_path", "old_text", "new_text", "expected_hash"]),
        ToolInfo::new(
            ToolName::new("list_dir").unwrap(),
            "List directory contents",
        )
        .with_required_args(vec!["path"])
        .with_optional_args(vec!["recursive", "file_type", "include_hidden"]),
        ToolInfo::new(
            ToolName::new("grep_search").unwrap(),
            "Search files with regex pattern",
        )
        .with_required_args(vec!["query", "path"])
        .with_optional_args(vec!["file_pattern"]),
        ToolInfo::new(
            ToolName::new("dependency_graph").unwrap(),
            "Build a bounded dependency graph for a source file",
        )
        .with_required_args(vec!["path"])
        .with_optional_args(vec!["max_depth"]),
        ToolInfo::new(
            ToolName::new("symbol_index").unwrap(),
            "Find symbol-like locations for a query",
        )
        .with_required_args(vec!["query"])
        .with_optional_args(vec!["path", "max_results"]),
        ToolInfo::new(
            ToolName::new("entrypoint_detector").unwrap(),
            "Detect project entry points",
        )
        .with_optional_args(vec!["path"]),
        ToolInfo::new(
            ToolName::new("lint_runner").unwrap(),
            "Run the configured lint policy",
        )
        .with_optional_args(vec!["path"]),
        ToolInfo::new(
            ToolName::new("test_runner").unwrap(),
            "Run the configured test policy",
        )
        .with_optional_args(vec!["path"]),
    ]
}

fn registered_tool_names() -> Vec<String> {
    registered_tool_infos()
        .into_iter()
        .map(|tool| tool.name.as_str().to_string())
        .collect()
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagModel {
    name: String,
}

#[derive(Debug, Clone)]
struct ResolvedPlannerModel {
    selected: String,
    note: String,
    css_compression: bool,
}

fn list_installed_ollama_models(endpoint: &str) -> Result<Vec<String>, ForgeError> {
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));
    let output = std::process::Command::new("curl")
        .args(["-s", "--connect-timeout", "5", "--max-time", "10", &url])
        .output()
        .map_err(|e| {
            ForgeError::PlannerBackendUnavailable(format!("Failed to query Ollama tags: {}", e))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForgeError::PlannerBackendUnavailable(format!(
            "Failed to query Ollama tags from {}: {}",
            endpoint,
            stderr.trim()
        )));
    }

    let response: OllamaTagsResponse = serde_json::from_slice(&output.stdout).map_err(|e| {
        ForgeError::PlannerNormalizationError(format!("Invalid Ollama tags response: {}", e))
    })?;

    Ok(response
        .models
        .into_iter()
        .map(|model| model.name)
        .collect())
}

fn match_installed_model(candidate: &str, installed_models: &[String]) -> Option<String> {
    let candidate_lower = candidate.to_lowercase();
    installed_models
        .iter()
        .find(|installed| {
            let installed_lower = installed.to_lowercase();
            installed_lower == candidate_lower
                || (!candidate_lower.contains(':')
                    && installed_lower.starts_with(&format!("{}:", candidate_lower)))
        })
        .cloned()
}

fn resolve_planner_model(endpoint: &str, requested: &str) -> ResolvedPlannerModel {
    let normalized = normalize_requested_model(requested);
    let mut selected = normalized.clone();
    let mut note = format!(
        "Planner request '{}' normalized to '{}'.",
        requested, normalized
    );

    match list_installed_ollama_models(endpoint) {
        Ok(installed_models) => {
            for candidate in preferred_model_candidates(&normalized) {
                if let Some(installed) = match_installed_model(&candidate, &installed_models) {
                    let fallback_used =
                        installed == FALLBACK_PLANNER_MODEL && installed != normalized;
                    note = if installed == normalized {
                        format!("Planner model '{}' verified locally.", installed)
                    } else if fallback_used {
                        format!(
                            "Planner model '{}' not found locally; falling back to '{}'.",
                            normalized, installed
                        )
                    } else {
                        format!(
                            "Planner model '{}' resolved to installed '{}'.",
                            normalized, installed
                        )
                    };
                    selected = installed;
                    break;
                }
            }

            if selected == normalized
                && !installed_models
                    .iter()
                    .any(|installed| installed == &selected)
            {
                if installed_models
                    .iter()
                    .any(|installed| installed == FALLBACK_PLANNER_MODEL)
                {
                    selected = FALLBACK_PLANNER_MODEL.to_string();
                    note = format!(
                        "Planner model '{}' was unavailable; falling back to '{}'.",
                        normalized, selected
                    );
                } else {
                    note = format!(
                        "Planner model '{}' was requested, but local Ollama only has: {}",
                        normalized,
                        installed_models.join(", ")
                    );
                }
            }
        }
        Err(error) => {
            note = format!(
                "Could not inspect installed Ollama models ({}); using '{}' directly.",
                error, normalized
            );
        }
    }

    ResolvedPlannerModel {
        css_compression: should_enable_css_compression(&selected),
        selected,
        note,
    }
}

/// ===========================================================================
/// RUNTIME RESULT
/// ===========================================================================
///
/// Runtime execution result
#[derive(Debug, Clone)]
pub struct RuntimeResult {
    pub success: bool,
    pub final_state: AgentState,
    pub iterations: u32,
    #[allow(dead_code)]
    pub logs: Vec<RuntimeLogEntry>,
    #[allow(dead_code)]
    pub jsonl_logs: Vec<JsonlLogEntry>,
    pub error: Option<String>, // Changed from ForgeError to String for Clone
}

/// JSONL logger for machine-readable structured logging
pub struct JsonlLogger {
    entries: Vec<JsonlLogEntry>,
}

impl JsonlLogger {
    pub fn new() -> Self {
        Self { entries: vec![] }
    }

    pub fn log(&mut self, entry: JsonlLogEntry) {
        println!("{}", entry.to_jsonl()); // Emit to console
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &Vec<JsonlLogEntry> {
        &self.entries
    }

    #[allow(dead_code)]
    pub fn write_to_file(&self, path: &str) -> Result<(), ForgeError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| ForgeError::IoError(format!("Failed to open log file: {}", e)))?;

        for entry in &self.entries {
            writeln!(file, "{}", entry.to_jsonl())
                .map_err(|e| ForgeError::IoError(format!("Failed to write log: {}", e)))?;
        }

        Ok(())
    }
}

/// ===========================================================================
/// STRUCTURED LOGGING
/// ===========================================================================
///
/// Log entry for runtime events (human-readable)
#[derive(Debug, Clone)]
pub struct RuntimeLogEntry {
    #[allow(dead_code)]
    pub stage: String,
    #[allow(dead_code)]
    pub iteration: u32,
    #[allow(dead_code)]
    pub message: String,
    #[allow(dead_code)]
    pub timestamp: u64,
    #[allow(dead_code)]
    pub severity: crate::types::LogSeverity,
}

/// ===========================================================================
/// RUNTIME CONTROLLER
/// ===========================================================================
///
/// Runtime controller with hardened enforcement
pub struct Runtime {
    state: AgentState,
    config: RuntimeConfig,
    executor: ToolExecutor,
    planner: BoxedPlanner,
    logs: Vec<RuntimeLogEntry>,
    jsonl_logger: JsonlLogger,
    // PHASE 4: Hardened enforcement components
    output_adapter: CanonicalOutputAdapter,
    repair_handler: RepairLoopHandler,
    pending_validation: bool,
    pending_validation_items: Vec<String>,
    known_errors: Vec<String>,
    governance: GovernanceEngine,
    tool_calls_executed: u32,
    recent_tool_executions: Vec<ToolExecutionRecord>,
    consecutive_validation_failures: u32,
    consecutive_read_only_tools: u32,
    recent_read_only_signatures: VecDeque<String>,
    no_progress_corrections: u32,
    deferred_validation_mutations: Vec<Mutation>,
    consecutive_failed_tool_calls: u32,
    recent_failed_tool_signatures: VecDeque<String>,
    failed_tool_corrections: u32,
    post_correction_progress_required: bool,
    post_correction_read_only_tools: u32,
    // PHASE 4: Repair loop state
    last_rejection: Option<(String, ValidationFailureClass)>, // (reason, failure_class)
    retry_count: u32,
    repair_prompt: Option<String>,
    // TASK CHAIN: Optional multi-step execution
    chain_executor: Option<crate::chain_executor::ChainExecutor>,
}

impl Runtime {
    /// Create a new runtime instance
    pub fn new(config: RuntimeConfig) -> Result<Self, ForgeError> {
        let mut config = config;
        let mut state = AgentState::new(config.max_iterations, config.task.clone(), config.mode);
        let session_id = state.session_id.clone();
        let resolved_model = resolve_planner_model(&config.planner_endpoint, &config.planner_model);
        eprintln!("[RUNTIME] {}", resolved_model.note);

        config.planner_model = resolved_model.selected.clone();
        if resolved_model.css_compression && !config.css_compression {
            eprintln!(
                "[RUNTIME] CSS compression auto-enabled for model '{}'.",
                config.planner_model
            );
        }
        config.css_compression = config.css_compression || resolved_model.css_compression;

        // PHASE 3: Create planner based on configuration
        let planner: BoxedPlanner = match config.planner_type.as_str() {
            "stub" => {
                eprintln!("[RUNTIME] Using StubPlanner (deterministic)");
                Box::new(StubPlanner::new())
            }
            "intelligent" => {
                eprintln!("[RUNTIME] Using IntelligentStubPlanner (Phase 2)");
                Box::new(IntelligentStubPlanner::new())
            }
            "model" => {
                eprintln!(
                    "[RUNTIME] Using ModelPlanner with backend: {}",
                    config.planner_endpoint
                );
                let backend = Box::new(
                    HttpPlannerBackend::new(
                        config.planner_endpoint.clone(),
                        config.planner_model.clone(),
                    )
                    .with_timeout(config.planner_timeout_seconds)
                    .with_temperature(config.planner_temperature),
                );
                Box::new(ModelPlanner::new(backend))
            }
            "http" => {
                eprintln!(
                    "[RUNTIME] Using HttpModelPlanner with Ollama API at {}",
                    config.planner_endpoint
                );
                eprintln!(
                    "[RUNTIME] Model: {}, Temperature: {} (enforced), Seed: {} (deterministic)",
                    config.planner_model, config.planner_temperature, config.planner_seed
                );
                let backend = HttpOllamaBackend::new(
                    config.planner_endpoint.clone(),
                    config.planner_model.clone(),
                )
                .with_timeout(config.planner_timeout_seconds)
                .with_temperature(config.planner_temperature)
                .with_seed(config.planner_seed);

                // Enable CSS compression automatically for larger coding models.
                let mut planner = HttpModelPlanner::with_backend(backend);
                if config.css_compression {
                    eprintln!(
                        "[RUNTIME] CSS compression ENABLED for planner model {}",
                        config.planner_model
                    );
                    planner = planner.with_css_compression();
                }
                Box::new(planner)
            }
            _ => {
                return Err(ForgeError::InvalidConfiguration(format!(
                    "Unknown planner type: {}",
                    config.planner_type
                )));
            }
        };

        // Transition from Initializing to Running
        state.start()?;
        let governance_snapshot = GovernanceRuntimeSnapshot::new(
            config.planner_type.clone(),
            config.planner_temperature,
            config.planner_seed,
        );
        let governance = init_governance(&governance_snapshot, &session_id.to_string());

        let mut runtime = Self {
            state,
            config,
            executor: ToolExecutor::new(),
            planner,
            logs: Vec::new(),
            jsonl_logger: JsonlLogger::new(),
            // PHASE 4: Hardened enforcement components
            output_adapter: CanonicalOutputAdapter::new(),
            repair_handler: RepairLoopHandler::new(3), // Max 3 retries per spec
            pending_validation: false,
            pending_validation_items: Vec::new(),
            known_errors: Vec::new(),
            governance,
            tool_calls_executed: 0,
            recent_tool_executions: Vec::new(),
            consecutive_validation_failures: 0,
            consecutive_read_only_tools: 0,
            recent_read_only_signatures: VecDeque::new(),
            no_progress_corrections: 0,
            deferred_validation_mutations: Vec::new(),
            consecutive_failed_tool_calls: 0,
            recent_failed_tool_signatures: VecDeque::new(),
            failed_tool_corrections: 0,
            post_correction_progress_required: false,
            post_correction_read_only_tools: 0,
            // PHASE 4: Repair loop state
            last_rejection: None,
            retry_count: 0,
            repair_prompt: None,
            // TASK CHAIN: Optional multi-step execution
            chain_executor: None,
        };

        runtime.log(
            LogSeverity::Info,
            "INIT",
            "RUNTIME_INIT",
            &format!(
                "Runtime initialized with {} planner (session: {})",
                runtime.planner.planner_type(),
                session_id
            ),
        );

        // PHASE 4: Log hardened enforcement mode
        runtime.log(LogSeverity::Info, "INIT", "ENFORCEMENT_MODE", "Hardened enforcement enabled: ProtocolValidator chokepoint + CompletionGate + ReadBeforeWriteGate");

        Ok(runtime)
    }

    /// Build StateView from current runtime state
    fn build_state_view(&mut self) -> StateView {
        let mut recent_executions: Vec<ToolExecutionRecord> = self
            .recent_tool_executions
            .iter()
            .rev()
            .take(5)
            .cloned()
            .collect();
        recent_executions.reverse();

        let mut state_view =
            StateView::from_agent_state(&self.state, registered_tool_infos(), recent_executions);

        // PHASE 5: Automatic Context Assembly on first iteration
        let task_with_context = if self.state.iteration == 0 {
            match crate::context_assembly::auto_assemble_context(
                &self.config.task,
                std::path::Path::new("."),
            ) {
                Ok(result) => {
                    if result.was_assembled {
                        self.log_paths_event(
                            LogSeverity::Info,
                            "CONTEXT",
                            "CONTEXT_ASSEMBLED",
                            &result.context_summary,
                            &result.files_read,
                        );
                        result.enriched_task
                    } else {
                        self.config.task.clone()
                    }
                }
                Err(e) => {
                    self.log(
                        LogSeverity::Warning,
                        "CONTEXT",
                        "CONTEXT_ASSEMBLY_FAILED",
                        &format!("Failed to assemble context: {}", e),
                    );
                    self.config.task.clone()
                }
            }
        } else {
            self.config.task.clone()
        };

        state_view.task = task_with_context;
        state_view.mode = self.config.mode;
        let mut merged_errors = self.known_errors.clone();
        for error in &state_view.recent_errors {
            if !merged_errors.iter().any(|known| known == error) {
                merged_errors.push(error.clone());
            }
        }
        state_view.recent_errors = merged_errors.into_iter().rev().take(5).collect();
        state_view.recent_errors.reverse();

        if let Some(repair_prompt) = &self.repair_prompt {
            state_view.task = format!(
                "{}\n\nRepair instruction: {}",
                state_view.task, repair_prompt
            );
        }

        state_view
    }

    fn build_protocol_validation_context(&self, state_view: &StateView) -> ValidationContext {
        let read_records: Vec<crate::planner::ReadRecord> = self
            .state
            .files_read
            .values()
            .map(|file| {
                crate::planner::ReadRecord::new(
                    &file.path.to_string_lossy(),
                    file.read_at_iteration,
                    file.is_full_read,
                    &file.content_hash,
                )
            })
            .collect();

        let mut context = ValidationContext::from_state_view(state_view);
        context.mode = self.config.mode;
        context.available_tools = registered_tool_names();
        context.has_pending_operations = !self.pending_validation_items.is_empty();
        context.files_read = self
            .state
            .files_read
            .keys()
            .map(|path| path.display().to_string())
            .collect();
        context.iteration = self.state.iteration;
        context.tool_calls_executed = self.tool_calls_executed;
        context.pending_validation = self.pending_validation_items.clone();
        context.known_errors = self.known_errors.clone();
        context.task_description = self.config.task.clone();
        context.read_records = read_records;
        context.allow_partial_read_writes = false;
        context
    }

    /// Log a runtime event (dual: human-readable + JSONL)
    fn log(&mut self, severity: LogSeverity, stage: &str, event_type: &str, message: &str) {
        let entry = JsonlLogEntry::new(
            event_type,
            &self.state.session_id,
            self.state.iteration,
            &severity.to_string(),
            message,
        );
        self.log_structured(severity, stage, message, entry);
    }

    fn log_structured(
        &mut self,
        severity: LogSeverity,
        stage: &str,
        message: &str,
        jsonl_entry: JsonlLogEntry,
    ) {
        let timestamp = timestamp_now();

        // Human-readable log entry
        let runtime_entry = RuntimeLogEntry {
            stage: stage.to_string(),
            iteration: self.state.iteration,
            message: message.to_string(),
            timestamp,
            severity,
        };

        let prefix = match severity {
            LogSeverity::Debug => "[DBG]",
            LogSeverity::Info => "[INF]",
            LogSeverity::Warning => "[WRN]",
            LogSeverity::Error => "[ERR]",
        };

        if !structured_output_only() {
            println!("{} [{}] {}", prefix, stage, message);
        }
        self.logs.push(runtime_entry);
        self.jsonl_logger.log(jsonl_entry);
    }

    fn log_tool_event(
        &mut self,
        severity: LogSeverity,
        stage: &str,
        event_type: &str,
        message: &str,
        tool_name: &str,
    ) {
        let entry = JsonlLogEntry::new(
            event_type,
            &self.state.session_id,
            self.state.iteration,
            &severity.to_string(),
            message,
        )
        .with_tool(tool_name);
        self.log_structured(severity, stage, message, entry);
    }

    fn log_paths_event(
        &mut self,
        severity: LogSeverity,
        stage: &str,
        event_type: &str,
        message: &str,
        paths: &[PathBuf],
    ) {
        let mut entry = JsonlLogEntry::new(
            event_type,
            &self.state.session_id,
            self.state.iteration,
            &severity.to_string(),
            message,
        );
        for path in paths {
            entry = entry.with_path(path);
        }
        self.log_structured(severity, stage, message, entry);
    }

    fn log_validation_stage_event(
        &mut self,
        stage_name: &str,
        status: &str,
        duration_ms: u64,
        summary: &str,
    ) {
        let mut entry = JsonlLogEntry::new(
            "VALIDATION_STAGE",
            &self.state.session_id,
            self.state.iteration,
            &LogSeverity::Info.to_string(),
            summary,
        )
        .with_metadata("stage", stage_name)
        .with_metadata("status", status)
        .with_metadata("duration_ms", &duration_ms.to_string());

        if !summary.is_empty() {
            entry = entry.with_metadata("summary", summary);
        }

        self.log_structured(LogSeverity::Info, "VALIDATION", summary, entry);
    }

    fn record_tool_execution(
        &mut self,
        tool_name: &str,
        success: bool,
        summary: impl Into<String>,
    ) {
        self.tool_calls_executed += 1;
        self.recent_tool_executions.push(ToolExecutionRecord {
            iteration: self.state.iteration,
            tool_name: tool_name.to_string(),
            success,
            summary: summary.into(),
        });
        if self.recent_tool_executions.len() > 10 {
            self.recent_tool_executions.remove(0);
        }
    }

    fn tool_call_signature(tool_call: &ToolCall) -> String {
        let mut args = tool_call
            .arguments
            .as_map()
            .iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect::<Vec<_>>();
        args.sort();
        format!("{}:{}", tool_call.name, args.join(","))
    }

    fn is_read_only_tool(tool_name: &str) -> bool {
        matches!(
            tool_name,
            "read_file"
                | "list_dir"
                | "grep_search"
                | "dependency_graph"
                | "symbol_index"
                | "entrypoint_detector"
                | "lint_runner"
                | "test_runner"
        )
    }

    fn record_read_only_progress_policy(
        &mut self,
        tool_call: &ToolCall,
    ) -> Result<bool, ForgeError> {
        if !Self::is_read_only_tool(tool_call.name.as_str()) {
            self.consecutive_read_only_tools = 0;
            self.recent_read_only_signatures.clear();
            self.no_progress_corrections = 0;
            self.post_correction_progress_required = false;
            self.post_correction_read_only_tools = 0;
            return Ok(true);
        }

        self.consecutive_read_only_tools += 1;
        if self.post_correction_progress_required {
            self.post_correction_read_only_tools += 1;
        }
        self.recent_read_only_signatures
            .push_back(Self::tool_call_signature(tool_call));
        while self.recent_read_only_signatures.len() > READ_ONLY_CHURN_THRESHOLD as usize {
            self.recent_read_only_signatures.pop_front();
        }

        let repeated_window = self.recent_read_only_signatures.len()
            == READ_ONLY_CHURN_THRESHOLD as usize
            && self
                .recent_read_only_signatures
                .iter()
                .all(|signature| signature == &self.recent_read_only_signatures[0]);

        let post_correction_churn =
            self.post_correction_progress_required && self.post_correction_read_only_tools >= 2;

        if !post_correction_churn
            && !repeated_window
            && self.consecutive_read_only_tools < READ_ONLY_CHURN_HALT_THRESHOLD
        {
            return Ok(true);
        }

        let reason = if post_correction_churn {
            format!(
                "Post-correction no-progress detected: {} read-only tool calls after planner output normalization without mutation, validation, or completion",
                self.post_correction_read_only_tools
            )
        } else if repeated_window {
            format!(
                "Read-only churn detected: repeated identical {} calls without mutation",
                tool_call.name
            )
        } else {
            format!(
                "Read-only churn detected: {} consecutive read-only tool calls without mutation",
                self.consecutive_read_only_tools
            )
        };

        self.log_tool_event(
            LogSeverity::Warning,
            "NO_PROGRESS",
            "NO_PROGRESS_READ_ONLY_CHURN",
            &reason,
            tool_call.name.as_str(),
        );
        self.audit_governance_event(
            "no_progress_policy",
            "Reject",
            Some(if post_correction_churn {
                "post_correction_read_only_churn".to_string()
            } else {
                "read_only_churn".to_string()
            }),
            reason.clone(),
        );

        if post_correction_churn {
            self.log(
                LogSeverity::Error,
                "NO_PROGRESS",
                "POST_CORRECTION_NO_PROGRESS_HALTED",
                "Planner output was normalized, but execution returned to read-only churn; halting fail-closed",
            );
            return Err(ForgeError::ValidationFailed(reason));
        }

        if let Some(continue_running) = self.try_refactor_read_only_churn_recovery(&reason)? {
            return Ok(continue_running);
        }

        if self.no_progress_corrections >= 1
            && self.consecutive_read_only_tools >= READ_ONLY_CHURN_HALT_THRESHOLD
        {
            self.log(
                LogSeverity::Error,
                "NO_PROGRESS",
                "NO_PROGRESS_POLICY_HALTED",
                "Read-only churn persisted after correction; halting fail-closed",
            );
            return Err(ForgeError::ValidationFailed(reason));
        }

        self.no_progress_corrections += 1;
        self.repair_prompt = Some(format!(
            "{}. Next action must be a concrete write_file or apply_patch using already-read evidence, or a specific recoverable failure. Do not repeat read-only exploration.",
            reason
        ));
        self.log(
            LogSeverity::Info,
            "NO_PROGRESS",
            "NO_PROGRESS_CORRECTION_FORCED",
            "Planner correction forced toward mutation or explicit failure",
        );
        Ok(true)
    }

    fn failed_tool_signature(tool_call: &ToolCall, error: &str) -> String {
        format!(
            "{}:{}:{}",
            tool_call.name,
            Self::tool_call_signature(tool_call),
            normalize_tool_error(error)
        )
    }

    fn reset_failed_tool_policy(&mut self) {
        self.consecutive_failed_tool_calls = 0;
        self.recent_failed_tool_signatures.clear();
        self.failed_tool_corrections = 0;
    }

    fn record_failed_tool_progress_policy(
        &mut self,
        tool_call: &ToolCall,
        error: &str,
    ) -> Result<bool, ForgeError> {
        self.consecutive_failed_tool_calls += 1;
        self.recent_failed_tool_signatures
            .push_back(Self::failed_tool_signature(tool_call, error));
        while self.recent_failed_tool_signatures.len() > 3 {
            self.recent_failed_tool_signatures.pop_front();
        }

        let repeated_window = self.recent_failed_tool_signatures.len() == 3
            && self
                .recent_failed_tool_signatures
                .iter()
                .all(|signature| signature == &self.recent_failed_tool_signatures[0]);

        if !repeated_window {
            return Ok(true);
        }

        let error_class = classify_tool_failure(tool_call.name.as_str(), error);
        let reason = format!(
            "Failed-tool churn detected: repeated {} failure ({}) without state advancement",
            tool_call.name, error_class
        );
        self.log_tool_event(
            LogSeverity::Warning,
            "NO_PROGRESS",
            "NO_PROGRESS_FAILED_TOOL_CHURN",
            &reason,
            tool_call.name.as_str(),
        );
        self.audit_governance_event(
            "failed_tool_policy",
            "Reject",
            Some(error_class.to_string()),
            reason.clone(),
        );

        if self.failed_tool_corrections >= 1 {
            self.log(
                LogSeverity::Error,
                "NO_PROGRESS",
                "FAILED_TOOL_POLICY_HALTED",
                "Failed tool churn persisted after correction; halting fail-closed",
            );
            return Err(ForgeError::ValidationFailed(reason));
        }

        self.failed_tool_corrections += 1;
        let corrective_action = corrective_prompt_for_tool_failure(tool_call.name.as_str(), error);
        self.repair_prompt = Some(format!(
            "{}. Recovery instruction: {}",
            reason, corrective_action
        ));
        self.log(
            LogSeverity::Info,
            "RECOVERY",
            "FAILED_TOOL_RECOVERY_SCHEDULED",
            corrective_action,
        );
        Ok(true)
    }

    fn audit_governance_event(
        &mut self,
        event_type: &str,
        decision: &str,
        rule_broken: Option<String>,
        details: String,
    ) {
        self.governance.log_validation_event(AuditLogEntry {
            timestamp: crate::types::timestamp_now(),
            session_id: self.state.session_id.to_string(),
            iteration: self.state.iteration,
            event_type: event_type.to_string(),
            rule_broken,
            decision: decision.to_string(),
            details,
        });
    }

    /// Run the runtime loop until completion
    pub fn run(&mut self) -> RuntimeResult {
        self.log(
            LogSeverity::Info,
            "START",
            "RUNTIME_START",
            &format!("Starting task: {}", self.config.task),
        );

        let mut error: Option<ForgeError> = None;

        loop {
            // Verify state integrity before each iteration
            if let Err(e) = self.state.verify_integrity() {
                error = Some(e);
                self.log(
                    LogSeverity::Error,
                    "INTEGRITY",
                    "STATE_INTEGRITY_FAILURE",
                    "State corruption detected",
                );
                break;
            }

            // Check iteration limit
            if self.state.iteration >= self.config.max_iterations {
                let _ = self.state.error();
                self.log(
                    LogSeverity::Error,
                    "LIMIT",
                    "MAX_ITERATIONS_EXCEEDED",
                    &format!("Max iterations ({}) exceeded", self.config.max_iterations),
                );
                break;
            }

            // Execute one iteration
            match self.iteration() {
                Ok(should_continue) => {
                    if !should_continue {
                        break;
                    }
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    error = Some(e);
                    self.log(
                        LogSeverity::Error,
                        "ERROR",
                        "RUNTIME_ERROR",
                        &format!("Runtime error: {}", err_msg),
                    );
                    let _ = self.state.error();
                    break;
                }
            }

            // Increment iteration
            if let Err(e) = self.state.next_iteration() {
                error = Some(e);
                break;
            }
        }

        self.log(
            LogSeverity::Info,
            "COMPLETE",
            "RUNTIME_COMPLETE",
            &format!("Runtime finished with status: {:?}", self.state.status),
        );

        RuntimeResult {
            success: self.state.status == SessionStatus::Complete,
            final_state: self.state.clone(),
            iterations: self.state.iteration,
            logs: self.logs.clone(),
            jsonl_logs: self.jsonl_logger.entries().clone(),
            error: error.map(|e| e.to_string()),
        }
    }

    /// TASK CHAIN: Set a chain executor for multi-step execution
    #[allow(dead_code)]
    pub fn with_chain_executor(mut self, chain: crate::chain_executor::ChainExecutor) -> Self {
        self.chain_executor = Some(chain);
        self
    }

    /// TASK CHAIN: Run with chain-aware execution
    /// Each chain step runs through the full iteration pipeline
    #[allow(dead_code)]
    pub fn run_chained(&mut self) -> ChainExecutionResult {
        use crate::types::{ChainStatus, StepOutcome};

        // Take ownership of chain executor to avoid borrow issues
        if let Some(mut chain) = self.chain_executor.take() {
            // Log chain start
            let chain_id_str = chain.chain_id().to_string();
            let objective = chain.objective().to_string();
            let total_steps = chain.total_steps();

            self.log(
                LogSeverity::Info,
                "CHAIN",
                "CHAIN_START",
                &format!("Starting chain: {} ({} steps)", objective, total_steps),
            );

            // Execute each step
            loop {
                // Check if chain can continue
                if !chain.can_execute() {
                    break;
                }

                let step_idx = chain.current_step_index();
                let step_desc = chain
                    .current_step_description()
                    .unwrap_or("unnamed step")
                    .to_string();

                // Mark step started
                if let Err(e) = chain.mark_step_started() {
                    self.chain_executor = Some(chain); // Restore chain before returning
                    return ChainExecutionResult {
                        success: false,
                        chain_id: chain_id_str,
                        completed_steps: 0,
                        total_steps,
                        final_status: ChainStatus::Failed,
                        error: Some(format!("Failed to start step {}: {}", step_idx, e)),
                    };
                }

                // Update task for this step
                let step_task = format!(
                    "{} [Step {}/{}: {}]",
                    objective,
                    step_idx + 1,
                    total_steps,
                    step_desc
                );
                self.config.task = step_task.clone();

                self.log(
                    LogSeverity::Info,
                    "CHAIN",
                    "CHAIN_STEP_START",
                    &format!(
                        "Executing step {}/{}: {}",
                        step_idx + 1,
                        total_steps,
                        step_desc
                    ),
                );

                // Run the bounded runtime for this chain step.
                let step_result = self.run();
                let mut validation_report = self.state.last_validation_report.clone();

                if step_result.success && validation_report.is_none() {
                    let report = crate::types::ValidationReport::accept(
                        "Read-only chain step completed without mutations",
                    );
                    if let Err(e) = self.state.mark_validated(report.clone()) {
                        let completed_steps = chain.completed_steps();
                        self.chain_executor = Some(chain);
                        return ChainExecutionResult {
                            success: false,
                            chain_id: chain_id_str,
                            completed_steps,
                            total_steps,
                            final_status: ChainStatus::Failed,
                            error: Some(format!("Failed to mark step validated: {}", e)),
                        };
                    }
                    validation_report = Some(report);
                }

                if step_result.success {
                    if let Err(e) = self.state.save_default() {
                        let completed_steps = chain.completed_steps();
                        self.chain_executor = Some(chain);
                        return ChainExecutionResult {
                            success: false,
                            chain_id: chain_id_str,
                            completed_steps,
                            total_steps,
                            final_status: ChainStatus::Failed,
                            error: Some(format!("Failed to persist chain state: {}", e)),
                        };
                    }

                    match self.state.save_checkpoint(step_idx) {
                        Ok(path) => {
                            chain.record_checkpoint_saved(step_idx, path.clone());
                            self.log(
                                LogSeverity::Info,
                                "CHAIN",
                                "CHAIN_CHECKPOINT_SAVED",
                                &format!(
                                    "Saved checkpoint for step {} at {}",
                                    step_idx,
                                    path.display()
                                ),
                            );
                        }
                        Err(e) => {
                            let completed_steps = chain.completed_steps();
                            self.chain_executor = Some(chain);
                            return ChainExecutionResult {
                                success: false,
                                chain_id: chain_id_str,
                                completed_steps,
                                total_steps,
                                final_status: ChainStatus::Failed,
                                error: Some(format!("Failed to save chain checkpoint: {}", e)),
                            };
                        }
                    }
                }

                // Map RuntimeResult to StepOutcome
                let mut files_modified: Vec<PathBuf> = step_result
                    .final_state
                    .files_written
                    .iter()
                    .cloned()
                    .collect();
                files_modified.sort();
                let outcome = if step_result.success {
                    StepOutcome::Resolved {
                        summary: format!("Step {} completed successfully", step_desc),
                        files_modified,
                    }
                } else if let Some(ref err) = step_result.error {
                    StepOutcome::Failed {
                        reason: err.clone(),
                        recoverable: false,
                    }
                } else {
                    StepOutcome::Failed {
                        reason: "Step failed without error details".to_string(),
                        recoverable: false,
                    }
                };

                // Complete the step and potentially advance
                let completed_steps_before = chain.completed_steps();
                let step_completed =
                    match chain.complete_step_with_validation(outcome, validation_report) {
                        Ok(advanced) => advanced,
                        Err(e) => {
                            self.chain_executor = Some(chain); // Restore chain before returning
                            return ChainExecutionResult {
                                success: false,
                                chain_id: chain_id_str,
                                completed_steps: completed_steps_before,
                                total_steps,
                                final_status: ChainStatus::Failed,
                                error: Some(format!("Step completion error: {}", e)),
                            };
                        }
                    };

                if !step_completed {
                    // Chain complete or failed
                    break;
                }

                // Reset state for next step
                let next_task = format!(
                    "{} [Step {}/{}: {}]",
                    objective,
                    chain.current_step_index() + 1,
                    total_steps,
                    chain.current_step_description().unwrap_or("unnamed step")
                );
                match self.state.continue_chain_step(
                    next_task.clone(),
                    self.config.max_iterations,
                    self.config.mode,
                ) {
                    Ok(next_state) => {
                        self.state = next_state;
                        self.config.task = next_task;
                    }
                    Err(e) => {
                        let completed_steps = chain.completed_steps();
                        self.chain_executor = Some(chain);
                        return ChainExecutionResult {
                            success: false,
                            chain_id: chain_id_str,
                            completed_steps,
                            total_steps,
                            final_status: ChainStatus::Failed,
                            error: Some(format!("Failed to prepare next chain step: {}", e)),
                        };
                    }
                }
            }

            // Build final result
            let completed = chain.completed_steps();
            let success = chain.is_complete();
            let summary = chain.summary();

            self.log(
                LogSeverity::Info,
                "CHAIN",
                if success {
                    "CHAIN_COMPLETE"
                } else {
                    "CHAIN_FAILED"
                },
                &format!(
                    "Chain {}: {}/{} steps completed",
                    if success { "complete" } else { "failed" },
                    completed,
                    total_steps
                ),
            );

            // Restore chain executor
            self.chain_executor = Some(chain);

            ChainExecutionResult {
                success,
                chain_id: chain_id_str,
                completed_steps: completed,
                total_steps,
                final_status: summary.status,
                error: if !success {
                    Some("Chain halted due to step failure".to_string())
                } else {
                    None
                },
            }
        } else {
            // No chain - run single task
            let result = self.run();
            ChainExecutionResult {
                success: result.success,
                chain_id: "single-task".to_string(),
                completed_steps: if result.success { 1 } else { 0 },
                total_steps: 1,
                final_status: if result.success {
                    ChainStatus::Complete
                } else {
                    ChainStatus::Failed
                },
                error: result.error,
            }
        }
    }

    /// TASK CHAIN: Get current chain status if present
    #[allow(dead_code)]
    pub fn chain_summary(&self) -> Option<crate::chain_executor::ChainSummary> {
        self.chain_executor.as_ref().map(|c| c.summary())
    }

    /// Execute one iteration of the runtime loop
    fn iteration(&mut self) -> Result<bool, ForgeError> {
        let iteration = self.state.iteration;
        self.log(
            LogSeverity::Info,
            "ITERATION",
            "ITERATION_START",
            &format!("--- Iteration {} ---", iteration),
        );

        // Stage 1: Preflight
        let preflight_result = self.preflight()?;
        if !preflight_result {
            self.log(
                LogSeverity::Warning,
                "PREFLIGHT",
                "PREFLIGHT_FAILED",
                "Preflight checks failed",
            );
            return Ok(false);
        }
        self.log(
            LogSeverity::Info,
            "PREFLIGHT",
            "PREFLIGHT_PASSED",
            "Checks passed",
        );

        if self.complete_if_goal_ready("POST_SUCCESS_PREFLIGHT_COMPLETE")? {
            return Ok(false);
        }

        // Stage 2: Planner invocation with HARDENED VALIDATION (PHASE 4)
        let state_view = self.build_state_view();
        let raw_planner_output = self.planner.generate_raw(&state_view)?;
        let validation_context = self.build_protocol_validation_context(&state_view);

        self.log(
            LogSeverity::Info,
            "VALIDATE",
            "PROTOCOL_VALIDATION_START",
            "Running ProtocolValidator on planner output",
        );

        let adapter_result = self
            .output_adapter
            .process(&raw_planner_output, &validation_context);

        // Log validation result
        match &adapter_result {
            AdapterResult::Success(_) => {
                self.log(
                    LogSeverity::Info,
                    "VALIDATE",
                    "PROTOCOL_VALIDATION_ACCEPT",
                    "Canonical JSON accepted (fast path)",
                );
                self.audit_governance_event(
                    "protocol_validation",
                    "Accept",
                    None,
                    "Canonical JSON accepted (fast path)".to_string(),
                );
                self.consecutive_validation_failures = 0;
                self.retry_count = 0; // Reset retry counter on success
                self.repair_prompt = None; // Clear repair prompt
            }
            AdapterResult::Normalized { action, .. } => {
                self.log(
                    LogSeverity::Warning,
                    "VALIDATE",
                    "PROTOCOL_VALIDATION_NORMALIZED",
                    &format!(
                        "Normalized output: {} -> {}",
                        action.drift_pattern, action.reason
                    ),
                );
                self.audit_governance_event(
                    "protocol_validation",
                    "Normalized",
                    Some(action.drift_pattern.clone()),
                    action.reason.clone(),
                );
                self.consecutive_validation_failures = 0;
                self.retry_count = 0; // Reset retry counter on success
                self.repair_prompt = None; // Clear repair prompt
                self.post_correction_progress_required = true;
                self.post_correction_read_only_tools = 0;
            }
            AdapterResult::Reject {
                reason,
                tier,
                failure_class,
                ..
            } => {
                self.log(
                    LogSeverity::Error,
                    "VALIDATE",
                    "PROTOCOL_VALIDATION_REJECT",
                    &format!("Rejected (tier {}): {} - {}", tier, failure_class, reason),
                );
                self.audit_governance_event(
                    "protocol_validation",
                    "Reject",
                    Some(failure_class.to_string()),
                    reason.clone(),
                );
                self.consecutive_validation_failures += 1;

                // PHASE 4: Repair Loop - Check if retry is allowed
                if self.repair_handler.can_retry(self.retry_count) {
                    // Generate repair prompt and store for next iteration
                    let repair_prompt = self
                        .repair_handler
                        .generate_repair_prompt(reason, failure_class);
                    self.repair_prompt = Some(repair_prompt);
                    self.last_rejection = Some((reason.clone(), failure_class.clone()));
                    self.retry_count += 1;

                    self.log(
                        LogSeverity::Info,
                        "REPAIR",
                        "REPAIR_LOOP_ACTIVE",
                        &format!("Retry {}/3 with repair prompt", self.retry_count),
                    );
                    return Ok(true); // Continue to retry with repair context
                } else {
                    // Max retries exceeded - escalate
                    self.log(
                        LogSeverity::Error,
                        "ESCALATE",
                        "REPAIR_LOOP_EXHAUSTED",
                        "Max retries (3) exceeded - escalating to halt",
                    );
                    return Err(ForgeError::ValidationFailed(format!(
                        "Planner failed to produce valid output after {} retries: {}",
                        self.retry_count, reason
                    )));
                }
            }
            AdapterResult::Escalate {
                reason,
                violation,
                failure_class,
                ..
            } => {
                self.log(
                    LogSeverity::Error,
                    "VALIDATE",
                    "PROTOCOL_VALIDATION_ESCALATE",
                    &format!("Escalating ({}): {} - {}", violation, failure_class, reason),
                );
                self.audit_governance_event(
                    "protocol_validation",
                    "Escalate",
                    Some(violation.clone()),
                    reason.clone(),
                );
                return Err(ForgeError::ValidationFailed(format!(
                    "Critical violation: {}",
                    violation
                )));
            }
        }

        // Extract typed output
        let planner_output = match adapter_result {
            AdapterResult::Success(output) => output,
            AdapterResult::Normalized { output, .. } => output,
            _ => return Ok(true), // Should not reach here due to early returns above
        };

        let output_type = match &planner_output {
            PlannerOutput::ToolCall(_) => "tool_call",
            PlannerOutput::Completion { .. } => "completion",
            PlannerOutput::Failure { .. } => "failure",
        };
        self.log(
            LogSeverity::Info,
            "PLANNER",
            "PLANNER_OUTPUT",
            &format!(
                "Output type: {} (planner: {})",
                output_type,
                self.planner.planner_type()
            ),
        );

        // Stage 3: Tool call parsing and completion check with HARDENED GATES
        match planner_output {
            PlannerOutput::Completion { reason } => {
                // PHASE 4: Run CompletionGate before allowing session termination
                let gate_result = CompletionGate::evaluate(
                    &reason,
                    &self.state,
                    self.pending_validation,
                    &self.known_errors,
                );

                match gate_result {
                    CompletionGateResult::Accept => {
                        let reason_str = reason.as_str().to_string();
                        self.state.complete(reason)?;
                        self.log(
                            LogSeverity::Info,
                            "COMPLETION",
                            "COMPLETION_GATE_ACCEPT",
                            &format!("Completion gate passed. Signal: {}", reason_str),
                        );
                        Ok(false) // Exit loop
                    }
                    CompletionGateResult::Reject {
                        reason: reject_reason,
                        failure_class,
                    } => {
                        self.log(
                            LogSeverity::Error,
                            "COMPLETION",
                            "COMPLETION_GATE_REJECT",
                            &format!(
                                "Completion gate rejected ({}): {}",
                                failure_class, reject_reason
                            ),
                        );
                        self.repair_prompt = Some(format!(
                            "Completion was rejected. Emit a completion reason that cites specific observable facts such as the file path, the created function name, and the resulting file state. Reject reason: {}",
                            reject_reason
                        ));
                        // Don't terminate - continue to let planner make progress
                        Ok(true)
                    }
                }
            }
            PlannerOutput::Failure {
                reason,
                recoverable,
            } => {
                self.log(
                    LogSeverity::Error,
                    "FAILURE",
                    "PLANNER_FAILURE",
                    &format!("Planner failure (recoverable: {}): {}", recoverable, reason),
                );
                if !recoverable {
                    self.state.halt()?;
                    return Ok(false);
                }
                Ok(true) // Continue to retry
            }
            PlannerOutput::ToolCall(tool_call) => {
                let arguments = serde_json::to_string(tool_call.arguments.as_map())
                    .unwrap_or_else(|_| "{}".to_string());
                let entry = JsonlLogEntry::new(
                    "TOOL_CALL_PARSED",
                    &self.state.session_id,
                    self.state.iteration,
                    &LogSeverity::Info.to_string(),
                    &format!("Tool parsed: {}", tool_call.name),
                )
                .with_tool(tool_call.name.as_str())
                .with_metadata("arguments", &arguments);
                self.log_structured(
                    LogSeverity::Info,
                    "PARSE",
                    &format!("Tool parsed: {}", tool_call.name),
                    entry,
                );
                self.execute_tool_call_hardened(&tool_call)
            }
        }
    }

    /// Execute a tool call and continue the loop
    #[allow(dead_code)]
    fn execute_tool_call(&mut self, tool_call: &ToolCall) -> Result<bool, ForgeError> {
        // Stage 4: Tool validation
        let validation_result = self.validate_tool_call(tool_call);
        if !validation_result {
            self.log(
                LogSeverity::Warning,
                "VALIDATE",
                "TOOL_VALIDATION_FAILED",
                "Tool validation failed",
            );
            return Ok(true); // Continue to retry
        }
        self.log(
            LogSeverity::Info,
            "VALIDATE",
            "TOOL_VALID",
            "Tool call valid",
        );
        self.log(
            LogSeverity::Info,
            "PARSE",
            "TOOL_PARSE",
            &format!("Tool: {}", tool_call.name),
        );

        // Stage 4b: Read-before-write enforcement for apply_patch
        if tool_call.name.as_str() == "apply_patch" {
            let path_str = tool_call
                .arguments
                .get("file_path")
                .ok_or_else(|| ForgeError::MissingArgument("file_path".to_string()))?;
            let path = PathBuf::from(path_str);

            // Check file was read
            if !self.state.is_file_fully_read(&path) {
                self.log(
                    LogSeverity::Error,
                    "ENFORCE",
                    "READ_BEFORE_WRITE_VIOLATION",
                    &format!(
                        "Read-before-write violation: {} must be fully read before patching",
                        path.display()
                    ),
                );
                return Ok(true); // Continue loop - planner should read the file
            }

            // Capture snapshot before patching (for revert)
            if let Ok(content) = std::fs::read_to_string(&path) {
                self.state.capture_snapshot(&path, &content);
                let normalized_path = normalize_path(&path);
                if normalized_path != path {
                    self.state.capture_snapshot(&normalized_path, &content);
                }
                self.log(
                    LogSeverity::Debug,
                    "SNAPSHOT",
                    "SNAPSHOT_CAPTURED",
                    &format!("Captured snapshot for {}", path.display()),
                );
            }
        }

        // Stage 5: Tool execution
        self.log(
            LogSeverity::Info,
            "EXECUTE",
            "TOOL_EXECUTE",
            &format!("Executing {}", tool_call.name),
        );

        // Create execution context
        let ctx = ExecutionContext {
            session_id: self.state.session_id.clone(),
            iteration: self.state.iteration,
            mode: self.config.mode,
            working_dir: PathBuf::from("."),
        };

        let tool_result = self.executor.execute(tool_call, &ctx)?;

        if !tool_result.success {
            let error_msg = tool_result
                .error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string());
            self.log(
                LogSeverity::Warning,
                "EXECUTE",
                "TOOL_FAILED",
                &format!("Tool failed: {}", error_msg),
            );
            return Ok(true); // Continue to retry
        }
        self.log(
            LogSeverity::Info,
            "EXECUTE",
            "TOOL_SUCCESS",
            &format!(
                "Tool succeeded: {} ({} ms)",
                tool_result.output.as_ref().unwrap_or(&"OK".to_string()),
                tool_result.execution_time_ms
            ),
        );

        // Stage 6: Handle read_file results
        if tool_call.name.as_str() == "read_file" && tool_result.success {
            // Record file read in state
            let path_str = tool_call
                .arguments
                .get("path")
                .ok_or_else(|| ForgeError::MissingArgument("path".to_string()))?;
            let path = PathBuf::from(path_str);

            // Read the file again to create the FileRecord
            // (In production we'd cache this from the tool execution)
            if let Ok(content) = std::fs::read_to_string(&path) {
                let offset = tool_call
                    .arguments
                    .get("offset")
                    .and_then(|s| s.parse::<usize>().ok());
                let limit = tool_call
                    .arguments
                    .get("limit")
                    .and_then(|s| s.parse::<usize>().ok());
                let file_record = self.build_file_record_from_read(
                    &path,
                    &content,
                    self.state.iteration,
                    false,
                    offset,
                    limit,
                );
                self.state.record_file_read(file_record);
                self.log(
                    LogSeverity::Info,
                    "TRACK",
                    "FILE_READ_TRACKED",
                    &format!("Recorded file read: {}", path.display()),
                );
            }
        }

        // Stage 6b: Mutation detection
        let mutations = &tool_result.mutations;
        if mutations.is_empty() {
            self.log(
                LogSeverity::Info,
                "MUTATION",
                "NO_MUTATIONS",
                "No mutations detected",
            );
            return Ok(true); // Continue loop
        }

        self.validate_and_commit_mutations(mutations)
    }

    /// Preflight checks before planner invocation
    fn preflight(&self) -> Result<bool, ForgeError> {
        // Check state integrity
        self.state.verify_integrity()?;

        // Check session status
        if self.state.status != SessionStatus::Running {
            return Ok(false);
        }

        // Check iteration bounds (u32 can't be negative, but check for overflow)
        if self.state.iteration > self.state.max_iterations {
            return Ok(false);
        }

        Ok(true)
    }

    /// Validate tool call structure
    fn validate_tool_call(&self, tool_call: &ToolCall) -> bool {
        // Validate tool name (already strongly typed)
        if tool_call.name.as_str().is_empty() {
            return false;
        }

        // Additional validation could go here
        true
    }

    fn complete_if_goal_ready(&mut self, event_type: &str) -> Result<bool, ForgeError> {
        match CompletionGate::readiness(&self.state, self.pending_validation, &self.known_errors) {
            CompletionReadiness::Ready { reason } => {
                self.state.complete(CompletionReason::new(&reason))?;
                self.log(
                    LogSeverity::Info,
                    "COMPLETION",
                    event_type,
                    &format!("Post-success completion accepted: {}", reason),
                );
                Ok(true)
            }
            CompletionReadiness::NotReady { .. } => Ok(false),
        }
    }

    /// Get current state (for debugging)
    #[allow(dead_code)]
    pub fn get_state(&self) -> &AgentState {
        &self.state
    }

    fn build_file_record_from_read(
        &self,
        path: &Path,
        content: &str,
        iteration: u32,
        normalize_paths: bool,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> crate::types::FileRecord {
        let lines_read = limit.map(|limit| (offset.unwrap_or(1), limit));
        let observed_content = extract_observed_read_content(content, offset, limit);
        let record_path = if normalize_paths {
            normalize_path(path)
        } else {
            path.to_path_buf()
        };

        crate::types::FileRecord::new(record_path, content, lines_read, iteration)
            .with_observed_content(observed_content)
    }

    fn validate_and_commit_mutations(
        &mut self,
        mutations: &[Mutation],
    ) -> Result<bool, ForgeError> {
        let mut combined_mutations = self.deferred_validation_mutations.clone();
        combined_mutations.extend(mutations.iter().cloned());

        self.log(
            LogSeverity::Info,
            "MUTATION",
            "MUTATIONS_DETECTED",
            &format!(
                "Detected {} mutation(s); {} pending validation mutation(s)",
                mutations.len(),
                self.deferred_validation_mutations.len()
            ),
        );
        self.pending_validation = true;
        self.pending_validation_items = combined_mutations
            .iter()
            .map(|mutation| mutation.path.display().to_string())
            .collect();
        // Update AgentState for CSS metadata tracking
        self.state.pending_validations = self.pending_validation_items.clone();
        self.log(
            LogSeverity::Info,
            "VALIDATION",
            "VALIDATION_RUNNING",
            "Running validation pipeline",
        );

        let validation_engine = ValidationEngine::new().with_auto_revert(self.config.auto_revert);
        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let validation_run = validation_engine.validate_detailed(&combined_mutations, &working_dir);
        for stage_result in &validation_run.stage_results {
            let status = if stage_result.skipped {
                "skipped"
            } else if stage_result.passed {
                "passed"
            } else {
                "failed"
            };
            let summary = if stage_result.stderr.is_empty() {
                stage_result.stdout.as_str()
            } else {
                stage_result.stderr.as_str()
            };
            self.log_validation_stage_event(
                &stage_result.stage.to_string(),
                status,
                stage_result.execution_time_ms,
                summary,
            );
        }
        let validation = validation_engine.generate_report(&validation_run, &combined_mutations);

        match &validation_run.outcome {
            crate::execution::ValidationOutcome::Accept => {
                self.consecutive_validation_failures = 0;
                self.repair_prompt = None;
                self.known_errors.clear();
                self.log(
                    LogSeverity::Info,
                    "VALIDATION",
                    "VALIDATION_ACCEPT",
                    &format!("✓✓ All validation passed: {}", validation.message),
                );
                self.audit_governance_event(
                    "mutation_validation",
                    "Accept",
                    None,
                    validation.message.clone(),
                );
            }
            crate::execution::ValidationOutcome::Reject {
                reason,
                failed_stage,
            } => {
                self.log(
                    LogSeverity::Error,
                    "VALIDATION",
                    "VALIDATION_REJECT",
                    &format!(
                        "✗ Validation failed at stage {:?}: {}",
                        failed_stage, reason
                    ),
                );

                let evidence = format!("Validation failed at stage {}: {}", failed_stage, reason);
                if matches!(failed_stage, EngineValidationStage::Format)
                    && let Some(format_mutations) =
                        self.try_rust_format_recovery(&combined_mutations, &working_dir)?
                {
                    combined_mutations.extend(format_mutations);
                    refresh_mutation_hashes(&mut combined_mutations);
                    self.deferred_validation_mutations = combined_mutations;
                    self.pending_validation = true;
                    self.pending_validation_items = self
                        .deferred_validation_mutations
                        .iter()
                        .map(|mutation| mutation.path.display().to_string())
                        .collect();
                    self.state.pending_validations = self.pending_validation_items.clone();
                    self.repair_prompt = Some(
                        "Rust formatting recovery applied from validation evidence; validation will rerun before commit."
                            .to_string(),
                    );
                    return self.validate_and_commit_mutations(&[]);
                }

                if let Some(companion_mutation) =
                    self.try_rust_companion_surface_recovery(reason, &combined_mutations)?
                {
                    self.deferred_validation_mutations = combined_mutations;
                    self.pending_validation = true;
                    self.pending_validation_items = self
                        .deferred_validation_mutations
                        .iter()
                        .map(|mutation| mutation.path.display().to_string())
                        .collect();
                    self.state.pending_validations = self.pending_validation_items.clone();
                    self.consecutive_validation_failures += 1;
                    self.known_errors.push(format!(
                        "Rust companion-surface recovery applied from validation evidence: {}",
                        reason
                    ));
                    return self.validate_and_commit_mutations(&[companion_mutation]);
                }

                if should_defer_multi_file_validation(
                    &self.config.task,
                    reason,
                    &combined_mutations,
                    self.consecutive_validation_failures,
                ) {
                    self.deferred_validation_mutations = combined_mutations;
                    self.pending_validation = true;
                    self.pending_validation_items = self
                        .deferred_validation_mutations
                        .iter()
                        .map(|mutation| mutation.path.display().to_string())
                        .collect();
                    self.state.pending_validations = self.pending_validation_items.clone();
                    self.consecutive_validation_failures += 1;
                    self.known_errors.push(format!(
                        "Deferred multi-file validation failure: {}",
                        reason
                    ));
                    self.audit_governance_event(
                        "multi_file_validation",
                        "Deferred",
                        Some(failed_stage.to_string()),
                        reason.clone(),
                    );
                    self.log_paths_event(
                        LogSeverity::Warning,
                        "RECOVERY",
                        "MULTI_FILE_VALIDATION_DEFERRED",
                        &format!(
                            "Validation failure kept pending for bounded multi-file recovery: {}",
                            evidence
                        ),
                        &self
                            .deferred_validation_mutations
                            .iter()
                            .map(|mutation| mutation.path.clone())
                            .collect::<Vec<_>>(),
                    );
                    self.repair_prompt = Some(format!(
                        "A multi-file edit is pending validation and was NOT reverted. Evidence: {}. Pending paths: {}. {} Complete the companion source/config/test surface now. Do not repeat already pending writes. Re-read any existing file before mutating it; create missing companion files when the compiler names them.",
                        evidence,
                        format_mutation_paths(&self.deferred_validation_mutations),
                        multi_file_recovery_hint(reason, &self.deferred_validation_mutations),
                    ));
                    self.log(
                        LogSeverity::Info,
                        "RECOVERY",
                        "MULTI_FILE_RECOVERY_SCHEDULED",
                        "Pending multi-file validation recovery scheduled",
                    );
                    return Ok(true);
                }

                if self.config.auto_revert {
                    self.log(
                        LogSeverity::Warning,
                        "REVERT",
                        "REVERTING",
                        "Validation failed - reverting mutations",
                    );
                    self.revert_mutations_from_snapshots(&combined_mutations)?;
                }

                self.pending_validation = false;
                self.pending_validation_items.clear();
                self.state.pending_validations.clear();
                self.deferred_validation_mutations.clear();
                self.consecutive_validation_failures += 1;
                self.known_errors
                    .push(format!("Validation failed: {}", reason));
                self.audit_governance_event(
                    "mutation_validation",
                    "Reject",
                    Some(failed_stage.to_string()),
                    reason.clone(),
                );
                self.log(
                    LogSeverity::Warning,
                    "RECOVERY",
                    "RECOVERY_STEP_GENERATED",
                    &format!(
                        "Generated live recovery step from validation evidence: {}",
                        evidence
                    ),
                );
                if self.consecutive_validation_failures <= MAX_LIVE_VALIDATION_RECOVERY_ATTEMPTS {
                    self.repair_prompt = Some(format!(
                        "Validation failed and mutations were reverted. Evidence: {}. Generate a bounded fix step, re-read any file you will mutate if needed, then retry within policy.",
                        evidence
                    ));
                    self.log(
                        LogSeverity::Info,
                        "RECOVERY",
                        "RECOVERY_RETRY_SCHEDULED",
                        &format!(
                            "Retry {}/{} scheduled after validation reject",
                            self.consecutive_validation_failures,
                            MAX_LIVE_VALIDATION_RECOVERY_ATTEMPTS
                        ),
                    );
                    return Ok(true);
                }

                self.log(
                    LogSeverity::Error,
                    "RECOVERY",
                    "RECOVERY_POLICY_EXHAUSTED",
                    "Validation recovery retry limit reached",
                );
                return Err(ForgeError::ValidationFailed(evidence));
            }
            crate::execution::ValidationOutcome::Escalate { reason } => {
                self.log(
                    LogSeverity::Error,
                    "VALIDATION",
                    "VALIDATION_ESCALATE",
                    &format!("✗✗ Validation escalation: {}", reason),
                );
                self.audit_governance_event(
                    "mutation_validation",
                    "Escalate",
                    None,
                    reason.clone(),
                );
                return Err(ForgeError::ValidationFailed(reason.clone()));
            }
        }

        self.state.commit(&validation, &combined_mutations)?;
        self.clear_snapshots_for_mutations(&combined_mutations);
        self.pending_validation = false;
        self.pending_validation_items.clear();
        self.state.pending_validations.clear();
        self.deferred_validation_mutations.clear();
        self.state.save_default()?;
        let written_paths: Vec<PathBuf> = combined_mutations
            .iter()
            .map(|mutation| mutation.path.clone())
            .collect();
        self.log_paths_event(
            LogSeverity::Info,
            "COMMIT",
            "STATE_COMMITTED",
            &format!(
                "State committed. Files written: {}",
                self.state.files_written.len()
            ),
            &written_paths,
        );

        if self.complete_if_goal_ready("POST_VALIDATION_COMPLETE")? {
            return Ok(false);
        }

        Ok(true)
    }

    fn try_rust_companion_surface_recovery(
        &mut self,
        reason: &str,
        mutations: &[Mutation],
    ) -> Result<Option<Mutation>, ForgeError> {
        let Some(action) = rust_companion_recovery_action(reason, mutations)? else {
            return Ok(None);
        };

        let current = std::fs::read_to_string(&action.target_path).map_err(|e| {
            ForgeError::IoError(format!(
                "Failed to read Rust companion target {}: {}",
                action.target_path.display(),
                e
            ))
        })?;
        if current == action.updated_content {
            return Ok(None);
        }

        self.state.capture_snapshot(&action.target_path, &current);
        self.state
            .capture_snapshot(&normalize_path(&action.target_path), &current);
        write_file_atomically(&action.target_path, &action.updated_content)?;

        let message = format!(
            "Applied Rust companion-surface recovery: {} on {} from {}",
            action.edit_class,
            action.target_path.display(),
            action.evidence_class
        );
        self.audit_governance_event(
            "rust_companion_surface_recovery",
            "Applied",
            Some(action.evidence_class.to_string()),
            message.clone(),
        );
        self.log_paths_event(
            LogSeverity::Warning,
            "RECOVERY",
            "RUST_COMPANION_RECOVERY_APPLIED",
            &message,
            std::slice::from_ref(&action.target_path),
        );
        self.repair_prompt = Some(format!(
            "{}. Validation will rerun against pending mutations plus companion target {}.",
            message,
            action.target_path.display()
        ));

        Ok(Some(Mutation {
            path: action.target_path,
            mutation_type: MutationType::Patch,
            content_hash_before: None,
            content_hash_after: None,
        }))
    }

    fn try_rust_format_recovery(
        &mut self,
        mutations: &[Mutation],
        working_dir: &Path,
    ) -> Result<Option<Vec<Mutation>>, ForgeError> {
        if !mutations.iter().any(|mutation| {
            mutation
                .path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        }) {
            return Ok(None);
        }

        let Some(project_root) = find_upward_marker(working_dir, "Cargo.toml") else {
            return Ok(None);
        };
        let rust_files = collect_rust_files(&project_root)?;
        if rust_files.is_empty() {
            return Ok(None);
        }

        let mut before = Vec::new();
        for path in &rust_files {
            if let Ok(content) = std::fs::read_to_string(path) {
                if self.matching_snapshot_keys(path).is_empty() {
                    self.state.capture_snapshot(path, &content);
                    self.state.capture_snapshot(&normalize_path(path), &content);
                }
                let hash = crate::crypto_hash::compute_content_hash(&content);
                before.push((path.clone(), content, hash));
            }
        }

        let output = Command::new("cargo")
            .arg("fmt")
            .current_dir(&project_root)
            .output()
            .map_err(|e| ForgeError::IoError(format!("Failed to run cargo fmt: {}", e)))?;

        if !output.status.success() {
            restore_changed_files(&before)?;
            self.audit_governance_event(
                "rust_format_recovery",
                "Reject",
                Some("cargo_fmt_failed".to_string()),
                String::from_utf8_lossy(&output.stderr).to_string(),
            );
            return Ok(None);
        }

        let mut changed = Vec::new();
        let mut changed_paths = Vec::new();
        for (path, _content_before, hash_before) in before {
            let Ok(content_after) = std::fs::read_to_string(&path) else {
                continue;
            };
            let hash_after = crate::crypto_hash::compute_content_hash(&content_after);
            if hash_before != hash_after {
                changed_paths.push(path.clone());
                if !mutations
                    .iter()
                    .any(|mutation| same_snapshot_target(&mutation.path, &path))
                {
                    changed.push(Mutation {
                        path,
                        mutation_type: MutationType::Patch,
                        content_hash_before: Some(hash_before),
                        content_hash_after: Some(hash_after),
                    });
                }
            }
        }

        if changed_paths.is_empty() {
            return Ok(None);
        }

        self.audit_governance_event(
            "rust_format_recovery",
            "Applied",
            Some("cargo_fmt".to_string()),
            format!(
                "Applied cargo fmt to {} Rust file(s) after format validation rejected pending mutations",
                changed_paths.len()
            ),
        );
        self.log_paths_event(
            LogSeverity::Warning,
            "RECOVERY",
            "RUST_FORMAT_RECOVERY_APPLIED",
            "Applied deterministic cargo fmt recovery from validation evidence",
            &changed_paths,
        );

        Ok(Some(changed))
    }

    fn try_refactor_read_only_churn_recovery(
        &mut self,
        reason: &str,
    ) -> Result<Option<bool>, ForgeError> {
        let task_lower = self.config.task.to_lowercase();
        if !(task_lower.contains("refactor")
            && task_lower.contains("prefix")
            && task_lower.contains("helper"))
        {
            return Ok(None);
        }
        if self.no_progress_corrections < 1
            || self.consecutive_read_only_tools < READ_ONLY_CHURN_HALT_THRESHOLD
        {
            return Ok(None);
        }

        let path = PathBuf::from("src/lib.rs");
        let normalized_path = normalize_path(&path);
        let current = std::fs::read_to_string(&path).map_err(|e| {
            ForgeError::IoError(format!(
                "Failed to read {} for refactor recovery: {}",
                path.display(),
                e
            ))
        })?;

        let Some(updated) = deterministic_prefix_refactor(&current) else {
            return Ok(None);
        };
        if updated == current {
            return Ok(None);
        }

        let rbw_result = ReadBeforeWriteGate::evaluate(
            &normalized_path,
            true,
            &self.state,
            Some(current.as_str()),
        );
        if let ReadBeforeWriteResult::Block {
            reason,
            failure_class,
            required_action,
        } = rbw_result
        {
            self.audit_governance_event(
                "refactor_read_only_churn_recovery",
                "Reject",
                Some(failure_class.to_string()),
                format!("Read-before-write blocked deterministic refactor recovery: {reason}. Required: {required_action}"),
            );
            return Ok(None);
        }

        self.state.capture_snapshot(&path, &current);
        self.state.capture_snapshot(&normalized_path, &current);
        write_file_atomically(&path, &updated)?;

        let mutation = Mutation {
            path: path.clone(),
            mutation_type: MutationType::Patch,
            content_hash_before: Some(crate::crypto_hash::compute_content_hash(&current)),
            content_hash_after: Some(crate::crypto_hash::compute_content_hash(&updated)),
        };
        self.audit_governance_event(
            "refactor_read_only_churn_recovery",
            "Applied",
            Some("prefix_helper_refactor".to_string()),
            format!(
                "Applied deterministic prefix-helper refactor after read-only churn: {}",
                reason
            ),
        );
        self.log_paths_event(
            LogSeverity::Warning,
            "RECOVERY",
            "REFACTOR_CHURN_RECOVERY_APPLIED",
            "Applied deterministic refactor recovery from read-only churn evidence",
            std::slice::from_ref(&path),
        );

        self.consecutive_read_only_tools = 0;
        self.recent_read_only_signatures.clear();
        self.no_progress_corrections = 0;
        self.post_correction_progress_required = false;
        self.post_correction_read_only_tools = 0;

        self.validate_and_commit_mutations(&[mutation]).map(Some)
    }

    fn clear_snapshots_for_mutations(&mut self, mutations: &[Mutation]) {
        for mutation in mutations {
            self.clear_snapshots_for_path(&mutation.path);
        }
    }

    fn clear_snapshots_for_path(&mut self, path: &PathBuf) {
        for key in self.matching_snapshot_keys(path) {
            self.state.clear_snapshot(&key);
        }
    }

    fn matching_snapshot_keys(&self, path: &PathBuf) -> Vec<PathBuf> {
        self.state
            .snapshots
            .keys()
            .filter(|candidate| same_snapshot_target(candidate, path))
            .cloned()
            .collect()
    }

    fn revert_mutations_from_snapshots(
        &mut self,
        mutations: &[Mutation],
    ) -> Result<(), ForgeError> {
        for mutation in mutations.iter().rev() {
            if let Some(snapshot) = self
                .matching_snapshot_keys(&mutation.path)
                .into_iter()
                .find_map(|key| self.state.get_snapshot(&key).cloned())
            {
                if let Some(parent) = mutation.path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        ForgeError::IoError(format!(
                            "Failed to prepare revert directory for {}: {}",
                            mutation.path.display(),
                            e
                        ))
                    })?;
                }
                std::fs::write(&mutation.path, &snapshot.content).map_err(|e| {
                    ForgeError::IoError(format!(
                        "Failed to restore snapshot for {}: {}",
                        mutation.path.display(),
                        e
                    ))
                })?;
                self.log(
                    LogSeverity::Info,
                    "REVERT",
                    "SNAPSHOT_RESTORED",
                    &format!("Restored snapshot for {}", mutation.path.display()),
                );
                self.clear_snapshots_for_path(&mutation.path);
                continue;
            }

            match mutation.mutation_type {
                MutationType::Write => {
                    crate::tool_registry::delete_file(&mutation.path)?;
                    self.log(
                        LogSeverity::Info,
                        "REVERT",
                        "WRITE_REMOVED",
                        &format!("Removed new file {}", mutation.path.display()),
                    );
                }
                MutationType::Patch | MutationType::Delete | MutationType::Move => {
                    return Err(ForgeError::IoError(format!(
                        "No snapshot available to revert {} on {}",
                        mutation_kind_name(mutation.mutation_type),
                        mutation.path.display()
                    )));
                }
            }
        }

        Ok(())
    }

    /// PHASE 4: Hardened tool call execution with ReadBeforeWriteGate
    fn execute_tool_call_hardened(&mut self, tool_call: &ToolCall) -> Result<bool, ForgeError> {
        // Stage 4: Tool validation
        let validation_result = self.validate_tool_call(tool_call);
        if !validation_result {
            self.log(
                LogSeverity::Warning,
                "VALIDATE",
                "TOOL_VALIDATION_FAILED",
                "Tool validation failed",
            );
            return Ok(true); // Continue to retry
        }
        self.log(
            LogSeverity::Info,
            "VALIDATE",
            "TOOL_VALID",
            "Tool call valid",
        );

        // PHASE 4: Read-before-write enforcement with hash checking for ALL mutation tools
        let mutation_tools = ["write_file", "apply_patch", "delete_file"];
        if mutation_tools.contains(&tool_call.name.as_str()) {
            if self.complete_if_goal_ready("POST_SUCCESS_MUTATION_SUPPRESSED")? {
                self.log_tool_event(
                    LogSeverity::Info,
                    "NO_PROGRESS",
                    "NONESSENTIAL_MUTATION_SUPPRESSED",
                    &format!(
                        "Skipped {} because completion evidence is already satisfied",
                        tool_call.name
                    ),
                    tool_call.name.as_str(),
                );
                return Ok(false);
            }

            let path = self.extract_mutation_path(tool_call)?;
            let normalized_path = normalize_path(&path);

            // Check if file exists (determines read authority requirement)
            let is_existing = path.exists();

            // Get current content for hash verification (if file exists)
            let current_content = if is_existing {
                std::fs::read_to_string(&path).ok()
            } else {
                None
            };

            // PHASE 4: Run ReadBeforeWriteGate
            let rbw_result = ReadBeforeWriteGate::evaluate(
                &normalized_path,
                is_existing,
                &self.state,
                current_content.as_deref(),
            );

            match rbw_result {
                ReadBeforeWriteResult::Allow => {
                    self.log(
                        LogSeverity::Info,
                        "ENFORCE",
                        "READ_BEFORE_WRITE_ALLOW",
                        &format!("Read authority confirmed for {}", path.display()),
                    );
                }
                ReadBeforeWriteResult::Block {
                    reason,
                    failure_class,
                    required_action,
                } => {
                    self.log(
                        LogSeverity::Error,
                        "ENFORCE",
                        "READ_BEFORE_WRITE_BLOCK",
                        &format!(
                            "Read-before-write gate blocked ({}): {}. Required: {}",
                            failure_class, reason, required_action
                        ),
                    );
                    return Ok(true); // Continue loop - planner must read first
                }
            }

            // Capture snapshot before mutation (for revert)
            if is_existing && let Ok(content) = std::fs::read_to_string(&path) {
                self.state.capture_snapshot(&path, &content);
                self.state.capture_snapshot(&normalized_path, &content);
                self.log(
                    LogSeverity::Debug,
                    "SNAPSHOT",
                    "SNAPSHOT_CAPTURED",
                    &format!("Captured snapshot for {}", path.display()),
                );
            }
        }

        // Stage 5: Tool execution
        self.log_tool_event(
            LogSeverity::Info,
            "EXECUTE",
            "TOOL_EXECUTE",
            &format!("Executing {}", tool_call.name),
            tool_call.name.as_str(),
        );

        let ctx = ExecutionContext {
            session_id: self.state.session_id.clone(),
            iteration: self.state.iteration,
            mode: self.config.mode,
            working_dir: PathBuf::from("."),
        };

        let tool_result = self.executor.execute(tool_call, &ctx)?;

        if !tool_result.success {
            let error_msg = tool_result
                .error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string());
            if tool_call.name.as_str() == "apply_patch" {
                if let Some(fallback_result) =
                    self.try_small_file_mutation_fallback(tool_call, &error_msg)?
                {
                    self.log_tool_event(
                        LogSeverity::Warning,
                        "EXECUTE",
                        "SMALL_FILE_FALLBACK_APPLIED",
                        "Malformed patch recovered through deterministic small-file fallback",
                        tool_call.name.as_str(),
                    );
                    self.record_tool_execution(
                        "apply_patch",
                        true,
                        "deterministic small-file fallback applied",
                    );
                    self.consecutive_read_only_tools = 0;
                    self.recent_read_only_signatures.clear();
                    self.no_progress_corrections = 0;
                    self.post_correction_progress_required = false;
                    self.post_correction_read_only_tools = 0;
                    return self.validate_and_commit_mutations(&fallback_result.mutations);
                }
            }
            self.log_tool_event(
                LogSeverity::Warning,
                "EXECUTE",
                "TOOL_FAILED",
                &format!("Tool failed: {}", error_msg),
                tool_call.name.as_str(),
            );
            self.record_tool_execution(tool_call.name.as_str(), false, error_msg.clone());
            // Track known errors for completion gate
            self.known_errors.push(error_msg.clone());

            return self.record_failed_tool_progress_policy(tool_call, &error_msg);
        }
        let success_message = format!(
            "Tool succeeded: {} ({} ms)",
            tool_result.output.as_ref().unwrap_or(&"OK".to_string()),
            tool_result.execution_time_ms
        );
        self.log_tool_event(
            LogSeverity::Info,
            "EXECUTE",
            "TOOL_SUCCESS",
            &success_message,
            tool_call.name.as_str(),
        );
        self.record_tool_execution(tool_call.name.as_str(), true, success_message.clone());
        self.reset_failed_tool_policy();

        // Stage 6: Handle read_file results with path normalization
        if tool_call.name.as_str() == "read_file"
            && tool_result.success
            && let Some(path_str) = tool_call.arguments.get("path")
        {
            let path = PathBuf::from(path_str);
            let normalized_path = normalize_path(&path);

            if let Ok(content) = std::fs::read_to_string(&path) {
                let offset = tool_call
                    .arguments
                    .get("offset")
                    .and_then(|s| s.parse::<usize>().ok());
                let limit = tool_call
                    .arguments
                    .get("limit")
                    .and_then(|s| s.parse::<usize>().ok());
                let file_record = self.build_file_record_from_read(
                    &path,
                    &content,
                    self.state.iteration,
                    true,
                    offset,
                    limit,
                );
                self.state.record_file_read(file_record);
                self.log(
                    LogSeverity::Info,
                    "TRACK",
                    "FILE_READ_TRACKED",
                    &format!(
                        "Recorded file read: {} (normalized)",
                        normalized_path.display()
                    ),
                );
            }
        }

        // Stage 6b: Mutation detection
        let mutations = &tool_result.mutations;
        if mutations.is_empty() {
            self.log(
                LogSeverity::Info,
                "MUTATION",
                "NO_MUTATIONS",
                "No mutations detected",
            );
            self.record_read_only_progress_policy(tool_call)?;
            return Ok(true); // Continue loop
        }

        self.consecutive_read_only_tools = 0;
        self.recent_read_only_signatures.clear();
        self.no_progress_corrections = 0;
        self.post_correction_progress_required = false;
        self.post_correction_read_only_tools = 0;
        self.validate_and_commit_mutations(mutations)
    }

    fn try_small_file_mutation_fallback(
        &mut self,
        tool_call: &ToolCall,
        tool_error: &str,
    ) -> Result<Option<ToolResult>, ForgeError> {
        if tool_call.name.as_str() != "apply_patch" {
            return Ok(None);
        }

        let path = self.extract_mutation_path(tool_call)?;
        let normalized_path = normalize_path(&path);
        let Some(read_record) = self.state.files_read.get(&normalized_path) else {
            return Ok(None);
        };
        if !read_record.is_full_read
            || read_record.size_bytes as usize > SMALL_FILE_FALLBACK_MAX_BYTES
        {
            return Ok(None);
        }

        let old_text = tool_call.arguments.require("old_text")?;
        let new_text = tool_call.arguments.require("new_text")?;
        if new_text.trim().is_empty() {
            return Ok(None);
        }

        let current_content = std::fs::read_to_string(&path).map_err(|e| {
            ForgeError::IoError(format!(
                "Failed to read {} for small-file fallback: {}",
                path.display(),
                e
            ))
        })?;
        let current_hash = crate::crypto_hash::compute_content_hash(&current_content);
        if current_hash != read_record.content_hash {
            return Ok(None);
        }
        if current_content.len() > SMALL_FILE_FALLBACK_MAX_BYTES {
            return Ok(None);
        }

        let replacement = if !old_text.is_empty() {
            let occurrences = current_content.matches(old_text).count();
            if occurrences != 1 {
                return Ok(None);
            }
            current_content.replacen(old_text, new_text, 1)
        } else {
            if current_content.contains(new_text) {
                return Ok(None);
            }
            let mut rewritten = current_content.clone();
            if !rewritten.is_empty() && !rewritten.ends_with('\n') {
                rewritten.push('\n');
            }
            rewritten.push_str(new_text);
            if !rewritten.ends_with('\n') {
                rewritten.push('\n');
            }
            rewritten
        };

        if replacement == current_content || replacement.len() > SMALL_FILE_FALLBACK_MAX_BYTES {
            return Ok(None);
        }

        write_file_atomically(&path, &replacement)?;
        let new_hash = crate::crypto_hash::compute_content_hash(&replacement);
        let mutation = Mutation {
            path: path.clone(),
            mutation_type: MutationType::Patch,
            content_hash_before: Some(current_hash.clone()),
            content_hash_after: Some(new_hash.clone()),
        };

        self.audit_governance_event(
            "small_file_mutation_fallback",
            "Accept",
            Some("patch_fallback".to_string()),
            format!(
                "Applied deterministic fallback to {} after tool failure: {}",
                path.display(),
                tool_error
            ),
        );

        Ok(Some(ToolResult {
            success: true,
            output: Some(format!(
                "SMALL_FILE_FALLBACK: {} {} -> {}",
                path.display(),
                &current_hash[..16.min(current_hash.len())],
                &new_hash[..16.min(new_hash.len())]
            )),
            error: None,
            mutations: vec![mutation],
            execution_time_ms: 0,
        }))
    }

    /// Extract mutation path from tool call arguments
    fn extract_mutation_path(&self, tool_call: &ToolCall) -> Result<PathBuf, ForgeError> {
        let path_str = match tool_call.name.as_str() {
            "write_file" => tool_call.arguments.get("path"),
            "apply_patch" => tool_call.arguments.get("file_path"),
            "delete_file" => tool_call.arguments.get("path"),
            _ => None,
        };

        path_str
            .map(PathBuf::from)
            .ok_or_else(|| ForgeError::MissingArgument("path/file_path".to_string()))
    }
}

/// Run the bootstrap runtime with the given configuration.
pub fn run_bootstrap(config: RuntimeConfig) -> RuntimeResult {
    match Runtime::new(config) {
        Ok(mut runtime) => runtime.run(),
        Err(e) => RuntimeResult {
            success: false,
            final_state: AgentState::new(0, String::new(), ExecutionMode::Edit),
            iterations: 0,
            logs: vec![],
            jsonl_logs: vec![],
            error: Some(e.to_string()),
        },
    }
}

fn timestamp_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn structured_output_only() -> bool {
    matches!(
        std::env::var("FORGE_OUTPUT_MODE").ok().as_deref(),
        Some("jsonl") | Some("JSONL")
    )
}

fn extract_observed_read_content(
    content: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> String {
    if offset.is_none() && limit.is_none() {
        return content.to_string();
    }

    let start = offset.unwrap_or(1).max(1).saturating_sub(1);
    let effective_limit = limit
        .unwrap_or(MAX_TRACKED_READ_LINES)
        .min(MAX_TRACKED_READ_LINES);
    let lines: Vec<&str> = content.lines().collect();

    if start >= lines.len() {
        return String::new();
    }

    lines
        .iter()
        .skip(start)
        .take(effective_limit)
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

fn mutation_kind_name(mutation_type: MutationType) -> &'static str {
    match mutation_type {
        MutationType::Write => "write",
        MutationType::Patch => "patch",
        MutationType::Delete => "delete",
        MutationType::Move => "move",
    }
}

fn write_file_atomically(path: &Path, content: &str) -> Result<(), ForgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ForgeError::IoError(format!(
                "Failed to create parent directory for {}: {}",
                path.display(),
                e
            ))
        })?;
    }
    let temp_path = path.with_extension("tmp");
    std::fs::write(&temp_path, content).map_err(|e| {
        ForgeError::IoError(format!(
            "Failed to write temp file {}: {}",
            temp_path.display(),
            e
        ))
    })?;
    std::fs::rename(&temp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        ForgeError::IoError(format!(
            "Failed to commit temp file {}: {}",
            path.display(),
            e
        ))
    })
}

fn normalize_tool_error(error: &str) -> String {
    let mut normalized = error.to_lowercase();
    if let Some(index) = normalized.find("/users/") {
        normalized.truncate(index);
    }
    normalized
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(16)
        .collect::<Vec<_>>()
        .join("_")
}

fn classify_tool_failure(tool_name: &str, error: &str) -> &'static str {
    let lower = error.to_lowercase();
    if tool_name == "read_file" && (lower.contains("no such file") || lower.contains("not found")) {
        "missing_file_read"
    } else if tool_name == "test_runner" && lower.contains("test failed") {
        "repeated_test_failure"
    } else if lower.contains("hash mismatch") || lower.contains("stale") {
        "stale_mutation_context"
    } else {
        "repeated_tool_failure"
    }
}

fn corrective_prompt_for_tool_failure(tool_name: &str, error: &str) -> &'static str {
    match classify_tool_failure(tool_name, error) {
        "missing_file_read" => {
            "Stop reading the missing path. Use list_dir or grep_search to find the real file, or create the requested file if the objective requires creation."
        }
        "repeated_test_failure" => {
            "Do not run test_runner again until after a code mutation. Read the failing implementation and tests, then patch or write the fix."
        }
        "stale_mutation_context" => {
            "Re-read the target file to refresh the content hash before attempting another mutation."
        }
        _ => {
            "Do not repeat the same failing tool call. Gather different evidence, choose a different tool, mutate with read-before-write authority, or emit an explicit recoverable failure."
        }
    }
}

#[derive(Debug, Clone)]
struct RustCompanionRecoveryAction {
    target_path: PathBuf,
    edit_class: &'static str,
    evidence_class: &'static str,
    updated_content: String,
}

fn rust_companion_recovery_action(
    reason: &str,
    mutations: &[Mutation],
) -> Result<Option<RustCompanionRecoveryAction>, ForgeError> {
    let reason_lower = reason.to_lowercase();
    if reason_lower.contains("unresolved import") || reason_lower.contains("could not find") {
        if let Some(action) = rust_module_exposure_recovery(reason, mutations)? {
            return Ok(Some(action));
        }
    }

    if reason_lower.contains("private") {
        if let Some(action) = rust_public_api_recovery(reason)? {
            return Ok(Some(action));
        }
    }

    Ok(None)
}

fn rust_module_exposure_recovery(
    reason: &str,
    mutations: &[Mutation],
) -> Result<Option<RustCompanionRecoveryAction>, ForgeError> {
    let crate_root = rust_crate_root_for_mutations(mutations)?;
    let reason_lower = reason.to_lowercase();

    for mutation in mutations {
        let Some(relative_source_path) =
            rust_source_path_relative_to_src(&crate_root, &mutation.path)
        else {
            continue;
        };
        if relative_source_path
            .file_name()
            .is_some_and(|name| name == "mod.rs")
        {
            continue;
        }

        let Some(module_name) = relative_source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
        else {
            continue;
        };
        if !rust_identifier_is_safe(module_name) || !reason_lower.contains(module_name) {
            continue;
        }

        let parent_target = rust_parent_module_target(&crate_root, &relative_source_path);
        if !parent_target.exists() {
            continue;
        }

        let current = std::fs::read_to_string(&parent_target).map_err(|e| {
            ForgeError::IoError(format!(
                "Failed to read Rust module parent {}: {}",
                parent_target.display(),
                e
            ))
        })?;
        if rust_module_declared(&current, module_name) {
            continue;
        }

        let updated = append_rust_module_declaration(&current, module_name);
        if updated == current {
            continue;
        }

        return Ok(Some(RustCompanionRecoveryAction {
            target_path: parent_target,
            edit_class: "module_exposure",
            evidence_class: "rust_unresolved_module_surface",
            updated_content: updated,
        }));
    }

    Ok(None)
}

fn rust_public_api_recovery(
    reason: &str,
) -> Result<Option<RustCompanionRecoveryAction>, ForgeError> {
    let Some(symbol) = rust_private_symbol_from_evidence(reason) else {
        return Ok(None);
    };
    let Some(target_path) = rust_evidence_source_path(reason) else {
        return Ok(None);
    };
    if !target_path.exists() || target_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        return Ok(None);
    }

    let current = std::fs::read_to_string(&target_path).map_err(|e| {
        ForgeError::IoError(format!(
            "Failed to read Rust API target {}: {}",
            target_path.display(),
            e
        ))
    })?;
    let Some(updated) = make_rust_type_public(&current, &symbol) else {
        return Ok(None);
    };
    if updated == current {
        return Ok(None);
    }

    Ok(Some(RustCompanionRecoveryAction {
        target_path,
        edit_class: "public_api_visibility",
        evidence_class: "rust_private_public_api_surface",
        updated_content: updated,
    }))
}

fn rust_crate_root_for_mutations(mutations: &[Mutation]) -> Result<PathBuf, ForgeError> {
    for mutation in mutations {
        let search_start = if mutation.path.is_absolute() {
            mutation.path.parent().map(Path::to_path_buf)
        } else {
            std::env::current_dir()
                .ok()
                .map(|cwd| cwd.join(&mutation.path))
                .and_then(|path| path.parent().map(Path::to_path_buf))
        };
        let Some(mut cursor) = search_start else {
            continue;
        };

        loop {
            if cursor.join("Cargo.toml").exists() {
                return Ok(cursor);
            }
            if !cursor.pop() {
                break;
            }
        }
    }

    std::env::current_dir()
        .map_err(|e| ForgeError::IoError(format!("Failed to determine current directory: {}", e)))
}

fn rust_source_path_relative_to_src(crate_root: &Path, path: &Path) -> Option<PathBuf> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        crate_root.join(path)
    };
    let src_root = crate_root.join("src");
    let relative = absolute_path.strip_prefix(src_root).ok()?;
    if relative.extension().and_then(|ext| ext.to_str()) == Some("rs") {
        Some(relative.to_path_buf())
    } else {
        None
    }
}

fn rust_parent_module_target(crate_root: &Path, relative_source_path: &Path) -> PathBuf {
    let src_root = crate_root.join("src");
    match relative_source_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            let direct_parent = src_root.join(parent).with_extension("rs");
            if direct_parent.exists() {
                direct_parent
            } else {
                src_root.join(parent).join("mod.rs")
            }
        }
        _ => src_root.join("lib.rs"),
    }
}

fn rust_module_declared(content: &str, module_name: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == format!("mod {};", module_name)
            || trimmed == format!("pub mod {};", module_name)
            || trimmed == format!("pub(crate) mod {};", module_name)
    })
}

fn append_rust_module_declaration(content: &str, module_name: &str) -> String {
    let declaration = format!("pub mod {};", module_name);
    let mut updated = content.trim_end().to_string();
    if !updated.is_empty() {
        updated.push_str("\n\n");
    }
    updated.push_str(&declaration);
    updated.push('\n');
    updated
}

fn rust_private_symbol_from_evidence(reason: &str) -> Option<String> {
    for quoted in backtick_segments(reason) {
        if rust_identifier_is_safe(&quoted) && !rust_keywords().contains(&quoted.as_str()) {
            return Some(quoted);
        }
    }
    None
}

fn rust_evidence_source_path(reason: &str) -> Option<PathBuf> {
    for line in reason.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("-->") else {
            continue;
        };
        let path_part = rest.trim().split(':').next()?.trim();
        if path_part.ends_with(".rs") {
            let path = PathBuf::from(path_part);
            return Some(if path.is_absolute() {
                path
            } else {
                std::env::current_dir().ok()?.join(path)
            });
        }
    }
    None
}

fn backtick_segments(input: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        segments.push(after_start[..end].to_string());
        rest = &after_start[end + 1..];
    }
    segments
}

fn make_rust_type_public(content: &str, symbol: &str) -> Option<String> {
    let struct_marker = format!("struct {}", symbol);
    let enum_marker = format!("enum {}", symbol);
    let trait_marker = format!("trait {}", symbol);
    if content.contains(&format!("pub {}", struct_marker))
        || content.contains(&format!("pub {}", enum_marker))
        || content.contains(&format!("pub {}", trait_marker))
    {
        return None;
    }

    if content.contains(&struct_marker) {
        return Some(make_struct_and_named_fields_public(content, symbol));
    }
    if content.contains(&enum_marker) {
        return Some(content.replacen(&enum_marker, &format!("pub {}", enum_marker), 1));
    }
    if content.contains(&trait_marker) {
        return Some(content.replacen(&trait_marker, &format!("pub {}", trait_marker), 1));
    }
    None
}

fn make_struct_and_named_fields_public(content: &str, symbol: &str) -> String {
    let marker = format!("struct {}", symbol);
    let updated = content.replacen(&marker, &format!("pub {}", marker), 1);
    let mut lines = Vec::new();
    let mut inside_target_struct = false;
    let mut brace_depth = 0usize;

    for line in updated.lines() {
        let trimmed = line.trim_start();
        if !inside_target_struct && trimmed.starts_with(&format!("pub struct {}", symbol)) {
            inside_target_struct = true;
        } else if inside_target_struct
            && brace_depth == 1
            && rust_named_field_line_needs_pub(trimmed)
        {
            let indent_len = line.len() - trimmed.len();
            let indent = &line[..indent_len];
            lines.push(format!("{}pub {}", indent, trimmed));
            brace_depth = brace_depth
                .saturating_add(line.matches('{').count())
                .saturating_sub(line.matches('}').count());
            if brace_depth == 0 {
                inside_target_struct = false;
            }
            continue;
        }

        if inside_target_struct {
            brace_depth = brace_depth
                .saturating_add(line.matches('{').count())
                .saturating_sub(line.matches('}').count());
            if brace_depth == 0 && line.contains('}') {
                inside_target_struct = false;
            }
        }
        lines.push(line.to_string());
    }

    if updated.ends_with('\n') {
        lines.join("\n") + "\n"
    } else {
        lines.join("\n")
    }
}

fn rust_named_field_line_needs_pub(trimmed: &str) -> bool {
    let Some((name, _)) = trimmed.split_once(':') else {
        return false;
    };
    let name = name.trim();
    !name.starts_with("pub ") && rust_identifier_is_safe(name)
}

fn rust_identifier_is_safe(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn rust_keywords() -> &'static [&'static str] {
    &[
        "as", "crate", "enum", "fn", "impl", "mod", "pub", "self", "struct", "super", "trait",
        "type", "use", "where",
    ]
}

fn should_defer_multi_file_validation(
    task: &str,
    reason: &str,
    mutations: &[Mutation],
    prior_validation_failures: u32,
) -> bool {
    if prior_validation_failures >= MAX_LIVE_VALIDATION_RECOVERY_ATTEMPTS {
        return false;
    }

    let task_lower = task.to_lowercase();
    let multi_file_task = task_lower.contains("multi-file")
        || task_lower.contains("expose")
        || task_lower.contains("module")
        || task_lower.contains("end-to-end")
        || task_lower.contains("whole crate")
        || task_lower.contains("tests/")
        || task_lower.contains("cargo test");
    if !multi_file_task {
        return false;
    }

    let distinct_paths = mutations
        .iter()
        .map(|mutation| normalize_path(&mutation.path))
        .collect::<std::collections::HashSet<_>>()
        .len();
    if distinct_paths > 3 {
        return false;
    }

    let reason_lower = reason.to_lowercase();
    reason_lower.contains("file not found for module")
        || reason_lower.contains("could not find")
        || reason_lower.contains("unresolved import")
        || reason_lower.contains("private")
        || reason_lower.contains("test failed")
}

fn format_mutation_paths(mutations: &[Mutation]) -> String {
    if mutations.is_empty() {
        return "(none)".to_string();
    }
    mutations
        .iter()
        .map(|mutation| mutation.path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn multi_file_recovery_hint(reason: &str, mutations: &[Mutation]) -> &'static str {
    let lower = reason.to_lowercase();
    let pending_paths = mutations
        .iter()
        .map(|mutation| mutation.path.to_string_lossy().to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    if lower.contains("unresolved import")
        && (lower.contains("::math") || pending_paths.contains("math.rs"))
    {
        "The pending module file already exists; next read src/lib.rs and expose it with `pub mod math;` instead of rewriting src/math.rs."
    } else if lower.contains("file not found for module") {
        "The compiler named the missing module file; create that module file, then validate the module exposure."
    } else if lower.contains("private")
        && (lower.contains("setting") || pending_paths.contains("lib.rs"))
    {
        "The public API exposes a private type; next read src/lib.rs and make the exposed struct and fields public."
    } else if lower.contains("format failed") {
        "The pending content needs rustfmt-compatible formatting; next read the pending file and rewrite only formatted Rust."
    } else {
        "Use the validation evidence to choose a different companion surface, not the same pending write."
    }
}

fn same_snapshot_target(candidate: &PathBuf, target: &PathBuf) -> bool {
    if candidate == target || normalize_path(candidate) == normalize_path(target) {
        return true;
    }

    match (
        candidate.file_name(),
        target.file_name(),
        candidate.parent(),
        target.parent(),
    ) {
        (Some(candidate_name), Some(target_name), Some(candidate_parent), Some(target_parent))
            if candidate_name == target_name =>
        {
            match (
                std::fs::canonicalize(candidate_parent),
                std::fs::canonicalize(target_parent),
            ) {
                (Ok(candidate_parent), Ok(target_parent)) => candidate_parent == target_parent,
                _ => false,
            }
        }
        _ => false,
    }
}

fn find_upward_marker(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };

    loop {
        if current.join(marker).exists() {
            return Some(current);
        }

        if !current.pop() {
            return None;
        }
    }
}

fn collect_rust_files(root: &Path) -> Result<Vec<PathBuf>, ForgeError> {
    fn visit(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), ForgeError> {
        let entries = std::fs::read_dir(path).map_err(|e| {
            ForgeError::IoError(format!("Failed to enumerate {}: {}", path.display(), e))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                ForgeError::IoError(format!("Failed to read directory entry: {}", e))
            })?;
            let entry_path = entry.path();
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if entry_path.is_dir() {
                if matches!(file_name.as_ref(), "target" | ".git") {
                    continue;
                }
                visit(&entry_path, files)?;
            } else if entry_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
            {
                files.push(entry_path);
            }
        }

        Ok(())
    }

    let mut files = Vec::new();
    visit(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn restore_changed_files(before: &[(PathBuf, String, String)]) -> Result<(), ForgeError> {
    for (path, content, hash_before) in before {
        let Ok(current) = std::fs::read_to_string(path) else {
            continue;
        };
        let hash_current = crate::crypto_hash::compute_content_hash(&current);
        if &hash_current != hash_before {
            write_file_atomically(path, content)?;
        }
    }
    Ok(())
}

fn refresh_mutation_hashes(mutations: &mut [Mutation]) {
    for mutation in mutations {
        if matches!(
            mutation.mutation_type,
            MutationType::Write | MutationType::Patch
        ) && let Ok(content) = std::fs::read_to_string(&mutation.path)
        {
            mutation.content_hash_after = Some(crate::crypto_hash::compute_content_hash(&content));
        }
    }
}

fn deterministic_prefix_refactor(content: &str) -> Option<String> {
    if content.contains("fn format_with_prefix(") {
        return None;
    }

    let prefixes = extract_prefix_format_literals(content);
    if prefixes.len() < 2 {
        return None;
    }

    let mut updated = content.to_string();
    for prefix in &prefixes {
        let from = format!("format!(\"{}:{{}}\", name)", prefix);
        let to = format!("format_with_prefix(\"{}\", name)", prefix);
        updated = updated.replace(&from, &to);
    }

    if updated == content {
        return None;
    }

    let helper = "fn format_with_prefix(prefix: &str, name: &str) -> String {\n    format!(\"{}:{}\", prefix, name)\n}\n\n";
    let insert_at = updated.find("pub fn").unwrap_or(0);
    updated.insert_str(insert_at, helper);
    if !updated.ends_with('\n') {
        updated.push('\n');
    }

    Some(updated)
}

fn extract_prefix_format_literals(content: &str) -> Vec<String> {
    let mut prefixes = Vec::new();
    let mut offset = 0;
    let marker = "format!(\"";
    let suffix = ":{}\", name)";

    while let Some(start) = content[offset..].find(marker) {
        let literal_start = offset + start + marker.len();
        let Some(end) = content[literal_start..].find(suffix) else {
            offset = literal_start;
            continue;
        };
        let prefix = &content[literal_start..literal_start + end];
        if !prefix.is_empty()
            && prefix
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            && !prefixes.iter().any(|known| known == prefix)
        {
            prefixes.push(prefix.to_string());
        }
        offset = literal_start + end + suffix.len();
    }

    prefixes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::state_view::{StateView, ToolExecutionRecord};
    use crate::types::{
        ExecutionMode, FileRecord, SessionStatus, ToolArguments, ValidationReport, ValidationStage,
        ValidationStageResult,
    };
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    fn cwd_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn accepted_syntax_validation() -> ValidationReport {
        let mut report = ValidationReport::accept("accepted");
        report.stage_results.push(ValidationStageResult {
            stage: ValidationStage::Syntax,
            passed: true,
            message: "Validated syntax for code artifact(s)".to_string(),
            execution_time_ms: 0,
        });
        report
    }

    fn accepted_full_validation() -> ValidationReport {
        let mut report = accepted_syntax_validation();
        report.stage_results.push(ValidationStageResult {
            stage: ValidationStage::Test,
            passed: true,
            message: "cargo test passed".to_string(),
            execution_time_ms: 0,
        });
        report
    }

    struct CurrentDirGuard {
        original: PathBuf,
    }

    impl CurrentDirGuard {
        fn change_to(path: &std::path::Path) -> Self {
            let original = env::current_dir().expect("current dir");
            env::set_current_dir(path).expect("set current dir");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.original);
        }
    }

    #[test]
    fn runtime_registered_tools_match_validation_context() {
        let tool_names = registered_tool_names();
        assert_eq!(
            tool_names,
            vec![
                "write_file".to_string(),
                "read_file".to_string(),
                "apply_patch".to_string(),
                "list_dir".to_string(),
                "grep_search".to_string(),
                "dependency_graph".to_string(),
                "symbol_index".to_string(),
                "entrypoint_detector".to_string(),
                "lint_runner".to_string(),
                "test_runner".to_string(),
            ]
        );

        let state = AgentState::new(4, "test task".to_string(), ExecutionMode::Edit);
        let state_view = StateView::from_agent_state(
            &state,
            registered_tool_infos(),
            Vec::<ToolExecutionRecord>::new(),
        );
        let state_tool_names: Vec<String> = state_view
            .available_tools
            .iter()
            .map(|tool| tool.name.as_str().to_string())
            .collect();

        assert_eq!(state_tool_names, tool_names);
        assert!(!tool_names.iter().any(|name| name == "execute_command"));
    }

    #[test]
    fn tracked_partial_reads_store_observed_excerpt() {
        let runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");

        let content = (1..=10)
            .map(|n| format!("line {}", n))
            .collect::<Vec<_>>()
            .join("\n");
        let record = runtime.build_file_record_from_read(
            &PathBuf::from("src/lib.rs"),
            &content,
            1,
            true,
            Some(3),
            Some(4),
        );

        assert_eq!(record.lines_read, Some((3, 4)));
        assert_eq!(
            record.content.as_deref(),
            Some("line 3\nline 4\nline 5\nline 6")
        );
        assert_eq!(record.total_lines, 10);
    }

    #[test]
    fn revert_restores_snapshot_after_patch_failure() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        let file_path = temp.path().join("broken.py");
        fs::write(&file_path, "print('ok')\n").expect("write initial file");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            auto_revert: true,
            ..RuntimeConfig::default()
        })
        .expect("runtime");

        runtime.state.capture_snapshot(&file_path, "print('ok')\n");
        runtime
            .state
            .capture_snapshot(&normalize_path(&file_path), "print('ok')\n");
        fs::remove_file(&file_path).expect("remove file to force validation failure");

        let mutations = vec![Mutation {
            path: file_path.clone(),
            mutation_type: MutationType::Patch,
            content_hash_before: None,
            content_hash_after: None,
        }];

        let result = runtime.validate_and_commit_mutations(&mutations);
        assert_eq!(result.expect("validation reject enters recovery"), true);
        assert_eq!(
            fs::read_to_string(&file_path).expect("restored file"),
            "print('ok')\n"
        );
        assert!(
            runtime
                .state
                .get_snapshot(&normalize_path(&file_path))
                .is_none()
        );
        assert!(
            runtime
                .known_errors
                .iter()
                .any(|error| error.contains("Validation failed"))
        );
    }

    #[test]
    fn runtime_completes_after_valid_slice_before_cosmetic_patch() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        let file_path = temp.path().join("cli.py");
        fs::write(
            &file_path,
            "import argparse\n\n\
             def main():\n\
                 parser = argparse.ArgumentParser()\n\
                 parser.add_argument('--name')\n\
                 args = parser.parse_args()\n\
                 print(args.name)\n\n\
             if __name__ == \"__main__\":\n\
                 main()\n",
        )
        .expect("write cli");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "build a Python CLI app".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.state.iteration = 1;
        runtime.state.files_written.insert(file_path.clone());
        runtime.state.last_validation_report = Some(accepted_syntax_validation());

        let mut args = ToolArguments::new();
        args.set("file_path", &file_path.to_string_lossy());
        args.set("old_text", "parser.add_argument('--name')");
        args.set("new_text", "parser.add_argument('--name', help='Name')");
        args.set("expected_hash", "sha256:any");
        let cosmetic_patch = ToolCall {
            name: ToolName::new("apply_patch").expect("tool name"),
            arguments: args,
        };

        let should_continue = runtime
            .execute_tool_call_hardened(&cosmetic_patch)
            .expect("execute hardened");

        assert!(!should_continue);
        assert_eq!(runtime.state.status, SessionStatus::Complete);
        assert!(
            runtime
                .logs
                .iter()
                .any(|entry| entry.message.contains("Skipped apply_patch"))
        );
    }

    #[test]
    fn runtime_does_not_complete_when_complex_goal_missing_surface() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        let file_path = temp.path().join("cli.py");
        fs::write(
            &file_path,
            "def main():\n    print('hello')\n\nif __name__ == \"__main__\":\n    main()\n",
        )
        .expect("write cli");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "build a Python CLI app".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.state.iteration = 1;
        runtime.state.files_written.insert(file_path);
        runtime.state.last_validation_report = Some(accepted_syntax_validation());

        let completed = runtime
            .complete_if_goal_ready("TEST_READY_CHECK")
            .expect("ready check");

        assert!(!completed);
        assert_eq!(runtime.state.status, SessionStatus::Running);
    }

    #[test]
    fn runtime_does_not_complete_when_validation_failed() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        let file_path = temp.path().join("cli.py");
        fs::write(
            &file_path,
            "import argparse\n\n\
             def main():\n\
                 parser = argparse.ArgumentParser()\n\
                 parser.add_argument('--name')\n\
                 args = parser.parse_args()\n\
                 print(args.name)\n\n\
             if __name__ == \"__main__\":\n\
                 main()\n",
        )
        .expect("write cli");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "build a Python CLI app".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.state.iteration = 1;
        runtime.state.files_written.insert(file_path);
        runtime.state.last_validation_report = Some(ValidationReport::reject("syntax failed"));
        runtime
            .known_errors
            .push("Validation failed: syntax failed".to_string());

        let completed = runtime
            .complete_if_goal_ready("TEST_READY_CHECK")
            .expect("ready check");

        assert!(!completed);
        assert_eq!(runtime.state.status, SessionStatus::Running);
    }

    #[test]
    fn read_only_churn_forces_audited_correction_before_budget_exhaustion() {
        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");

        let mut args = ToolArguments::new();
        args.set("path", "src/lib.rs");
        let read_call = ToolCall {
            name: ToolName::new("read_file").expect("tool name"),
            arguments: args,
        };

        assert!(
            runtime
                .record_read_only_progress_policy(&read_call)
                .expect("first read")
        );
        assert!(
            runtime
                .record_read_only_progress_policy(&read_call)
                .expect("second read")
        );
        assert!(
            runtime
                .record_read_only_progress_policy(&read_call)
                .expect("third read")
        );

        assert_eq!(
            runtime.consecutive_read_only_tools,
            READ_ONLY_CHURN_THRESHOLD
        );
        assert!(runtime.repair_prompt.is_some());
        assert!(runtime.logs.iter().any(|entry| {
            entry
                .message
                .contains("Planner correction forced toward mutation")
        }));
    }

    #[test]
    fn deterministic_small_file_fallback_appends_when_patch_shape_is_malformed() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        let file_path = temp.path().join("src/lib.rs");
        let original = "pub fn existing() -> &'static str {\n    \"ok\"\n}\n";
        fs::write(&file_path, original).expect("write lib");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        let record = runtime.build_file_record_from_read(
            &PathBuf::from("src/lib.rs"),
            original,
            1,
            true,
            None,
            None,
        );
        runtime.state.record_file_read(record);

        let mut args = ToolArguments::new();
        args.set("file_path", "src/lib.rs");
        args.set("old_text", "");
        args.set("new_text", "pub fn double(n: i32) -> i32 {\n    n * 2\n}");
        args.set("expected_hash", "sha256:bad");
        let patch_call = ToolCall {
            name: ToolName::new("apply_patch").expect("tool name"),
            arguments: args,
        };

        let fallback = runtime
            .try_small_file_mutation_fallback(&patch_call, "hash mismatch")
            .expect("fallback")
            .expect("fallback applied");

        let updated = fs::read_to_string(&file_path).expect("read updated");
        assert!(updated.contains("pub fn double"));
        assert_eq!(fallback.mutations.len(), 1);
        assert!(
            runtime.logs.iter().any(|entry| {
                entry.message.contains("Applied deterministic fallback")
                    || entry.stage == "NO_PROGRESS"
            }) || !runtime.governance.export_audit_logs().is_empty()
        );
    }

    #[test]
    fn post_correction_read_only_churn_halts_explicitly() {
        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.post_correction_progress_required = true;

        let mut args = ToolArguments::new();
        args.set("path", "src/lib.rs");
        let read_call = ToolCall {
            name: ToolName::new("read_file").expect("tool name"),
            arguments: args,
        };

        assert!(
            runtime
                .record_read_only_progress_policy(&read_call)
                .expect("first read after correction")
        );
        let result = runtime.record_read_only_progress_policy(&read_call);

        assert!(matches!(result, Err(ForgeError::ValidationFailed(_))));
        assert!(runtime.logs.iter().any(|entry| {
            entry.stage == "NO_PROGRESS"
                && entry
                    .message
                    .contains("Planner output was normalized, but execution returned")
        }));
        assert!(
            runtime
                .governance
                .export_audit_logs()
                .contains("post_correction_read_only_churn")
        );
    }

    #[test]
    fn validation_reject_schedules_live_recovery_instead_of_dead_end() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\n\n[package]\nname = \"validation_recovery\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .expect("write cargo");
        let file_path = temp.path().join("src/lib.rs");
        fs::write(&file_path, "pub fn answer() -> i32 {\n    42\n}\n").expect("write original");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            auto_revert: true,
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime
            .state
            .capture_snapshot(&file_path, "pub fn answer() -> i32 {\n    42\n}\n");
        fs::write(&file_path, "pub fn answer() -> i32 {\n    42\n").expect("write broken");
        let mutation = Mutation {
            path: file_path.clone(),
            mutation_type: MutationType::Patch,
            content_hash_before: None,
            content_hash_after: None,
        };

        let should_continue = runtime
            .validate_and_commit_mutations(&[mutation])
            .expect("validation reject should enter recovery");

        assert!(should_continue);
        assert!(runtime.repair_prompt.is_some());
        assert_eq!(runtime.consecutive_validation_failures, 1);
        assert_eq!(
            fs::read_to_string(&file_path).expect("reverted"),
            "pub fn answer() -> i32 {\n    42\n}\n"
        );
        assert!(
            runtime
                .logs
                .iter()
                .any(|entry| entry.message.contains("Retry 1/2 scheduled"))
        );
    }

    #[test]
    fn multi_file_validation_defers_and_commits_combined_surface() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::create_dir_all(temp.path().join("tests")).expect("create tests");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\n\n[package]\nname = \"multi_surface\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .expect("write cargo");
        let lib_path = temp.path().join("src/lib.rs");
        let math_path = temp.path().join("src/math.rs");
        fs::write(
            &lib_path,
            "pub fn existing() -> &'static str {\n    \"ok\"\n}\n",
        )
        .expect("write lib");
        fs::write(
            temp.path().join("tests/feature_tests.rs"),
            "use multi_surface::math::{is_even, triple};\n\n#[test]\nfn feature_math_works() {\n    assert_eq!(triple(4), 12);\n    assert!(is_even(6));\n}\n",
        )
        .expect("write tests");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "Implement missing math module. Expose from src/lib.rs. Complete only after cargo test passes.".to_string(),
            auto_revert: true,
            ..RuntimeConfig::default()
        })
        .expect("runtime");

        fs::write(
            &lib_path,
            "pub fn existing() -> &'static str {\n    \"ok\"\n}\n\npub mod math;\n",
        )
        .expect("write pending lib");
        let lib_mutation = Mutation {
            path: lib_path.clone(),
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        };

        let should_continue = runtime
            .validate_and_commit_mutations(&[lib_mutation])
            .expect("defer first surface");

        assert!(should_continue);
        assert!(runtime.pending_validation);
        assert_eq!(runtime.deferred_validation_mutations.len(), 1);
        assert!(
            fs::read_to_string(&lib_path)
                .unwrap()
                .contains("pub mod math;")
        );
        assert!(runtime.logs.iter().any(|entry| {
            entry.stage == "RECOVERY" && entry.message.contains("multi-file recovery")
        }));

        fs::write(
            &math_path,
            "pub fn triple(x: i32) -> i32 {\n    x * 3\n}\n\npub fn is_even(x: i32) -> bool {\n    x % 2 == 0\n}\n",
        )
        .expect("write math");
        let math_mutation = Mutation {
            path: math_path.clone(),
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        };

        let should_continue = runtime
            .validate_and_commit_mutations(&[math_mutation])
            .expect("combined validation");

        assert!(!should_continue);
        assert_eq!(runtime.state.status, SessionStatus::Complete);
        assert!(runtime.deferred_validation_mutations.is_empty());
        assert!(
            runtime
                .state
                .files_written
                .iter()
                .any(|path| same_snapshot_target(path, &lib_path))
        );
        assert!(
            runtime
                .state
                .files_written
                .iter()
                .any(|path| same_snapshot_target(path, &math_path))
        );
    }

    #[test]
    fn validation_pass_path_finalizes_validation_bound_task() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        let file_path = temp.path().join("src/lib.rs");
        fs::create_dir_all(file_path.parent().unwrap()).expect("create src");
        fs::write(&file_path, "pub fn double(n: i32) -> i32 { n * 2 }\n").expect("write lib");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "Add double and complete only after cargo test passes.".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.state.iteration = 1;
        runtime.state.files_written.insert(file_path);
        runtime.state.last_validation_report = Some(accepted_full_validation());

        let completed = runtime
            .complete_if_goal_ready("TEST_VALIDATION_FINALIZE")
            .expect("completion check");

        assert!(completed);
        assert_eq!(runtime.state.status, SessionStatus::Complete);
        assert!(
            runtime
                .state
                .completion_reason
                .as_ref()
                .is_some_and(|reason| reason.as_str().contains("cargo test passed"))
        );
    }

    #[test]
    fn repeated_missing_file_read_gets_bounded_corrective_recovery() {
        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        let mut args = ToolArguments::new();
        args.set("path", "src/main.rs");
        let read_call = ToolCall {
            name: ToolName::new("read_file").expect("tool name"),
            arguments: args,
        };
        let error = "IO error: Failed to read 'src/main.rs': No such file or directory";

        assert!(
            runtime
                .record_failed_tool_progress_policy(&read_call, error)
                .expect("first failure")
        );
        assert!(
            runtime
                .record_failed_tool_progress_policy(&read_call, error)
                .expect("second failure")
        );
        assert!(
            runtime
                .record_failed_tool_progress_policy(&read_call, error)
                .expect("third failure")
        );
        assert!(
            runtime
                .repair_prompt
                .as_ref()
                .is_some_and(|prompt| { prompt.contains("Stop reading the missing path") })
        );

        let halted = runtime.record_failed_tool_progress_policy(&read_call, error);
        assert!(matches!(halted, Err(ForgeError::ValidationFailed(_))));
    }

    #[test]
    fn repeated_test_runner_failure_requires_mutation_before_rerun() {
        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        let test_call = ToolCall {
            name: ToolName::new("test_runner").expect("tool name"),
            arguments: ToolArguments::new(),
        };
        let error = "Execution failed: error: test failed, to rerun pass `--test bug_tests`";

        runtime
            .record_failed_tool_progress_policy(&test_call, error)
            .expect("first failure");
        runtime
            .record_failed_tool_progress_policy(&test_call, error)
            .expect("second failure");
        runtime
            .record_failed_tool_progress_policy(&test_call, error)
            .expect("third failure");

        assert!(runtime.repair_prompt.as_ref().is_some_and(|prompt| {
            prompt.contains("Do not run test_runner again until after a code mutation")
        }));
        assert!(
            runtime
                .logs
                .iter()
                .any(|entry| entry.message.contains("Failed-tool churn detected"))
        );
    }

    #[test]
    fn rust_format_recovery_formats_and_commits_pending_mutation() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"format_recovery\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write cargo");
        let lib_path = temp.path().join("src/lib.rs");
        fs::write(&lib_path, "pub fn answer()->i32{42}").expect("write unformatted lib");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        let mutation = Mutation {
            path: lib_path.clone(),
            mutation_type: MutationType::Patch,
            content_hash_before: None,
            content_hash_after: None,
        };

        let continue_running = runtime
            .validate_and_commit_mutations(&[mutation])
            .expect("format recovery validation");

        assert!(continue_running);
        assert!(
            fs::read_to_string(&lib_path)
                .expect("read formatted lib")
                .contains("pub fn answer() -> i32")
        );
        assert!(
            runtime
                .governance
                .export_audit_logs()
                .contains("rust_format_recovery")
        );
    }

    #[test]
    fn validation_pass_after_rust_format_recovery_finalizes_compile_repair() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"compile_repair_finalize\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write cargo");
        let lib_path = temp.path().join("src/lib.rs");
        fs::write(
            &lib_path,
            "pub fn answer()->i32{42}\n\n#[cfg(test)]\nmod tests{use super::*;#[test]fn answer_is_42(){assert_eq!(answer(),42);}}\n",
        )
        .expect("write unformatted fixed lib");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "This project has a compile error. Read src/lib.rs, repair the compile error, and complete only after cargo check and cargo test pass.".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.state.iteration = 3;
        let mutation = Mutation {
            path: lib_path.clone(),
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        };

        let continue_running = runtime
            .validate_and_commit_mutations(&[mutation])
            .expect("format recovery validation");

        assert!(!continue_running);
        assert_eq!(runtime.state.status, SessionStatus::Complete);
        assert!(
            runtime
                .state
                .completion_reason
                .as_ref()
                .is_some_and(|reason| reason
                    .as_str()
                    .contains("cargo check and cargo test passed"))
        );
        assert!(runtime.logs.iter().any(|entry| {
            entry.stage == "COMPLETION"
                && entry.message.contains("Post-success completion accepted")
        }));
    }

    #[test]
    fn refactor_read_only_churn_recovery_applies_prefix_helper() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::create_dir_all(temp.path().join("tests")).expect("create tests");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"refactor_recovery\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write cargo");
        let lib_path = temp.path().join("src/lib.rs");
        let original = "pub fn label_user(name: &str) -> String {\n    format!(\"user:{}\", name)\n}\n\npub fn label_team(name: &str) -> String {\n    format!(\"team:{}\", name)\n}\n";
        fs::write(&lib_path, original).expect("write lib");
        fs::write(
            temp.path().join("tests/refactor_tests.rs"),
            "use refactor_recovery::{label_team, label_user};\n\n#[test]\nfn labels_are_stable() {\n    assert_eq!(label_user(\"ada\"), \"user:ada\");\n    assert_eq!(label_team(\"core\"), \"team:core\");\n}\n",
        )
        .expect("write tests");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "Refactor the repeated prefix formatting logic into a private helper without changing behavior.".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.state.record_file_read(FileRecord::new(
            normalize_path(&lib_path),
            original,
            None,
            0,
        ));
        runtime.consecutive_read_only_tools = READ_ONLY_CHURN_HALT_THRESHOLD;
        runtime.no_progress_corrections = 1;

        let applied = runtime
            .try_refactor_read_only_churn_recovery("read-only churn")
            .expect("refactor fallback")
            .expect("fallback applied");

        assert!(applied);
        let updated = fs::read_to_string(&lib_path).expect("read updated");
        assert!(updated.contains("fn format_with_prefix"));
        assert!(updated.contains("format_with_prefix(\"user\", name)"));
        assert!(updated.contains("format_with_prefix(\"team\", name)"));
        assert!(
            runtime
                .governance
                .export_audit_logs()
                .contains("refactor_read_only_churn_recovery")
        );
    }

    #[test]
    fn multi_file_recovery_hint_redirects_unresolved_module_to_lib_exposure() {
        let mutation = Mutation {
            path: PathBuf::from("src/math.rs"),
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        };

        let hint = multi_file_recovery_hint(
            "error[E0432]: unresolved import `bench_feature::math`",
            &[mutation],
        );

        assert!(hint.contains("src/lib.rs"));
        assert!(hint.contains("pub mod math"));
        assert!(hint.contains("instead of rewriting src/math.rs"));
    }

    #[test]
    fn rust_companion_recovery_adds_pub_mod_for_pending_root_module() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"companion_root\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write cargo");
        let lib_path = temp.path().join("src/lib.rs");
        let math_path = temp.path().join("src/math.rs");
        fs::write(
            &lib_path,
            "pub fn existing() -> &'static str {\n    \"ok\"\n}\n",
        )
        .expect("write lib");
        fs::write(&math_path, "pub fn triple(x: i32) -> i32 {\n    x * 3\n}\n")
            .expect("write math");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        let mutation = Mutation {
            path: math_path,
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        };

        let recovery = runtime
            .try_rust_companion_surface_recovery(
                "error[E0432]: unresolved import `companion_root::math`",
                &[mutation],
            )
            .expect("recovery attempt")
            .expect("recovery mutation");

        assert!(same_snapshot_target(&recovery.path, &lib_path));
        assert!(
            fs::read_to_string(temp.path().join("src/lib.rs"))
                .expect("read lib")
                .contains("pub mod math;")
        );
        assert!(runtime.logs.iter().any(|entry| {
            entry.stage == "RECOVERY" && entry.message.contains("Rust companion-surface recovery")
        }));
        assert!(
            runtime
                .governance
                .export_audit_logs()
                .contains("rust_companion_surface_recovery")
        );
    }

    #[test]
    fn rust_companion_recovery_adds_nested_parent_module() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src/foo")).expect("create nested src");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"companion_nested\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write cargo");
        fs::write(temp.path().join("src/lib.rs"), "pub mod foo;\n").expect("write lib");
        let foo_path = temp.path().join("src/foo.rs");
        let bar_path = temp.path().join("src/foo/bar.rs");
        fs::write(
            &foo_path,
            "pub fn foo() -> &'static str {\n    \"foo\"\n}\n",
        )
        .expect("write foo");
        fs::write(
            &bar_path,
            "pub fn bar() -> &'static str {\n    \"bar\"\n}\n",
        )
        .expect("write bar");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        let mutation = Mutation {
            path: bar_path,
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        };

        let recovery = runtime
            .try_rust_companion_surface_recovery(
                "error[E0432]: unresolved import `companion_nested::foo::bar`",
                &[mutation],
            )
            .expect("recovery attempt")
            .expect("recovery mutation");

        assert!(same_snapshot_target(&recovery.path, &foo_path));
        assert!(
            fs::read_to_string(temp.path().join("src/foo.rs"))
                .expect("read foo")
                .contains("pub mod bar;")
        );
    }

    #[test]
    fn rust_companion_recovery_makes_private_api_public() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        let lib_path = temp.path().join("src/lib.rs");
        fs::write(
            &lib_path,
            "struct Setting {\n    key: String,\n    value: String,\n}\n\npub fn parse_setting(input: &str) -> Setting {\n    Setting { key: input.to_string(), value: String::new() }\n}\n",
        )
        .expect("write lib");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");

        let recovery = runtime
            .try_rust_companion_surface_recovery(
                "warning: type `Setting` is more private than the item `parse_setting`\n  --> src/lib.rs:6:1",
                &[],
            )
            .expect("recovery attempt")
            .expect("recovery mutation");

        assert!(same_snapshot_target(&recovery.path, &lib_path));
        let updated = fs::read_to_string(temp.path().join("src/lib.rs")).expect("read lib");
        assert!(updated.contains("pub struct Setting"));
        assert!(updated.contains("pub key: String"));
        assert!(updated.contains("pub value: String"));
    }

    #[test]
    fn rust_companion_recovery_ignores_ambiguous_evidence() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        let lib_path = temp.path().join("src/lib.rs");
        fs::write(
            &lib_path,
            "pub fn existing() -> &'static str {\n    \"ok\"\n}\n",
        )
        .expect("write lib");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            ..RuntimeConfig::default()
        })
        .expect("runtime");

        let recovery = runtime
            .try_rust_companion_surface_recovery("error: could not compile crate", &[])
            .expect("recovery attempt");

        assert!(recovery.is_none());
        assert_eq!(
            fs::read_to_string(lib_path).expect("read lib"),
            "pub fn existing() -> &'static str {\n    \"ok\"\n}\n"
        );
    }

    #[test]
    fn successful_companion_recovery_reenters_validation_and_finalizes() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let _cwd = CurrentDirGuard::change_to(temp.path());
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::create_dir_all(temp.path().join("tests")).expect("create tests");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\n\n[package]\nname = \"companion_finalize\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .expect("write cargo");
        let lib_path = temp.path().join("src/lib.rs");
        let math_path = temp.path().join("src/math.rs");
        fs::write(
            &lib_path,
            "pub fn existing() -> &'static str {\n    \"ok\"\n}\n",
        )
        .expect("write lib");
        fs::write(
            temp.path().join("tests/feature_tests.rs"),
            "use companion_finalize::math::triple;\n\n#[test]\nfn feature_math_works() {\n    assert_eq!(triple(4), 12);\n}\n",
        )
        .expect("write tests");
        fs::write(&math_path, "pub fn triple(x: i32) -> i32 {\n    x * 3\n}\n")
            .expect("write math");

        let mut runtime = Runtime::new(RuntimeConfig {
            planner_type: "stub".to_string(),
            task: "Implement missing math module. Expose from src/lib.rs. Complete only after cargo test passes.".to_string(),
            auto_revert: true,
            ..RuntimeConfig::default()
        })
        .expect("runtime");
        runtime.state.iteration = 1;
        let math_mutation = Mutation {
            path: math_path.clone(),
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        };

        let should_continue = runtime
            .validate_and_commit_mutations(&[math_mutation])
            .expect("companion recovery should validate");

        assert!(!should_continue);
        assert_eq!(runtime.state.status, SessionStatus::Complete);
        assert!(
            fs::read_to_string(&lib_path)
                .expect("read lib")
                .contains("pub mod math;")
        );
        assert!(runtime.logs.iter().any(|entry| {
            entry.stage == "RECOVERY" && entry.message.contains("Rust companion-surface recovery")
        }));
        assert!(
            runtime
                .state
                .files_written
                .iter()
                .any(|path| same_snapshot_target(path, &lib_path))
        );
    }

    #[test]
    #[ignore = "requires a live local model and can fail on model output drift"]
    fn smoke_runs_with_qwen_coder_14b() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempfile::tempdir().expect("tempdir");

        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"forge_smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .expect("write Cargo.toml");
        fs::write(
            temp.path().join("src/lib.rs"),
            "pub fn existing() -> &'static str {\n    \"ok\"\n}\n",
        )
        .expect("write lib.rs");
        fs::write(
            temp.path().join("rasputin.json"),
            "{\n  \"ollama_model\": \"qwen2.5-coder:14b\"\n}\n",
        )
        .expect("write rasputin.json");

        let _cwd = CurrentDirGuard::change_to(temp.path());
        let result = run_bootstrap(RuntimeConfig {
            max_iterations: 8,
            task: "Create src/greeting.rs with a simple Rust function. Read Cargo.toml and src/lib.rs first. Complete only after src/greeting.rs exists.".to_string(),
            auto_revert: true,
            mode: ExecutionMode::Edit,
            planner_type: "http".to_string(),
            planner_endpoint: "http://127.0.0.1:11434".to_string(),
            planner_model: "14b".to_string(),
            planner_timeout_seconds: 60,
            planner_temperature: 0.0,
            planner_seed: 42,
            css_compression: false,
        });

        let greeting_path = temp.path().join("src/greeting.rs");
        let greeting = fs::read_to_string(&greeting_path).expect("read generated greeting.rs");

        assert!(
            result.success,
            "expected live smoke test to succeed, error={:?}",
            result.error
        );
        assert!(
            greeting.contains("fn ") && greeting.contains("greet"),
            "unexpected greeting.rs content: {}",
            greeting
        );
    }
}
