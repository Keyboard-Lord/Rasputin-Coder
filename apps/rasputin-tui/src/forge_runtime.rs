//! Forge Runtime Integration Module
//!
//! Bridges natural language chat into the hardened Forge execution pipeline:
//! INPUT -> PLAN -> EXECUTE -> VALIDATE -> COMMIT -> COMPLETE
//!
//! Streams real forge_bootstrap_rust process events into the TUI.
//! Execution always spawns one bounded Forge worker process per task.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender, channel},
};
use std::thread;
use tracing::{error, info};

// V1.5: Thread-safe wrapper for process handle to enable cancellation
#[derive(Clone)]
struct CancelToken(Arc<Mutex<Option<std::process::Child>>>);

impl CancelToken {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    fn set_process(&self, child: Child) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Some(child);
        }
    }

    fn cancel(&self) -> Result<()> {
        if let Ok(mut guard) = self.0.lock() {
            if let Some(mut child) = guard.take() {
                // Kill the process
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        Ok(())
    }

    fn get_process(&self) -> Option<Arc<Mutex<Option<Child>>>> {
        Some(self.0.clone())
    }
}

/// Unified runtime event types for streaming to UI
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    /// Runtime initialized
    Init {
        session_id: String,
        task: String,
        planner: String,
    },
    /// Iteration started
    IterationStart { iteration: u32 },
    /// Preflight checks passed
    PreflightPassed,
    /// Planner generated output
    PlannerOutput { raw: String, output_type: String },
    /// Protocol validation result
    ProtocolValidation {
        status: String,
        reason: Option<String>,
    },
    /// Tool call parsed
    ToolCall { name: String, arguments: String },
    /// Tool executing
    ToolExecuting { name: String },
    /// Tool execution result
    ToolResult {
        name: String,
        success: bool,
        output: Option<String>,
        error: Option<String>,
    },
    /// Browser preview server started
    BrowserPreview {
        url: String,
        port: u16,
        directory: String,
    },
    /// Mutations detected
    MutationsDetected { count: u32 },
    /// Validation running
    ValidationRunning,
    /// Validation result
    ValidationResult { decision: String, message: String },
    /// State committing
    StateCommitting { files_written: Vec<String> },
    /// Completion signal
    Completion { reason: String },
    /// Failure signal
    Failure { reason: String, recoverable: bool },
    /// Repair loop active
    RepairLoop {
        attempt: u32,
        max: u32,
        reason: String,
    },
    /// Runtime finished
    Finished {
        success: bool,
        iterations: u32,
        error: Option<String>,
    },
    /// Validation stage update with per-stage details
    ValidationStage {
        stage: String,
        status: ValidationStageStatus,
        duration_ms: u64,
        summary: Option<String>,
    },
    /// Context assembly completed with full V3 authority metadata
    ContextAssembly {
        /// Files with full selection metadata (path, reason, priority, inclusion status)
        files: Vec<ContextFileInfo>,
        /// Validation status and warnings/errors
        validation: ContextValidationInfo,
        /// Budget information (limits, usage, trimming)
        budget: ContextBudgetInfo,
        /// Human-readable summary
        summary: String,
    },
}

/// File selection metadata for ContextAssembly event
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFileInfo {
    /// File path
    pub path: String,
    /// Why this file was selected
    pub reason: String,
    /// Priority (higher = more important)
    pub priority: u32,
    /// Whether included in final context (may be trimmed due to budget)
    pub included: bool,
    /// Reason for trimming if excluded
    pub trimmed_reason: Option<String>,
}

/// Validation status for context assembly
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextValidationStatus {
    Valid,
    Warning,
    Invalid,
}

/// Validation information for ContextAssembly event
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextValidationInfo {
    /// Overall validation status
    pub status: ContextValidationStatus,
    /// Warning messages
    pub warnings: Vec<String>,
    /// Error messages
    pub errors: Vec<String>,
    /// Total files considered
    pub total_files: usize,
    /// Estimated token usage for included files
    pub estimated_tokens: usize,
}

/// Budget information for ContextAssembly event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudgetInfo {
    /// Maximum files allowed
    pub max_files: usize,
    /// Maximum tokens allowed
    pub max_tokens: usize,
    /// Number of files selected after trimming
    pub files_selected: usize,
    /// Tokens used after trimming
    pub tokens_used: usize,
    /// Whether trimming was triggered
    pub trimming_triggered: bool,
}

/// Status for validation stages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStageStatus {
    Running,
    Passed,
    Failed,
    Skipped,
}

/// Configuration for Forge runtime execution
#[derive(Debug, Clone)]
pub struct ForgeConfig {
    pub task: String,
    pub max_iterations: u32,
    pub planner_type: String, // "stub", "intelligent", "model", or "http"
    pub planner_endpoint: String,
    pub planner_model: String,
    pub working_dir: String,
    pub sandbox_mode: String,
    pub disposable_workspace_backend: Option<String>,
    pub disposable_workspace_retain_on_failure: Option<bool>,
    pub disposable_workspace_retain_on_success: Option<bool>,
    pub disposable_workspace_require_explicit_promotion: Option<bool>,
    pub css_compression: bool,
    pub planner_seed: u64,
    pub planner_temperature: f32,
}

impl Default for ForgeConfig {
    fn default() -> Self {
        Self {
            task: "Execute task".to_string(),
            max_iterations: 10,
            planner_type: "http".to_string(),
            planner_endpoint: "http://127.0.0.1:11434".to_string(),
            planner_model: "huihui_ai/deepseek-r1-abliterated:14b".to_string(),
            working_dir: ".".to_string(),
            sandbox_mode: "repo_boundary_only".to_string(),
            disposable_workspace_backend: None,
            disposable_workspace_retain_on_failure: None,
            disposable_workspace_retain_on_success: None,
            disposable_workspace_require_explicit_promotion: None,
            css_compression: true,
            planner_seed: 42,
            planner_temperature: 0.0,
        }
    }
}

/// Forge runtime handle for controlling execution
pub struct ForgeRuntimeHandle {
    pub event_receiver: Receiver<RuntimeEvent>,
    /// V1.5: Cancel token for stopping execution
    cancel_token: CancelToken,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct JsonlEvent {
    event_type: String,
    session_id: String,
    iteration: u32,
    timestamp: u64,
    severity: String,
    tool: Option<String>,
    result_code: Option<String>,
    affected_paths: Vec<String>,
    message: String,
    metadata: HashMap<String, String>,
}

#[derive(Debug, Default)]
struct RuntimeBridgeState {
    last_jsonl_line: Option<String>,
    last_tool: Option<String>,
    last_iteration: u32,
    final_iteration: Option<u32>,
    last_error: Option<String>,
}

#[derive(Debug)]
struct DisposableWorkspace {
    source_dir: PathBuf,
    worktree_dir: PathBuf,
    source_head: String,
    retain_on_success: bool,
    retain_on_failure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildRuntimeConfig {
    working_dir: String,
    sandbox_mode: String,
}

impl DisposableWorkspace {
    fn prepare(config: &ForgeConfig) -> Result<Option<Self>> {
        match config.sandbox_mode.as_str() {
            "none" | "repo_boundary_only" | "" => Ok(None),
            "disposable_workspace" => Self::prepare_git_worktree(config).map(Some),
            "external_container" => Err(anyhow!(
                "external_container is declared but not implemented; current runtime supports repo_boundary_only and disposable_workspace"
            )),
            other => Err(anyhow!("unsupported sandbox_mode '{}'", other)),
        }
    }

    fn prepare_git_worktree(config: &ForgeConfig) -> Result<Self> {
        let backend = config
            .disposable_workspace_backend
            .as_deref()
            .unwrap_or("git_worktree");
        if backend != "git_worktree" {
            return Err(anyhow!(
                "disposable_workspace backend '{}' is not implemented; current TUI backend is git_worktree",
                backend
            ));
        }

        if config
            .disposable_workspace_require_explicit_promotion
            .is_some_and(|required| !required)
        {
            return Err(anyhow!(
                "disposable_workspace requires explicit promotion; require_explicit_promotion=false is not supported"
            ));
        }

        let source_dir = PathBuf::from(&config.working_dir)
            .canonicalize()
            .with_context(|| {
                format!(
                    "cannot canonicalize source workspace {}",
                    config.working_dir
                )
            })?;

        ensure_git_repo(&source_dir)?;
        let source_head = git_stdout(&source_dir, &["rev-parse", "HEAD"])
            .context("failed to look up source HEAD before disposable execution")?;
        let worktree_dir = std::env::temp_dir().join(format!(
            "rasputin-disposable-worktree-{}",
            uuid::Uuid::new_v4()
        ));
        validate_disposable_worktree_path(&worktree_dir)?;

        let output = Command::new("git")
            .arg("-C")
            .arg(&source_dir)
            .args(["worktree", "add", "--detach"])
            .arg(&worktree_dir)
            .arg("HEAD")
            .output()
            .context("failed to create disposable git worktree")?;

        if !output.status.success() {
            return Err(anyhow!(
                "failed to create disposable git worktree: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(Self {
            source_dir,
            worktree_dir,
            source_head,
            retain_on_success: config
                .disposable_workspace_retain_on_success
                .unwrap_or(false),
            retain_on_failure: config
                .disposable_workspace_retain_on_failure
                .unwrap_or(false),
        })
    }

    fn execution_dir(&self) -> &Path {
        &self.worktree_dir
    }

    fn child_runtime_config(&self) -> ChildRuntimeConfig {
        ChildRuntimeConfig {
            working_dir: self.worktree_dir.to_string_lossy().to_string(),
            sandbox_mode: "repo_boundary_only".to_string(),
        }
    }

    fn promotion_report(&self) -> Result<String> {
        let current_source_head = git_stdout(&self.source_dir, &["rev-parse", "HEAD"])
            .context("failed to look up source HEAD after disposable execution")?;
        let source_changed = current_source_head != self.source_head;

        let changed_files = git_lines(
            &self.worktree_dir,
            &["status", "--porcelain", "--untracked-files=all"],
        )?;

        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.worktree_dir)
            .args(["add", "-N", "--", "."])
            .output();

        let diff = git_stdout_raw(&self.worktree_dir, &["diff", "--binary", "HEAD", "--"])
            .context("failed to generate disposable workspace diff")?;

        let mut report = String::new();
        report.push_str("Disposable workspace promotion package\n");
        report.push_str("execution_environment: disposable_workspace (git_worktree)\n");
        report.push_str("Mode: disposable_workspace (git_worktree)\n");
        report.push_str(&format!(
            "Source workspace: {}\n",
            self.source_dir.display()
        ));
        report.push_str(&format!(
            "Disposable workspace: {}\n",
            self.worktree_dir.display()
        ));
        report.push_str(&format!("Source HEAD at start: {}\n", self.source_head));
        report.push_str(&format!("Source HEAD now: {}\n", current_source_head));
        report.push_str(&format!("source_head_before: {}\n", self.source_head));
        report.push_str(&format!("source_head_after: {}\n", current_source_head));
        if source_changed {
            report.push_str(
                "Promotion status: BLOCKED - source workspace changed since task start\n",
            );
            report.push_str("promotion_status: blocked\n");
        } else {
            report.push_str(
                "Promotion status: READY FOR REVIEW - explicit promotion command not implemented\n",
            );
            report.push_str("promotion_status: pending_review\n");
        }
        report.push_str("changes_made:\n");
        report.push_str("\nChanged files:\n");
        if changed_files.is_empty() {
            report.push_str("- <none>\n");
        } else {
            for file in &changed_files {
                report.push_str(&format!("- {}\n", file));
            }
        }
        report.push_str("validation_results: report_only\n");
        report.push_str("\nUnified diff:\n");
        if diff.trim().is_empty() {
            report.push_str("<empty>\n");
        } else {
            report.push_str(&diff);
            if !diff.ends_with('\n') {
                report.push('\n');
            }
        }
        report.push_str("\nNo source workspace files were modified by this run.\n");
        Ok(report)
    }

    fn cleanup(&self, success: bool) -> Result<bool> {
        let retain = if success {
            self.retain_on_success
        } else {
            self.retain_on_failure
        };
        if retain {
            return Ok(false);
        }

        let output = Command::new("git")
            .arg("-C")
            .arg(&self.source_dir)
            .args(["worktree", "remove", "--force"])
            .arg(&self.worktree_dir)
            .output()
            .context("failed to remove disposable git worktree")?;

        if !output.status.success() {
            return Err(anyhow!(
                "failed to remove disposable git worktree: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        if self.worktree_dir.exists() {
            fs::remove_dir_all(&self.worktree_dir).with_context(|| {
                format!(
                    "failed to remove disposable worktree directory {}",
                    self.worktree_dir.display()
                )
            })?;
        }

        Ok(true)
    }
}

fn child_runtime_config(
    config: &ForgeConfig,
    disposable_workspace: Option<&DisposableWorkspace>,
) -> ChildRuntimeConfig {
    disposable_workspace
        .map(DisposableWorkspace::child_runtime_config)
        .unwrap_or_else(|| ChildRuntimeConfig {
            working_dir: config.working_dir.clone(),
            sandbox_mode: config.sandbox_mode.clone(),
        })
}

impl ForgeRuntimeHandle {
    /// Run a Forge task asynchronously, streaming real runtime events.
    /// Always spawns one bounded Forge worker process for this task.
    pub fn run_task(config: ForgeConfig) -> Self {
        info!("Spawning Forge worker process for task: {}", config.task);
        Self::run_task_process(config)
    }

    /// Run a Forge task via process spawn.
    fn run_task_process(config: ForgeConfig) -> Self {
        let (event_sender, event_receiver) = channel::<RuntimeEvent>();
        let cancel_token = CancelToken::new();
        let cancel_token_clone = cancel_token.clone();

        thread::spawn(move || {
            info!("Starting Forge runtime for task: {}", config.task);

            if let Err(e) =
                Self::execute_real_runtime(config, event_sender.clone(), cancel_token_clone)
            {
                error!("Forge runtime bridge error: {}", e);
                let _ = event_sender.send(RuntimeEvent::Failure {
                    reason: e.to_string(),
                    recoverable: false,
                });
                let _ = event_sender.send(RuntimeEvent::Finished {
                    success: false,
                    iterations: 0,
                    error: Some(e.to_string()),
                });
            }
        });

        ForgeRuntimeHandle {
            event_receiver,
            cancel_token,
        }
    }

    /// V1.5: Cancel the running execution
    pub fn cancel(&self) -> Result<()> {
        info!("Cancelling Forge runtime execution");
        self.cancel_token.cancel()
    }

    fn execute_real_runtime(
        config: ForgeConfig,
        sender: Sender<RuntimeEvent>,
        cancel_token: CancelToken,
    ) -> Result<()> {
        let disposable_workspace = DisposableWorkspace::prepare(&config)?;
        if let Some(workspace) = disposable_workspace.as_ref() {
            sender.send(RuntimeEvent::ToolResult {
                name: "disposable_workspace_created".to_string(),
                success: true,
                output: Some(format!(
                    "Created disposable git worktree at {}. Source workspace will not be mutated.",
                    workspace.execution_dir().display()
                )),
                error: None,
            })?;
        }

        let forge_binary = ensure_forge_binary()?;
        let child_config = child_runtime_config(&config, disposable_workspace.as_ref());
        if disposable_workspace.is_some() {
            sender.send(RuntimeEvent::ToolResult {
                name: "disposable_workspace_execution_started".to_string(),
                success: true,
                output: Some(format!(
                    "Worker config working_dir={} sandbox_mode={}",
                    child_config.working_dir, child_config.sandbox_mode
                )),
                error: None,
            })?;
        }
        let mut command = Command::new(&forge_binary);
        command
            .arg(&config.task)
            .arg(config.max_iterations.to_string())
            .arg(&config.planner_type)
            .current_dir(&child_config.working_dir)
            .env("FORGE_PLANNER_ENDPOINT", &config.planner_endpoint)
            .env("FORGE_PLANNER_MODEL", &config.planner_model)
            .env("FORGE_SANDBOX_MODE", &child_config.sandbox_mode)
            .env(
                "FORGE_CSS_COMPRESSION",
                if config.css_compression { "1" } else { "0" },
            )
            .env("FORGE_PLANNER_SEED", config.planner_seed.to_string())
            .env(
                "FORGE_PLANNER_TEMPERATURE",
                config.planner_temperature.to_string(),
            )
            .env("FORGE_OUTPUT_MODE", "jsonl")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                if let Some(workspace) = disposable_workspace.as_ref() {
                    let _ = workspace.cleanup(false);
                }
                return Err(error).with_context(|| {
                    format!(
                        "failed to launch Forge runtime in {}",
                        child_config.working_dir
                    )
                });
            }
        };

        // V1.5: Store child process ID in cancel token for external cancellation
        // We need to extract stdout/stderr before storing the child
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Forge runtime stdout not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Forge runtime stderr not available"))?;

        // Now store the child in cancel token (stdout/stderr already extracted)
        cancel_token.set_process(child);

        // Use the same Arc from cancel token for waiting
        let child_arc = cancel_token
            .get_process()
            .ok_or_else(|| anyhow!("cancel token not initialized"))?;
        let stderr_lines = Arc::new(Mutex::new(Vec::<String>::new()));

        let stderr_store = Arc::clone(&stderr_lines);
        let stderr_handle = thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(std::result::Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                if let Ok(mut buffer) = stderr_store.lock() {
                    buffer.push(trimmed.to_string());
                    if buffer.len() > 20 {
                        buffer.remove(0);
                    }
                }
            }
        });

        let stdout_sender = sender.clone();
        let stdout_config = config.clone();
        let stdout_handle = thread::spawn(move || -> RuntimeBridgeState {
            let mut state = RuntimeBridgeState::default();
            let reader = BufReader::new(stdout);

            for line in reader.lines().map_while(std::result::Result::ok) {
                for event in parse_runtime_line(&line, &stdout_config, &mut state) {
                    let _ = stdout_sender.send(event);
                }
            }

            state
        });

        let status = {
            let mut child_guard = child_arc
                .lock()
                .map_err(|_| anyhow!("failed to lock Forge runtime child process"))?;
            if let Some(ref mut child) = *child_guard {
                child.wait().context("failed waiting for Forge runtime")?
            } else {
                // Child was cancelled; treat the disposable workspace as failed work.
                if let Some(workspace) = disposable_workspace.as_ref() {
                    sender.send(RuntimeEvent::ToolResult {
                        name: "disposable_workspace_validation_failed".to_string(),
                        success: false,
                        output: None,
                        error: Some("Disposable worktree execution was cancelled".to_string()),
                    })?;
                    match workspace.cleanup(false) {
                        Ok(true) => sender.send(RuntimeEvent::ToolResult {
                            name: "disposable_workspace_cleaned".to_string(),
                            success: true,
                            output: Some(format!(
                                "Removed disposable worktree {}",
                                workspace.execution_dir().display()
                            )),
                            error: None,
                        })?,
                        Ok(false) if workspace.retain_on_failure => {
                            sender.send(RuntimeEvent::ToolResult {
                                name: "disposable_workspace_retained".to_string(),
                                success: true,
                                output: Some(format!(
                                    "Retained disposable worktree {}",
                                    workspace.execution_dir().display()
                                )),
                                error: None,
                            })?
                        }
                        Ok(false) => {}
                        Err(error) => sender.send(RuntimeEvent::ToolResult {
                            name: "disposable_workspace_cleanup_failed".to_string(),
                            success: false,
                            output: None,
                            error: Some(error.to_string()),
                        })?,
                    }
                }
                sender.send(RuntimeEvent::Finished {
                    success: false,
                    iterations: 0,
                    error: Some("Forge runtime execution was cancelled".to_string()),
                })?;
                return Ok(());
            }
        };

        let bridge_state = stdout_handle
            .join()
            .map_err(|_| anyhow!("Forge runtime stdout thread panicked"))?;
        stderr_handle
            .join()
            .map_err(|_| anyhow!("Forge runtime stderr thread panicked"))?;

        let stderr_tail = stderr_lines
            .lock()
            .ok()
            .map(|buffer| buffer.join("\n"))
            .filter(|joined| !joined.is_empty());

        let iterations = bridge_state
            .final_iteration
            .unwrap_or(bridge_state.last_iteration);
        let error = if status.success() {
            None
        } else {
            bridge_state.last_error.or(stderr_tail)
        };
        if disposable_workspace.is_some() {
            sender.send(RuntimeEvent::ToolResult {
                name: if status.success() {
                    "disposable_workspace_validation_passed".to_string()
                } else {
                    "disposable_workspace_validation_failed".to_string()
                },
                success: status.success(),
                output: status
                    .success()
                    .then(|| "Disposable worktree execution completed successfully".to_string()),
                error: if status.success() {
                    None
                } else {
                    error.clone().or_else(|| {
                        Some("Disposable worktree execution failed validation".to_string())
                    })
                },
            })?;
        }
        if status.success()
            && let Some(workspace) = disposable_workspace.as_ref()
        {
            match workspace.promotion_report() {
                Ok(report) => {
                    sender.send(RuntimeEvent::ToolResult {
                        name: "disposable_workspace_promotion_report_created".to_string(),
                        success: true,
                        output: Some(report),
                        error: None,
                    })?;
                    sender.send(RuntimeEvent::ToolResult {
                        name: "disposable_workspace_promotion_pending".to_string(),
                        success: true,
                        output: Some("Ran safely in a disposable worktree. Source files were not changed. Promotion report is ready.".to_string()),
                        error: None,
                    })?;
                }
                Err(error) => {
                    sender.send(RuntimeEvent::ToolResult {
                        name: "disposable_workspace_promotion_report_created".to_string(),
                        success: false,
                        output: None,
                        error: Some(format!("Failed to build promotion package: {}", error)),
                    })?;
                    if let Some(workspace) = disposable_workspace.as_ref() {
                        let _ = workspace.cleanup(false);
                    }
                    sender.send(RuntimeEvent::Finished {
                        success: false,
                        iterations,
                        error: Some(format!("Failed to build promotion package: {}", error)),
                    })?;
                    return Ok(());
                }
            }
        }

        if let Some(workspace) = disposable_workspace.as_ref() {
            let retained = if status.success() {
                workspace.retain_on_success
            } else {
                workspace.retain_on_failure
            };
            match workspace.cleanup(status.success()) {
                Ok(true) => sender.send(RuntimeEvent::ToolResult {
                    name: "disposable_workspace_cleaned".to_string(),
                    success: true,
                    output: Some(format!(
                        "Removed disposable worktree {}",
                        workspace.execution_dir().display()
                    )),
                    error: None,
                })?,
                Ok(false) if retained => sender.send(RuntimeEvent::ToolResult {
                    name: "disposable_workspace_retained".to_string(),
                    success: true,
                    output: Some(format!(
                        "Retained disposable worktree {}",
                        workspace.execution_dir().display()
                    )),
                    error: None,
                })?,
                Ok(false) => {}
                Err(error) => {
                    sender.send(RuntimeEvent::ToolResult {
                        name: "disposable_workspace_cleanup_failed".to_string(),
                        success: false,
                        output: None,
                        error: Some(error.to_string()),
                    })?;
                    sender.send(RuntimeEvent::Finished {
                        success: false,
                        iterations,
                        error: Some(format!("Disposable workspace cleanup failed: {}", error)),
                    })?;
                    return Ok(());
                }
            }
        }

        sender.send(RuntimeEvent::Finished {
            success: status.success(),
            iterations,
            error,
        })?;

        Ok(())
    }

    /// Poll for next event (non-blocking)
    pub fn poll_event(&self) -> Option<RuntimeEvent> {
        self.event_receiver.try_recv().ok()
    }

    #[cfg(test)]
    pub fn from_test_receiver(event_receiver: Receiver<RuntimeEvent>) -> Self {
        Self {
            event_receiver,
            cancel_token: CancelToken::new(),
        }
    }
}

fn forge_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("apps/rasputin-tui must live inside the repository root")
        .join("crates")
        .join("forge-runtime")
}

fn repo_root() -> PathBuf {
    forge_crate_dir()
        .parent()
        .and_then(|path| path.parent())
        .expect("forge crate should live under repository root")
        .to_path_buf()
}

fn ensure_forge_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("FORGE_RUNTIME_BIN") {
        let binary = PathBuf::from(path);
        if binary.exists() {
            return Ok(binary);
        }
        return Err(anyhow!("FORGE_RUNTIME_BIN points to a missing binary"));
    }

    if let Some(binary) = forge_binary_candidates()
        .into_iter()
        .find(|path| path.exists())
    {
        return Ok(binary);
    }

    build_forge_binary()?;

    forge_binary_candidates()
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| anyhow!("Forge binary not found after build"))
}

fn build_forge_binary() -> Result<()> {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--quiet")
        .arg("-p")
        .arg("forge_bootstrap")
        .current_dir(repo_root())
        .status()
        .context("failed to build Forge binary")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("cargo build failed while preparing Forge binary"))
    }
}

fn forge_binary_candidates() -> Vec<PathBuf> {
    let crate_dir = forge_crate_dir();
    let repo_root = repo_root();

    vec![
        crate_dir
            .join("target")
            .join("debug")
            .join(forge_binary_name()),
        repo_root
            .join("target")
            .join("debug")
            .join(forge_binary_name()),
    ]
}

fn forge_binary_name() -> &'static str {
    if cfg!(windows) {
        "forge_bootstrap.exe"
    } else {
        "forge_bootstrap"
    }
}

fn ensure_git_repo(path: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .context("failed to inspect git repository")?;

    if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true" {
        Ok(())
    } else {
        Err(anyhow!(
            "disposable_workspace currently requires a git repository; non-git copy backend is not implemented"
        ))
    }
}

fn git_stdout(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(git_stdout_raw(repo, args)?.trim().to_string())
}

fn git_stdout_raw(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn git_lines(repo: &Path, args: &[&str]) -> Result<Vec<String>> {
    Ok(git_stdout(repo, args)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn validate_disposable_worktree_path(path: &Path) -> Result<()> {
    let temp_root = std::env::temp_dir()
        .canonicalize()
        .context("failed to canonicalize temporary directory for disposable workspace")?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("disposable worktree path has no parent"))?;
    let parent = parent.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize disposable worktree parent {}",
            parent.display()
        )
    })?;

    if parent.starts_with(&temp_root) {
        Ok(())
    } else {
        Err(anyhow!(
            "disposable worktree path {} is outside temporary root {}",
            path.display(),
            temp_root.display()
        ))
    }
}

fn parse_runtime_line(
    line: &str,
    config: &ForgeConfig,
    state: &mut RuntimeBridgeState,
) -> Vec<RuntimeEvent> {
    let Some(raw_json) = extract_jsonl_payload(line) else {
        return Vec::new();
    };

    if state
        .last_jsonl_line
        .as_ref()
        .is_some_and(|previous| previous == raw_json)
    {
        return Vec::new();
    }
    state.last_jsonl_line = Some(raw_json.to_string());

    let Ok(entry) = serde_json::from_str::<JsonlEvent>(raw_json) else {
        return Vec::new();
    };

    state.last_iteration = entry.iteration;

    match entry.event_type.as_str() {
        "RUNTIME_INIT" => vec![RuntimeEvent::Init {
            session_id: entry.session_id,
            task: config.task.clone(),
            planner: config.planner_type.clone(),
        }],
        "ITERATION_START" => vec![RuntimeEvent::IterationStart {
            iteration: entry.iteration,
        }],
        "PREFLIGHT_PASSED" => vec![RuntimeEvent::PreflightPassed],
        "PLANNER_OUTPUT" => vec![RuntimeEvent::PlannerOutput {
            raw: entry.message.clone(),
            output_type: parse_output_type(&entry.message),
        }],
        "PROTOCOL_VALIDATION_START" => vec![RuntimeEvent::ProtocolValidation {
            status: "running".to_string(),
            reason: None,
        }],
        "PROTOCOL_VALIDATION_ACCEPT" => vec![RuntimeEvent::ProtocolValidation {
            status: "accepted".to_string(),
            reason: None,
        }],
        "PROTOCOL_VALIDATION_NORMALIZED" => vec![RuntimeEvent::ProtocolValidation {
            status: "normalized".to_string(),
            reason: Some(entry.message),
        }],
        "PROTOCOL_VALIDATION_REJECT" => {
            state.last_error = Some(entry.message.clone());
            vec![RuntimeEvent::ProtocolValidation {
                status: "rejected".to_string(),
                reason: Some(entry.message),
            }]
        }
        "PROTOCOL_VALIDATION_ESCALATE" => {
            state.last_error = Some(entry.message.clone());
            vec![RuntimeEvent::ProtocolValidation {
                status: "escalated".to_string(),
                reason: Some(entry.message),
            }]
        }
        "TOOL_EXECUTE" => {
            let tool_name = parse_tool_name(&entry.message)
                .or(entry.tool)
                .unwrap_or_else(|| "tool".to_string());
            state.last_tool = Some(tool_name.clone());
            vec![RuntimeEvent::ToolExecuting { name: tool_name }]
        }
        "TOOL_CALL_PARSED" => {
            let tool_name = entry
                .tool
                .or_else(|| parse_tool_name(&entry.message))
                .unwrap_or_else(|| "tool".to_string());
            let arguments = entry
                .metadata
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| "{}".to_string());
            vec![RuntimeEvent::ToolCall {
                name: tool_name,
                arguments,
            }]
        }
        "TOOL_SUCCESS" => {
            let tool_name = state
                .last_tool
                .clone()
                .or(entry.tool.clone())
                .unwrap_or_else(|| "tool".to_string());

            // Special handling for browser_preview tool
            if tool_name == "browser_preview" {
                // Parse port and directory from the output message
                // Format: "Browser preview server started\nURL: http://127.0.0.1:PORT\nPort: PORT\nDirectory: DIR"
                let url = extract_url(&entry.message)
                    .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
                let port = extract_port(&entry.message).unwrap_or(8080);
                let directory =
                    extract_directory(&entry.message).unwrap_or_else(|| ".".to_string());

                vec![
                    RuntimeEvent::ToolResult {
                        name: tool_name,
                        success: true,
                        output: Some(entry.message.clone()),
                        error: None,
                    },
                    RuntimeEvent::BrowserPreview {
                        url,
                        port,
                        directory,
                    },
                ]
            } else {
                vec![RuntimeEvent::ToolResult {
                    name: tool_name,
                    success: true,
                    output: Some(entry.message),
                    error: None,
                }]
            }
        }
        "TOOL_FAILED" => {
            state.last_error = Some(entry.message.clone());
            vec![RuntimeEvent::ToolResult {
                name: state
                    .last_tool
                    .clone()
                    .or(entry.tool.clone())
                    .unwrap_or_else(|| "tool".to_string()),
                success: false,
                output: None,
                error: Some(entry.message),
            }]
        }
        "MUTATIONS_DETECTED" => vec![RuntimeEvent::MutationsDetected {
            count: extract_numbers(&entry.message)
                .first()
                .copied()
                .unwrap_or(1),
        }],
        "VALIDATION_ACCEPT" => vec![RuntimeEvent::ValidationResult {
            decision: "accept".to_string(),
            message: entry.message,
        }],
        "VALIDATION_RUNNING" => vec![RuntimeEvent::ValidationRunning],
        "VALIDATION_STAGE" => {
            let stage = entry
                .metadata
                .get("stage")
                .cloned()
                .unwrap_or_else(|| "validation".to_string());
            let status = entry
                .metadata
                .get("status")
                .map(|value| parse_validation_stage_status(value))
                .unwrap_or(ValidationStageStatus::Running);
            let duration_ms = entry
                .metadata
                .get("duration_ms")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let summary = entry.metadata.get("summary").cloned();
            vec![RuntimeEvent::ValidationStage {
                stage,
                status,
                duration_ms,
                summary,
            }]
        }
        "VALIDATION_RESULT" => vec![RuntimeEvent::ValidationResult {
            decision: parse_validation_decision(&entry.message),
            message: entry.message,
        }],
        "VALIDATION_REJECT" => {
            state.last_error = Some(entry.message.clone());
            vec![RuntimeEvent::ValidationResult {
                decision: "reject".to_string(),
                message: entry.message,
            }]
        }
        "VALIDATION_ESCALATE" => {
            state.last_error = Some(entry.message.clone());
            vec![RuntimeEvent::ValidationResult {
                decision: "escalate".to_string(),
                message: entry.message,
            }]
        }
        "CONTEXT_ASSEMBLED" => {
            // Parse V3 authority metadata from event fields and metadata
            let files = parse_context_files_v3(&entry.affected_paths, &entry.metadata);
            let validation = parse_context_validation_v3(&entry.metadata);
            let budget = parse_context_budget_v3(&entry.metadata);

            vec![RuntimeEvent::ContextAssembly {
                files,
                validation,
                budget,
                summary: entry.message.clone(),
            }]
        }
        "STATE_COMMITTED" => vec![RuntimeEvent::StateCommitting {
            files_written: entry.affected_paths,
        }],
        "COMPLETION_GATE_ACCEPT" => vec![RuntimeEvent::Completion {
            reason: entry.message,
        }],
        "COMPLETION_GATE_REJECT" => {
            state.last_error = Some(entry.message.clone());
            vec![RuntimeEvent::Failure {
                reason: entry.message,
                recoverable: true,
            }]
        }
        "REPAIR_LOOP_ACTIVE" => {
            let numbers = extract_numbers(&entry.message);
            let attempt = numbers.first().copied().unwrap_or(1);
            let max = numbers.get(1).copied().unwrap_or(3);
            vec![RuntimeEvent::RepairLoop {
                attempt,
                max,
                reason: entry.message,
            }]
        }
        "PLANNER_FAILURE" => {
            let recoverable = entry.message.contains("recoverable: true");
            if !recoverable {
                state.last_error = Some(entry.message.clone());
            }
            vec![RuntimeEvent::Failure {
                reason: entry.message,
                recoverable,
            }]
        }
        "RUNTIME_ERROR" | "REPAIR_LOOP_EXHAUSTED" => {
            state.last_error = Some(entry.message.clone());
            vec![RuntimeEvent::Failure {
                reason: entry.message,
                recoverable: false,
            }]
        }
        "RUNTIME_COMPLETE" => {
            state.final_iteration = Some(entry.iteration);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn extract_jsonl_payload(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(json) = trimmed.strip_prefix("JSONL: ") {
        return Some(json.trim());
    }

    if trimmed.starts_with('{') && trimmed.contains("\"event_type\"") {
        return Some(trimmed);
    }

    None
}

fn parse_output_type(message: &str) -> String {
    message
        .strip_prefix("Output type: ")
        .and_then(|rest| rest.split_whitespace().next())
        .map(|value| value.trim_end_matches(',').to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_tool_name(message: &str) -> Option<String> {
    message
        .strip_prefix("Executing ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_validation_decision(message: &str) -> String {
    if message.contains("Reject") || message.contains("REJECT") {
        "reject".to_string()
    } else if message.contains("Escalate") || message.contains("ESCALATE") {
        "escalate".to_string()
    } else {
        "accept".to_string()
    }
}

fn parse_validation_stage_status(value: &str) -> ValidationStageStatus {
    match value {
        "passed" => ValidationStageStatus::Passed,
        "failed" => ValidationStageStatus::Failed,
        "skipped" => ValidationStageStatus::Skipped,
        _ => ValidationStageStatus::Running,
    }
}

/// Parse ContextAssembly V3 file metadata from JSONL event
fn parse_context_files_v3(
    affected_paths: &[String],
    metadata: &HashMap<String, String>,
) -> Vec<ContextFileInfo> {
    let mut files = vec![];

    // Parse file metadata from metadata hashmap
    // Format: file_0_path, file_0_reason, file_0_priority, file_0_included, file_0_trimmed_reason
    for (i, path) in affected_paths.iter().enumerate() {
        let prefix = format!("file_{}_", i);

        let reason = metadata
            .get(&format!("{}reason", prefix))
            .cloned()
            .unwrap_or_else(|| "Selected by context assembly".to_string());

        let priority = metadata
            .get(&format!("{}priority", prefix))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(50); // Default mid-priority

        let included = metadata
            .get(&format!("{}included", prefix))
            .map(|v| v == "true")
            .unwrap_or(true); // Default to included

        let trimmed_reason = metadata.get(&format!("{}trimmed_reason", prefix)).cloned();

        files.push(ContextFileInfo {
            path: path.clone(),
            reason,
            priority,
            included,
            trimmed_reason,
        });
    }

    // If no file metadata found but paths exist, create basic entries
    if files.is_empty() && !affected_paths.is_empty() {
        for path in affected_paths {
            files.push(ContextFileInfo {
                path: path.clone(),
                reason: "Selected by context assembly".to_string(),
                priority: 50,
                included: true,
                trimmed_reason: None,
            });
        }
    }

    files
}

/// Parse ContextAssembly V3 validation metadata from JSONL event
fn parse_context_validation_v3(metadata: &HashMap<String, String>) -> ContextValidationInfo {
    let status_str = metadata
        .get("validation_status")
        .map(|s| s.as_str())
        .unwrap_or("valid");

    let status = match status_str {
        "invalid" => ContextValidationStatus::Invalid,
        "warning" => ContextValidationStatus::Warning,
        _ => ContextValidationStatus::Valid,
    };

    let warnings: Vec<String> = metadata
        .get("validation_warnings")
        .map(|s| s.split('|').map(String::from).collect())
        .unwrap_or_default();

    let errors: Vec<String> = metadata
        .get("validation_errors")
        .map(|s| s.split('|').map(String::from).collect())
        .unwrap_or_default();

    let total_files = metadata
        .get("total_files")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let estimated_tokens = metadata
        .get("estimated_tokens")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    ContextValidationInfo {
        status,
        warnings,
        errors,
        total_files,
        estimated_tokens,
    }
}

/// Parse ContextAssembly V3 budget metadata from JSONL event
fn parse_context_budget_v3(metadata: &HashMap<String, String>) -> ContextBudgetInfo {
    let max_files = metadata
        .get("budget_max_files")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10); // Default from context_assembly.rs

    let max_tokens = metadata
        .get("budget_max_tokens")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50000); // Default from context_assembly.rs

    let files_selected = metadata
        .get("files_selected")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let tokens_used = metadata
        .get("tokens_used")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let trimming_triggered = metadata
        .get("trimming_triggered")
        .map(|v| v == "true")
        .unwrap_or(false);

    ContextBudgetInfo {
        max_files,
        max_tokens,
        files_selected,
        tokens_used,
        trimming_triggered,
    }
}

fn extract_numbers(message: &str) -> Vec<u32> {
    message
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|chunk| !chunk.is_empty())
        .filter_map(|chunk| chunk.parse::<u32>().ok())
        .collect()
}

/// Convert Forge event to user-friendly message
pub fn format_forge_event(event: &RuntimeEvent) -> String {
    match event {
        RuntimeEvent::Init {
            session_id,
            task,
            planner,
        } => {
            format!(
                "[start] Forge runtime\n  Session: {}\n  Task: {}\n  Planner: {}",
                crate::text::take_chars(session_id, 16),
                task,
                planner
            )
        }
        RuntimeEvent::IterationStart { iteration } => {
            format!("[iter] Iteration {}", iteration)
        }
        RuntimeEvent::PreflightPassed => "[OK] Preflight checks passed".to_string(),
        RuntimeEvent::PlannerOutput { raw, output_type } => {
            let preview: String = raw.chars().take(80).collect();
            format!("[plan] Planner output [{}]: {}...", output_type, preview)
        }
        RuntimeEvent::ProtocolValidation { status, reason } => {
            if let Some(r) = reason {
                format!("[check] Protocol validation: {} - {}", status, r)
            } else {
                format!("[check] Protocol validation: {}", status)
            }
        }
        RuntimeEvent::ToolCall { name, arguments } => {
            format!("[tool] {} {}", name, arguments)
        }
        RuntimeEvent::ToolExecuting { name } => {
            format!("[exec] {}", name)
        }
        RuntimeEvent::ToolResult {
            name,
            success,
            output,
            error,
        } => {
            if *success {
                format!(
                    "[OK] {}: {}",
                    name,
                    output.as_ref().unwrap_or(&"done".to_string())
                )
            } else {
                format!(
                    "[FAIL] {}: {}",
                    name,
                    error.as_ref().unwrap_or(&"error".to_string())
                )
            }
        }
        RuntimeEvent::MutationsDetected { count } => {
            format!("[edit] {} file(s) modified", count)
        }
        RuntimeEvent::BrowserPreview {
            url,
            port,
            directory,
        } => {
            format!(
                "[preview] Server started at {} (port: {}, dir: {})",
                url, port, directory
            )
        }
        RuntimeEvent::ValidationRunning => "[check] Running validation...".to_string(),
        RuntimeEvent::ValidationResult { decision, message } => {
            format!("[check] Validation {}: {}", decision, message)
        }
        RuntimeEvent::StateCommitting { files_written } => {
            if files_written.is_empty() {
                "[save] State committed".to_string()
            } else {
                format!("[save] {} file(s) written", files_written.len())
            }
        }
        RuntimeEvent::Completion { reason } => {
            format!("[done] Task completed: {}", reason)
        }
        RuntimeEvent::Failure {
            reason,
            recoverable,
        } => {
            if *recoverable {
                format!("[warn] Recoverable failure: {}", reason)
            } else {
                format!("[FAIL] Fatal failure: {}", reason)
            }
        }
        RuntimeEvent::RepairLoop {
            attempt,
            max,
            reason,
        } => {
            format!("[repair] attempt {}/{} - {}", attempt, max, reason)
        }
        RuntimeEvent::Finished {
            success,
            iterations,
            error,
        } => {
            if *success {
                format!(
                    "[done] Finished successfully after {} iterations",
                    iterations
                )
            } else if let Some(e) = error {
                format!("[FAIL] Failed after {} iterations: {}", iterations, e)
            } else {
                format!("[FAIL] Finished after {} iterations", iterations)
            }
        }
        RuntimeEvent::ValidationStage {
            stage,
            status,
            duration_ms,
            summary,
        } => {
            let status_str = match status {
                ValidationStageStatus::Running => "running",
                ValidationStageStatus::Passed => "passed",
                ValidationStageStatus::Failed => "failed",
                ValidationStageStatus::Skipped => "skipped",
            };
            let summary_str = summary.as_deref().unwrap_or("");
            format!(
                "[validation] {}: {} ({}ms) {}",
                stage, status_str, duration_ms, summary_str
            )
        }
        RuntimeEvent::ContextAssembly {
            files,
            validation,
            budget,
            summary,
        } => {
            let included_count = files.iter().filter(|f| f.included).count();
            let trimmed_count = files.len() - included_count;
            let status_icon = match validation.status {
                ContextValidationStatus::Valid => "✓",
                ContextValidationStatus::Warning => "⚠",
                ContextValidationStatus::Invalid => "✗",
            };
            if trimmed_count > 0 {
                format!(
                    "[context] {} {} file(s) selected ({} trimmed): {} [{} tokens]",
                    status_icon, included_count, trimmed_count, summary, budget.tokens_used
                )
            } else {
                format!(
                    "[context] {} {} file(s) selected: {} [{} tokens]",
                    status_icon, included_count, summary, budget.tokens_used
                )
            }
        }
    }
}

/// Extract URL from browser_preview tool output
fn extract_url(message: &str) -> Option<String> {
    // Look for line starting with "URL: "
    message
        .lines()
        .find_map(|line| line.strip_prefix("URL: ").map(|s| s.to_string()))
}

/// Extract port from browser_preview tool output
fn extract_port(message: &str) -> Option<u16> {
    // Look for line starting with "Port: "
    message.lines().find_map(|line| {
        line.strip_prefix("Port: ")
            .and_then(|s| s.trim().parse::<u16>().ok())
    })
}

/// Extract directory from browser_preview tool output
fn extract_directory(message: &str) -> Option<String> {
    // Look for line starting with "Directory: "
    message
        .lines()
        .find_map(|line| line.strip_prefix("Directory: ").map(|s| s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_config() -> ForgeConfig {
        ForgeConfig {
            task: "Create src/main.rs".to_string(),
            planner_type: "http".to_string(),
            ..ForgeConfig::default()
        }
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn temp_git_repo() -> TempDir {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        run_git(temp.path(), &["init", "-q"]);
        run_git(temp.path(), &["config", "user.email", "test@example.com"]);
        run_git(temp.path(), &["config", "user.name", "Test User"]);
        fs::write(temp.path().join("README.md"), "initial\n").expect("seed file should be written");
        run_git(temp.path(), &["add", "README.md"]);
        run_git(temp.path(), &["commit", "-q", "-m", "initial"]);
        temp
    }

    fn disposable_config(source_dir: &Path) -> ForgeConfig {
        ForgeConfig {
            working_dir: source_dir.to_string_lossy().to_string(),
            sandbox_mode: "disposable_workspace".to_string(),
            disposable_workspace_backend: Some("git_worktree".to_string()),
            disposable_workspace_require_explicit_promotion: Some(true),
            ..sample_config()
        }
    }

    fn source_status(repo: &Path) -> Vec<String> {
        git_lines(repo, &["status", "--porcelain", "--untracked-files=all"])
            .expect("status should be available")
    }

    #[test]
    fn disposable_workspace_creates_distinct_worktree_and_child_repo_boundary_config() {
        let repo = temp_git_repo();
        let source_dir = repo.path().canonicalize().unwrap();
        let config = disposable_config(&source_dir);

        let workspace = DisposableWorkspace::prepare(&config)
            .expect("workspace preparation should succeed")
            .expect("disposable workspace should be created");
        let child_config = child_runtime_config(&config, Some(&workspace));

        assert_ne!(workspace.execution_dir(), source_dir.as_path());
        assert_eq!(
            PathBuf::from(&child_config.working_dir),
            workspace.worktree_dir
        );
        assert_ne!(PathBuf::from(&child_config.working_dir), source_dir);
        assert_eq!(child_config.sandbox_mode, "repo_boundary_only");

        workspace.cleanup(false).expect("cleanup should succeed");
        assert!(!workspace.worktree_dir.exists());
    }

    #[test]
    fn disposable_workspace_rejects_invalid_backend_and_non_git_source() {
        let repo = temp_git_repo();
        let mut config = disposable_config(repo.path());
        config.disposable_workspace_backend = Some("copy".to_string());

        let error = DisposableWorkspace::prepare(&config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("backend 'copy' is not implemented"));

        let non_git = tempfile::tempdir().expect("temp dir should be created");
        let error = DisposableWorkspace::prepare(&disposable_config(non_git.path()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("requires a git repository"));

        let mut config = disposable_config(repo.path());
        config.sandbox_mode = "invalid_backend_mode".to_string();
        let error = DisposableWorkspace::prepare(&config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported sandbox_mode"));
    }

    #[test]
    fn disposable_workspace_rejects_disabled_explicit_promotion() {
        let repo = temp_git_repo();
        let mut config = disposable_config(repo.path());
        config.disposable_workspace_require_explicit_promotion = Some(false);

        let error = DisposableWorkspace::prepare(&config)
            .unwrap_err()
            .to_string();

        assert!(error.contains("require_explicit_promotion=false is not supported"));
    }

    #[test]
    fn disposable_workspace_rejects_git_repo_without_head() {
        let repo = tempfile::tempdir().expect("temp dir should be created");
        run_git(repo.path(), &["init", "-q"]);

        let error = DisposableWorkspace::prepare(&disposable_config(repo.path()))
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed to look up source HEAD before disposable execution"));
    }

    #[test]
    fn disposable_worktree_path_must_stay_under_temp_root() {
        let outside_temp = repo_root().join("rasputin-disposable-worktree-test");

        let error = validate_disposable_worktree_path(&outside_temp)
            .unwrap_err()
            .to_string();

        assert!(error.contains("outside temporary root"));
    }

    #[test]
    fn disposable_workspace_changes_stay_in_worktree_until_promotion_exists() {
        let repo = temp_git_repo();
        let config = disposable_config(repo.path());
        let workspace = DisposableWorkspace::prepare(&config)
            .expect("workspace preparation should succeed")
            .expect("disposable workspace should be created");
        let source_readme_before =
            fs::read_to_string(repo.path().join("README.md")).expect("source read should work");

        fs::write(
            workspace.worktree_dir.join("README.md"),
            "changed in disposable\n",
        )
        .expect("worktree file should be writable");
        fs::write(
            workspace.worktree_dir.join("new.txt"),
            "new disposable file\n",
        )
        .expect("worktree new file should be writable");

        assert_eq!(
            fs::read_to_string(repo.path().join("README.md")).expect("source read should work"),
            source_readme_before
        );
        assert!(source_status(repo.path()).is_empty());
        assert!(!repo.path().join("new.txt").exists());

        let report = workspace
            .promotion_report()
            .expect("promotion report should be generated");
        assert!(report.contains("execution_environment: disposable_workspace (git_worktree)"));
        assert!(report.contains("changes_made:"));
        assert!(report.contains("validation_results: report_only"));
        assert!(report.contains("source_head_before:"));
        assert!(report.contains("source_head_after:"));
        assert!(report.contains("promotion_status: pending_review"));
        assert!(report.contains("README.md"));
        assert!(report.contains("new.txt"));

        workspace.cleanup(false).expect("cleanup should succeed");
    }

    #[test]
    fn disposable_workspace_cleanup_and_retention_follow_failure_and_success_flags() {
        let repo = temp_git_repo();
        let mut config = disposable_config(repo.path());

        let cleaned = DisposableWorkspace::prepare(&config)
            .expect("workspace preparation should succeed")
            .expect("disposable workspace should be created");
        let cleaned_path = cleaned.worktree_dir.clone();
        assert!(cleaned.cleanup(false).expect("cleanup should succeed"));
        assert!(!cleaned_path.exists());

        config.disposable_workspace_retain_on_failure = Some(true);
        let mut retained_failure = DisposableWorkspace::prepare(&config)
            .expect("workspace preparation should succeed")
            .expect("disposable workspace should be created");
        let retained_failure_path = retained_failure.worktree_dir.clone();
        assert!(
            !retained_failure
                .cleanup(false)
                .expect("retention should skip cleanup")
        );
        assert!(retained_failure_path.exists());
        retained_failure.retain_on_failure = false;
        retained_failure
            .cleanup(false)
            .expect("manual cleanup should succeed");

        config.disposable_workspace_retain_on_failure = Some(false);
        config.disposable_workspace_retain_on_success = Some(true);
        let mut retained_success = DisposableWorkspace::prepare(&config)
            .expect("workspace preparation should succeed")
            .expect("disposable workspace should be created");
        let retained_success_path = retained_success.worktree_dir.clone();
        assert!(
            !retained_success
                .cleanup(true)
                .expect("retention should skip cleanup")
        );
        assert!(retained_success_path.exists());
        retained_success.retain_on_success = false;
        retained_success
            .cleanup(true)
            .expect("manual cleanup should succeed");
    }

    #[test]
    fn promotion_report_blocks_when_source_head_changes() {
        let repo = temp_git_repo();
        let config = disposable_config(repo.path());
        let workspace = DisposableWorkspace::prepare(&config)
            .expect("workspace preparation should succeed")
            .expect("disposable workspace should be created");

        fs::write(repo.path().join("README.md"), "changed in source\n")
            .expect("source update should be written");
        run_git(repo.path(), &["add", "README.md"]);
        run_git(repo.path(), &["commit", "-q", "-m", "source moved"]);

        let report = workspace
            .promotion_report()
            .expect("promotion report should be generated");

        assert!(report.contains("Promotion status: BLOCKED"));
        assert!(report.contains("promotion_status: blocked"));
        workspace.cleanup(false).expect("cleanup should succeed");
    }

    #[test]
    fn promotion_report_fails_when_source_head_lookup_fails() {
        let source = tempfile::tempdir().expect("temp dir should be created");
        let worktree = temp_git_repo();
        let workspace = DisposableWorkspace {
            source_dir: source.path().to_path_buf(),
            worktree_dir: worktree.path().to_path_buf(),
            source_head: "missing".to_string(),
            retain_on_success: true,
            retain_on_failure: true,
        };

        let error = workspace.promotion_report().unwrap_err().to_string();

        assert!(error.contains("failed to look up source HEAD after disposable execution"));
    }

    #[test]
    fn promotion_report_fails_when_diff_generation_fails() {
        let source = temp_git_repo();
        let worktree = tempfile::tempdir().expect("temp dir should be created");
        run_git(worktree.path(), &["init", "-q"]);
        let source_head = git_stdout(source.path(), &["rev-parse", "HEAD"]).unwrap();
        let workspace = DisposableWorkspace {
            source_dir: source.path().to_path_buf(),
            worktree_dir: worktree.path().to_path_buf(),
            source_head,
            retain_on_success: true,
            retain_on_failure: true,
        };

        let error = workspace.promotion_report().unwrap_err().to_string();

        assert!(error.contains("failed to generate disposable workspace diff"));
    }

    #[test]
    fn cleanup_failure_is_reported() {
        let source = tempfile::tempdir().expect("temp dir should be created");
        let worktree = tempfile::tempdir().expect("temp dir should be created");
        let workspace = DisposableWorkspace {
            source_dir: source.path().to_path_buf(),
            worktree_dir: worktree.path().to_path_buf(),
            source_head: "missing".to_string(),
            retain_on_success: false,
            retain_on_failure: false,
        };

        let error = workspace.cleanup(false).unwrap_err().to_string();

        assert!(error.contains("failed to remove disposable git worktree"));
    }

    #[test]
    fn disposable_workspace_user_facing_events_are_stable() {
        let expected = [
            "disposable_workspace_created",
            "disposable_workspace_execution_started",
            "disposable_workspace_validation_passed",
            "disposable_workspace_validation_failed",
            "disposable_workspace_promotion_report_created",
            "disposable_workspace_promotion_pending",
            "disposable_workspace_cleaned",
            "disposable_workspace_retained",
            "disposable_workspace_cleanup_failed",
        ];

        for name in expected {
            let formatted = format_forge_event(&RuntimeEvent::ToolResult {
                name: name.to_string(),
                success: true,
                output: Some("ok".to_string()),
                error: None,
            });
            assert!(formatted.contains(name));
        }
    }

    #[test]
    fn parses_prefixed_jsonl_line() {
        let mut state = RuntimeBridgeState::default();
        let config = sample_config();
        let events = parse_runtime_line(
            r#"JSONL: {"event_type":"ITERATION_START","session_id":"forge-123","iteration":2,"timestamp":1,"severity":"INFO","tool":null,"result_code":null,"affected_paths":[],"message":"--- Iteration 2 ---","metadata":{}}"#,
            &config,
            &mut state,
        );

        assert_eq!(events, vec![RuntimeEvent::IterationStart { iteration: 2 }]);
        assert_eq!(state.last_iteration, 2);
    }

    #[test]
    fn deduplicates_consecutive_jsonl_lines() {
        let mut state = RuntimeBridgeState::default();
        let config = sample_config();
        let line = r#"{"event_type":"PREFLIGHT_PASSED","session_id":"forge-123","iteration":0,"timestamp":1,"severity":"INFO","tool":null,"result_code":null,"affected_paths":[],"message":"Checks passed","metadata":{}}"#;

        let first = parse_runtime_line(line, &config, &mut state);
        let second = parse_runtime_line(line, &config, &mut state);

        assert_eq!(first, vec![RuntimeEvent::PreflightPassed]);
        assert!(second.is_empty());
    }

    #[test]
    fn tracks_tool_name_for_tool_results() {
        let mut state = RuntimeBridgeState::default();
        let config = sample_config();
        let execute_line = r#"{"event_type":"TOOL_EXECUTE","session_id":"forge-123","iteration":1,"timestamp":1,"severity":"INFO","tool":null,"result_code":null,"affected_paths":[],"message":"Executing write_file","metadata":{}}"#;
        let success_line = r#"{"event_type":"TOOL_SUCCESS","session_id":"forge-123","iteration":1,"timestamp":2,"severity":"INFO","tool":null,"result_code":null,"affected_paths":[],"message":"Tool succeeded: ok","metadata":{}}"#;

        let execute_events = parse_runtime_line(execute_line, &config, &mut state);
        let success_events = parse_runtime_line(success_line, &config, &mut state);

        assert_eq!(
            execute_events,
            vec![RuntimeEvent::ToolExecuting {
                name: "write_file".to_string()
            }]
        );
        assert_eq!(
            success_events,
            vec![RuntimeEvent::ToolResult {
                name: "write_file".to_string(),
                success: true,
                output: Some("Tool succeeded: ok".to_string()),
                error: None,
            }]
        );
    }

    #[test]
    fn parses_tool_call_event_from_structured_metadata() {
        let mut state = RuntimeBridgeState::default();
        let config = sample_config();
        let line = r#"{"event_type":"TOOL_CALL_PARSED","session_id":"forge-123","iteration":1,"timestamp":1,"severity":"INFO","tool":"read_file","result_code":null,"affected_paths":[],"message":"Tool parsed: read_file","metadata":{"arguments":"{\"path\":\"src/lib.rs\"}"}}"#;

        let events = parse_runtime_line(line, &config, &mut state);

        assert_eq!(
            events,
            vec![RuntimeEvent::ToolCall {
                name: "read_file".to_string(),
                arguments: "{\"path\":\"src/lib.rs\"}".to_string(),
            }]
        );
    }

    #[test]
    fn parses_validation_stage_event() {
        let mut state = RuntimeBridgeState::default();
        let config = sample_config();
        let line = r#"{"event_type":"VALIDATION_STAGE","session_id":"forge-123","iteration":1,"timestamp":1,"severity":"INFO","tool":null,"result_code":null,"affected_paths":[],"message":"cargo test passed","metadata":{"stage":"test","status":"passed","duration_ms":"42","summary":"cargo test passed"}}"#;

        let events = parse_runtime_line(line, &config, &mut state);

        assert_eq!(
            events,
            vec![RuntimeEvent::ValidationStage {
                stage: "test".to_string(),
                status: ValidationStageStatus::Passed,
                duration_ms: 42,
                summary: Some("cargo test passed".to_string()),
            }]
        );
    }

    #[test]
    fn task_start_default_policy_allows_no_git_repo_with_warning() {
        let grounding = GitGrounding::no_repo();
        let result = TaskStartChecker::check(&grounding, &TaskStartPolicy::default());

        assert!(result.allowed);
        assert!(!result.requires_approval);
        assert!(result.has_warnings());
        assert_eq!(result.warnings[0].class, "no_repo");
    }

    #[test]
    fn task_start_disabled_git_policy_suppresses_no_repo_warning() {
        let grounding = GitGrounding::no_repo();
        let result = TaskStartChecker::check(&grounding, &TaskStartPolicy::disabled());

        assert!(result.allowed);
        assert!(!result.requires_approval);
        assert!(!result.has_warnings());
    }
}

// ============================================================================
// Git Grounding Types (Stubs for compilation - full implementation in sprint)
// ============================================================================

/// Git repository state for grounding and safety checks
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitGrounding {
    pub repo_detected: bool,
    pub branch_name: Option<String>,
    pub head_commit: Option<String>,
    pub is_dirty: bool,
    pub modified_files: Vec<GitFileStatus>,
    pub staged_files: Vec<GitFileStatus>,
    pub untracked_files: Vec<GitFileStatus>,
    pub recent_commits: Vec<GitCommitSummary>,
}

impl GitGrounding {
    /// Create a "no repo" grounding
    pub fn no_repo() -> Self {
        Self {
            repo_detected: false,
            ..Default::default()
        }
    }

    /// Create grounding from repo state
    pub fn from_repo(
        branch_name: Option<String>,
        head_commit: Option<String>,
        is_dirty: bool,
        modified_files: Vec<GitFileStatus>,
        staged_files: Vec<GitFileStatus>,
        untracked_files: Vec<GitFileStatus>,
        recent_commits: Vec<GitCommitSummary>,
    ) -> Self {
        Self {
            repo_detected: true,
            branch_name,
            head_commit,
            is_dirty,
            modified_files,
            staged_files,
            untracked_files,
            recent_commits,
        }
    }

    /// Check if worktree is clean
    pub fn is_clean(&self) -> bool {
        !self.is_dirty
            && self.modified_files.is_empty()
            && self.staged_files.is_empty()
            && self.untracked_files.is_empty()
    }

    /// Total number of changes
    pub fn total_changes(&self) -> usize {
        self.modified_files.len() + self.staged_files.len() + self.untracked_files.len()
    }

    /// Summary string for display
    pub fn summary(&self) -> String {
        if !self.repo_detected {
            return "no repo".to_string();
        }

        let commit = self
            .head_commit
            .as_ref()
            .map(|c| crate::text::take_chars(c, 7))
            .unwrap_or_else(|| "???????".to_string());
        let branch = self.branch_name.as_deref().unwrap_or("(detached)");
        let dirty_marker = if self.is_dirty { "*" } else { "" };

        format!("{}{}: {}{}", commit, dirty_marker, branch, dirty_marker)
    }
}

/// Status of a single file in Git
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitFileStatus {
    pub path: String,
    pub status: String,
}

impl GitFileStatus {
    pub fn new(path: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            status: status.into(),
        }
    }
}

/// Summary of a Git commit
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitCommitSummary {
    pub short_hash: String,
    pub subject: String,
    pub author: Option<String>,
}

impl GitCommitSummary {
    pub fn new(short_hash: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            short_hash: short_hash.into(),
            subject: subject.into(),
            author: None,
        }
    }

    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
}

/// Warning severity for task start checks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningSeverity {
    Info,
    Warning,
    Error,
}

/// A single warning for task start
#[derive(Debug, Clone)]
pub struct TaskStartWarning {
    pub class: String,
    pub message: String,
    pub severity: WarningSeverity,
}

/// Result of task start safety check
#[derive(Debug, Clone)]
pub struct TaskStartCheckResult {
    pub allowed: bool,
    pub warnings: Vec<TaskStartWarning>,
    pub requires_approval: bool,
    pub summary: String,
}

impl TaskStartCheckResult {
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// Git safety behavior for task start checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitPolicy {
    /// Require approval for configured risky Git states.
    Strict,
    /// Warn about Git state but do not block execution.
    Advisory,
    /// Skip Git state checks.
    Disabled,
}

/// Policy for task start checks
#[derive(Debug, Clone)]
pub struct TaskStartPolicy {
    pub git_policy: GitPolicy,
    pub require_approval_on_dirty_worktree: bool,
    pub require_approval_on_detached_head: bool,
    pub warn_on_staged_changes: bool,
    pub warn_on_untracked_targets: bool,
}

impl Default for TaskStartPolicy {
    fn default() -> Self {
        Self {
            git_policy: GitPolicy::Advisory,
            require_approval_on_dirty_worktree: false,
            require_approval_on_detached_head: false,
            warn_on_staged_changes: true,
            warn_on_untracked_targets: true,
        }
    }
}

impl TaskStartPolicy {
    pub fn strict() -> Self {
        Self {
            git_policy: GitPolicy::Strict,
            require_approval_on_dirty_worktree: true,
            require_approval_on_detached_head: true,
            warn_on_staged_changes: true,
            warn_on_untracked_targets: true,
        }
    }

    pub fn advisory() -> Self {
        Self::default()
    }

    pub fn disabled() -> Self {
        Self {
            git_policy: GitPolicy::Disabled,
            require_approval_on_dirty_worktree: false,
            require_approval_on_detached_head: false,
            warn_on_staged_changes: false,
            warn_on_untracked_targets: false,
        }
    }
}

/// Task start safety checker
pub struct TaskStartChecker;

impl TaskStartChecker {
    /// Check if task can start given Git state and policy
    pub fn check(grounding: &GitGrounding, policy: &TaskStartPolicy) -> TaskStartCheckResult {
        let mut warnings = Vec::new();
        let mut requires_approval = false;
        let allowed = true;

        if policy.git_policy == GitPolicy::Disabled {
            return TaskStartCheckResult {
                allowed,
                warnings,
                requires_approval,
                summary: "Git checks disabled".to_string(),
            };
        }

        // Check for repo
        if !grounding.repo_detected {
            warnings.push(TaskStartWarning {
                class: "no_repo".to_string(),
                message: "No Git repository detected - changes will not be tracked".to_string(),
                severity: WarningSeverity::Warning,
            });
        }

        // Check for detached HEAD
        if grounding.branch_name.is_none() && grounding.repo_detected {
            if policy.git_policy == GitPolicy::Strict && policy.require_approval_on_detached_head {
                requires_approval = true;
            }
            warnings.push(TaskStartWarning {
                class: "detached_head".to_string(),
                message: "Detached HEAD state".to_string(),
                severity: WarningSeverity::Warning,
            });
        }

        // Check for dirty worktree
        if grounding.is_dirty {
            if policy.git_policy == GitPolicy::Strict && policy.require_approval_on_dirty_worktree {
                requires_approval = true;
            }
            warnings.push(TaskStartWarning {
                class: "dirty_worktree".to_string(),
                message: format!(
                    "Dirty worktree ({} modified files)",
                    grounding.modified_files.len()
                ),
                severity: WarningSeverity::Warning,
            });
        }

        // Check for staged changes
        if !grounding.staged_files.is_empty() && policy.warn_on_staged_changes {
            warnings.push(TaskStartWarning {
                class: "staged_changes".to_string(),
                message: format!(
                    "{} staged but uncommitted files",
                    grounding.staged_files.len()
                ),
                severity: WarningSeverity::Info,
            });
        }

        // Check for untracked files
        if !grounding.untracked_files.is_empty() && policy.warn_on_untracked_targets {
            warnings.push(TaskStartWarning {
                class: "untracked_files".to_string(),
                message: format!("{} untracked files", grounding.untracked_files.len()),
                severity: WarningSeverity::Info,
            });
        }

        let summary = if allowed {
            if requires_approval {
                "Approval required".to_string()
            } else if warnings.is_empty() {
                "Clean".to_string()
            } else {
                format!("{} warnings", warnings.len())
            }
        } else {
            "Blocked".to_string()
        };

        TaskStartCheckResult {
            allowed,
            warnings,
            requires_approval,
            summary,
        }
    }
}

// ============================================================================
// Observability Types (Stubs for compilation)
// ============================================================================

/// Observability module for timeline tracking
pub mod observability {
    /// Status of a timeline entry
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TimelineStatus {
        Completed,
        Failed,
        Skipped,
        Pending,
        Running,
    }

    /// Outcome of an execution timeline
    #[derive(Debug, Clone)]
    pub enum TimelineOutcome {
        Success,
        Failure { reason: String },
        InProgress,
    }

    /// Single entry in execution timeline
    #[derive(Debug, Clone)]
    pub struct TimelineEntry {
        pub index: u32,
        pub phase: String,
        pub status: TimelineStatus,
        pub summary: String,
        pub detail: Option<String>,
        pub related_step: Option<u32>,
        pub related_tool: Option<String>,
        pub timestamp: u64,
    }

    impl TimelineEntry {
        /// Create a new timeline entry
        pub fn new(
            index: u32,
            phase: impl Into<String>,
            status: TimelineStatus,
            summary: impl Into<String>,
        ) -> Self {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Self {
                index,
                phase: phase.into(),
                status,
                summary: summary.into(),
                detail: None,
                related_step: None,
                related_tool: None,
                timestamp,
            }
        }
    }

    /// Full execution timeline
    #[derive(Debug, Clone)]
    pub struct ExecutionTimeline {
        pub run_id: String,
        pub task: String,
        pub entries: Vec<TimelineEntry>,
        pub duration_ms: Option<u64>,
        pub outcome: TimelineOutcome,
    }

    impl ExecutionTimeline {
        /// Calculate total duration in milliseconds
        pub fn total_duration_ms(&self) -> Option<u64> {
            self.duration_ms
        }
    }

    /// Failure explanation for debugging
    #[derive(Debug, Clone)]
    pub struct FailureExplanation {
        pub category: String,
        pub headline: String,
        pub explanation: String,
        pub context: String,
        pub remediation: Vec<String>,
    }

    /// Summary of mutations made in a step
    #[derive(Debug, Clone)]
    pub struct StepMutationSummary {
        pub files_created: Vec<String>,
        pub files_modified: Vec<String>,
        pub files_deleted: Vec<String>,
        pub total_lines_changed: usize,
        pub committed_mutations: Vec<String>,
        pub reverted_mutations: Vec<String>,
    }

    /// Planner trace for debugging
    #[derive(Debug, Clone)]
    pub struct PlannerTrace {
        pub iterations: Vec<PlannerIteration>,
        pub final_prompt: String,
        pub response_tokens: usize,
        pub step_index: u32,
        pub raw_output_excerpt: String,
        pub classification: String,
        pub accepted_tool: Option<String>,
        pub rejected_reason: Option<String>,
        pub repair_attempted: bool,
        pub repair_outcome: Option<String>,
    }

    #[derive(Debug, Clone)]
    pub struct PlannerIteration {
        pub index: u32,
        pub prompt_preview: String,
        pub response_preview: String,
        pub tool_calls: Vec<String>,
    }

    /// Replay comparison section
    #[derive(Debug, Clone)]
    pub struct ReplaySection {
        pub section: String,
        pub matched: bool,
        pub expected: String,
        pub actual: String,
        pub explanation: Option<String>,
    }

    /// Replay comparison result
    #[derive(Debug, Clone)]
    pub struct ReplayComparison {
        pub step_index: u32,
        pub fingerprint_match: bool,
        pub tool_calls_match: bool,
        pub mutations_match: bool,
        pub divergence_point: Option<String>,
        pub compared_sections: Vec<ReplaySection>,
        pub original_run_id: String,
        pub replay_run_id: String,
        pub matched: bool,
    }

    /// Debug bundle for export
    #[derive(Debug, Clone)]
    pub struct DebugBundle {
        pub timestamp: u64,
        pub session_id: String,
        pub timeline: ExecutionTimeline,
        pub logs: Vec<String>,
        pub run_id: String,
        pub task: String,
        pub failure_explanation: Option<FailureExplanation>,
        pub replay_comparison: Option<ReplayComparison>,
    }
}
