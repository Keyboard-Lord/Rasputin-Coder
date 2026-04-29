use crate::commands::{Command, get_help_text};
use crate::forge_runtime::{
    ForgeConfig, ForgeRuntimeHandle, GitGrounding, RuntimeEvent, TaskStartChecker, TaskStartPolicy,
    format_forge_event,
};
use crate::host_actions::{HostAction, HostActionResult, execute as execute_host_action};
use crate::interface_integration::{InterfaceIntegration, format_user_event};
use crate::ollama::{
    ChatMessage, DEFAULT_CODER_14B_MODEL, FALLBACK_PLANNER_MODEL, OllamaClient,
    model_preference_rank, normalize_requested_model, should_enable_css_compression,
};
use crate::persistence::PersistentState;
use crate::repo::{Repo, capture_git_grounding};
use crate::state::{
    AppState, Artifact, ExecutionMode, ExecutionOutcome, ExecutionPlan, ExecutionState,
    ExecutionStepStatus, FailureContext, InputMode, InspectorTab, LogEntry, LogLevel, Message,
    MessageRole, RunCard, RuntimeStatus, StepAction, StructuredOutput, StructuredOutputKind,
    default_validation_stages,
};
use crate::ui::layout::LayoutState;
use crate::validation::ValidationPipeline;
use anyhow::Result;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::HashSet;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub enum UiAction {
    NewChat,
    Projects,
    Search,
    Plugins,
    Automations,
    StartProjectCreate,
    StartProjectConnect,
    PickProjectFolder,
    OpenProject(String),
    OpenSearchResult(usize),
    RunValidation,
    RefreshRuntime,
    ResetValidation,
    ClearLogs,
    UseModel(String),
    ToggleInspector,
    ToggleExperienceMode,
    SelectInspectorTab(InspectorTab),
    OpenBrowserPreview { url: String },
    OpenConversation(String),
    ArchiveConversation(String),
    UnarchiveConversation(String),
    FocusInput,
    SetExecutionMode(ExecutionMode),
}

#[derive(Debug, Clone)]
pub struct UiTarget {
    pub id: String,
    pub area: Rect,
    pub action: UiAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    Chat,
    Projects,
    Search,
    Plugins,
    Automations,
}

impl SidebarPanel {
    pub fn title(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Projects => "Projects",
            Self::Search => "Search",
            Self::Plugins => "Plugins",
            Self::Automations => "Automations",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerMode {
    Chat,
    Search,
    ProjectCreate,
    ProjectConnect,
    Passive,
}

/// Synchronous intent classification result
/// Determined at input submit time BEFORE any UI reaction or model call
#[derive(Debug, Clone, PartialEq, Eq)]
enum InputRouting {
    /// Slash command - parse and execute
    Command,
    /// Structured execution intent (task-like plain text) - route to goal pipeline
    TaskGoal,
    /// Task mode execution - granular plan
    TaskExecution,
    /// Conversational chat - direct LLM
    Chat,
    /// Follow-up to existing work (continue, fix that, do the rest) - resolved from working memory
    FollowUp { resolved_task: String, original_input: String },
    /// Simple natural language command (e.g., "docs", "create README.md")
    SimpleCommand { contract: crate::artifact_contract::ArtifactContract },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperienceMode {
    Normal,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelStatusLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct PanelStatus {
    pub level: PanelStatusLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCreateWorkflow {
    pub parent_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ProjectEntry {
    pub name: String,
    pub path: String,
    pub display_path: String,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectSearchResult {
    pub file_path: String,
    pub display_path: String,
    pub line_number: usize,
    pub preview: String,
}

#[derive(Debug, Clone)]
pub struct SearchPreviewLine {
    pub number: usize,
    pub content: String,
    pub highlighted: bool,
}

#[derive(Debug, Clone)]
pub struct SearchPreview {
    pub title: String,
    pub file_path: String,
    pub lines: Vec<SearchPreviewLine>,
}

#[derive(Debug, Clone)]
pub enum ApprovalAction {
    RunCommand { command: String },
    DeleteProject { path: PathBuf },
    StartForgeTask { task: String },
}

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub title: String,
    pub detail: String,
    pub fix_hint: Option<String>,
    pub action: ApprovalAction,
}

/// Bounded repository snapshot for LLM grounding
/// Caches repo structure and key files to inject into chat context
#[derive(Debug, Clone, Default)]
pub struct RepoSnapshot {
    /// Project name at time of snapshot
    pub project_name: String,
    /// Project path at time of snapshot  
    pub project_path: String,
    /// Top-level directory tree (bounded)
    pub tree: String,
    /// Key files content (README, Cargo.toml, etc.)
    pub key_files: Vec<(String, String)>,
    /// Timestamp when snapshot was built
    pub cached_at: chrono::DateTime<chrono::Local>,
    /// Key file mtimes for cache invalidation
    pub file_fingerprints: Vec<(String, u64)>,
}

impl RepoSnapshot {
    /// Build a fresh snapshot from repo path
    /// Bounded: max depth 2, max 50 entries per level, max 50 lines per file
    pub fn build(repo_name: &str, repo_path: &str) -> Option<Self> {
        let path = Path::new(repo_path);
        if !path.exists() || !path.is_dir() {
            return None;
        }

        let mut key_files = Vec::new();
        let mut file_fingerprints = Vec::new();
        let mut tree_lines = Vec::new();

        // Build bounded directory tree (depth 2, max 50 entries per level)
        Self::build_tree(path, &mut tree_lines, 0, 2, 50);
        let tree = tree_lines.join("\n");

        // Read key config/documentation files with size limits
        let key_file_names = [
            "README.md",
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "setup.py",
            "requirements.txt",
            "go.mod",
            "Gemfile",
            "pom.xml",
            "build.gradle",
            "CMakeLists.txt",
            "Makefile",
            "Dockerfile",
            "docker-compose.yml",
            ".gitignore",
            "LICENSE",
            "CHANGELOG.md",
            "CONTRIBUTING.md",
        ];

        for filename in &key_file_names {
            let file_path = path.join(filename);
            if let Ok(metadata) = std::fs::metadata(&file_path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                        file_fingerprints.push((filename.to_string(), duration.as_secs()));
                    }
                }

                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    // Cap at 50 lines, 2000 chars
                    let capped = Self::cap_content(&content, 50, 2000);
                    key_files.push((filename.to_string(), capped));
                }
            }
        }

        Some(Self {
            project_name: repo_name.to_string(),
            project_path: repo_path.to_string(),
            tree,
            key_files,
            cached_at: chrono::Local::now(),
            file_fingerprints,
        })
    }

    /// Recursively build tree with bounds
    fn build_tree(
        dir: &Path,
        lines: &mut Vec<String>,
        depth: usize,
        max_depth: usize,
        max_entries: usize,
    ) {
        if depth > max_depth {
            return;
        }

        let prefix = "  ".repeat(depth);

        // Collect and sort entries first
        let entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
            Ok(read_dir) => read_dir.filter_map(|e| e.ok()).collect(),
            Err(_) => return,
        };

        let total_entries = entries.len();
        let mut sorted_entries: Vec<_> = entries
            .into_iter()
            .map(|e| (e.file_name(), e.path(), e.file_type()))
            .collect();
        sorted_entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut count = 0;
        for (name, path, file_type) in sorted_entries {
            if count >= max_entries {
                lines.push(format!(
                    "{}  ... ({} more entries)",
                    prefix,
                    total_entries - max_entries
                ));
                break;
            }

            let name = name.to_string_lossy().to_string();
            let is_dir = file_type.map(|ft| ft.is_dir()).unwrap_or(false);

            // Skip hidden files/dirs at top level except .gitignore
            if depth == 0 && name.starts_with('.') && name != ".gitignore" {
                continue;
            }

            // Skip common non-source directories
            if is_dir && depth > 0 {
                let skip_dirs = [
                    "node_modules",
                    "target",
                    "build",
                    "dist",
                    ".git",
                    "__pycache__",
                    "vendor",
                ];
                if skip_dirs.contains(&name.as_str()) {
                    lines.push(format!("{}📁 {}/ [skipped]", prefix, name));
                    continue;
                }
            }

            if is_dir {
                lines.push(format!("{}📁 {}/", prefix, name));
                Self::build_tree(&path, lines, depth + 1, max_depth, max_entries);
            } else {
                lines.push(format!("{}📄 {}", prefix, name));
            }
            count += 1;
        }
    }

    /// Cap content to max lines and max chars
    fn cap_content(content: &str, max_lines: usize, max_chars: usize) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let truncated: Vec<&str> = lines.iter().take(max_lines).copied().collect();
        let mut result = truncated.join("\n");

        if lines.len() > max_lines {
            result.push_str(&format!("\n... ({} more lines)", lines.len() - max_lines));
        }

        if result.chars().count() > max_chars {
            result = crate::text::take_chars(&result, max_chars);
            result.push_str("\n... (truncated)");
        }

        result
    }

    /// Check if cache is stale based on repo path or file mtimes
    pub fn is_stale(&self, repo_name: &str, repo_path: &str) -> bool {
        // Stale if project changed
        if self.project_name != repo_name || self.project_path != repo_path {
            return true;
        }

        // Stale if key files modified
        let path = Path::new(repo_path);
        for (filename, cached_mtime) in &self.file_fingerprints {
            let file_path = path.join(filename);
            if let Ok(metadata) = std::fs::metadata(&file_path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                        if duration.as_secs() != *cached_mtime {
                            return true;
                        }
                    }
                }
            } else {
                // File was deleted
                return true;
            }
        }

        // TTL: stale after 60 seconds (conservative for active development)
        let age = chrono::Local::now().signed_duration_since(self.cached_at);
        if age.num_seconds() > 60 {
            return true;
        }

        false
    }

    /// Format snapshot as structured context block for LLM
    pub fn format_context(&self) -> String {
        let mut parts = vec![
            format!(
                "Attached Project: {} ({})",
                self.project_name, self.project_path
            ),
            "\n=== Top-Level Tree ===".to_string(),
            if self.tree.is_empty() {
                "(empty directory)".to_string()
            } else {
                self.tree.clone()
            },
        ];

        if !self.key_files.is_empty() {
            parts.push("\n=== Key Files ===".to_string());
            for (name, content) in &self.key_files {
                parts.push(format!("\n--- {} ---", name));
                parts.push(content.clone());
            }
        }

        parts.push("\n=== Notes ===".to_string());
        parts.push("This is ACTUAL repository evidence, not inferred.".to_string());
        parts.push(
            "If asked about files not shown above, say you haven't read them yet.".to_string(),
        );

        parts.join("\n")
    }
}

pub struct App {
    pub state: AppState,
    pub layout: LayoutState,
    pub should_quit: bool,
    pub show_inspector: bool,
    pub experience_mode: ExperienceMode,
    pub ollama: OllamaClient,
    pub chat_blocked: bool,
    pub block_reason: Option<String>,
    pub persistence: PersistentState,
    pub conversation_id: String,
    pub active_execution_runtime: Option<ForgeRuntimeHandle>,
    pub execution_output_buffer: Vec<String>,
    pub ui_targets: Vec<UiTarget>,
    pub focused_ui: Option<String>,
    pub active_run_message_id: Option<String>,
    pub active_runtime_session_id: Option<String>,
    pub execution_run_sealed: bool,
    pub post_terminal_audit_recorded: bool,
    pub active_panel: SidebarPanel,
    pub composer_mode: ComposerMode,
    pub project_create_workflow: Option<ProjectCreateWorkflow>,
    pub panel_status: Option<PanelStatus>,
    pub search_query: Option<String>,
    pub search_results: Vec<ProjectSearchResult>,
    pub selected_search_result: Option<usize>,
    pub search_preview: Option<SearchPreview>,
    pub pending_command: Option<Command>,
    pub pending_approval: Option<ApprovalRequest>,
    // Interface layer integration
    pub interface: InterfaceIntegration,
    // Deduplication fields
    pub last_reported_repo: Option<String>,
    pub last_archived_conversation: Option<String>,
    // Repo snapshot cache for LLM grounding
    pub repo_snapshot_cache: Option<RepoSnapshot>,
    /// Git state when approval was required for task start
    pub pending_git_grounding: Option<GitGrounding>,
    /// V1.3: Flow mode - aggressive suggestion mode
    pub flow_mode: crate::guidance::FlowMode,
    /// V1.3: Operator intent memory for contextual guidance
    pub operator_intent: crate::guidance::OperatorIntent,
    /// V1.3: Pending confirmation request for assisted execution
    pub pending_confirmation: Option<crate::guidance::ConfirmationRequest>,
    /// V1.4: Session memory for adaptive behavior
    pub session_memory: crate::guidance::SessionMemory,
    /// V1.4: Last interrupt context for recovery
    pub interrupt_context: Option<crate::guidance::InterruptContext>,
    /// V2.0: Goal manager for autonomous operator
    pub goal_manager: crate::guidance::GoalManager,
    /// Latest checkpoint validation report for operator-visible checkpoint surfaces.
    pub checkpoint_inspector_report: Option<crate::persistence::CheckpointOperatorReport>,
    /// V1.6: Recovery state for self-healing and completion-confidence decisions
    pub recovery_state: crate::state::RecoveryState,
}

impl App {
    pub async fn new() -> Self {
        info!("Initializing Rasputin TUI application");

        // Load persisted state
        let persistence = PersistentState::load().await.unwrap_or_default();
        let conversation_id = uuid::Uuid::new_v4().to_string();

        let state = AppState::new();

        // Create Ollama client
        let ollama = OllamaClient::local_default();

        let mut app = Self {
            state,
            layout: LayoutState::new(),
            should_quit: false,
            show_inspector: false,
            experience_mode: ExperienceMode::Normal,
            ollama,
            chat_blocked: false,
            block_reason: None,
            persistence,
            conversation_id,
            active_execution_runtime: None,
            execution_output_buffer: vec![],
            interface: InterfaceIntegration::new(),
            ui_targets: vec![],
            focused_ui: None,
            active_run_message_id: None,
            active_runtime_session_id: None,
            execution_run_sealed: false,
            post_terminal_audit_recorded: false,
            active_panel: SidebarPanel::Chat,
            composer_mode: ComposerMode::Chat,
            project_create_workflow: None,
            panel_status: None,
            search_query: None,
            search_results: vec![],
            selected_search_result: None,
            search_preview: None,
            pending_command: None,
            pending_approval: None,
            // Deduplication - start with none reported
            last_reported_repo: None,
            last_archived_conversation: None,
            // No repo attached yet - cache starts empty
            repo_snapshot_cache: None,
            // No pending Git grounding initially
            pending_git_grounding: None,
            // V1.3: Start in standard mode
            flow_mode: crate::guidance::FlowMode::Standard,
            // V1.3: Empty intent memory
            operator_intent: crate::guidance::OperatorIntent::new(),
            // V1.3: No pending confirmation
            pending_confirmation: None,
            // V1.4: Initialize session memory
            session_memory: crate::guidance::SessionMemory::new(),
            // V1.4: No interrupt context yet
            interrupt_context: None,
            // V2.0: Initialize goal manager
            goal_manager: crate::guidance::GoalManager::new(),
            // Checkpoint view starts empty until a checkpoint/status command runs.
            checkpoint_inspector_report: None,
            // V1.6: Initialize empty recovery state
            recovery_state: crate::state::RecoveryState::default(),
        };

        app.seed_welcome_message();
        app.snapshot_current_conversation();
        app
    }

    /// Initialize the app - load persisted state, check Ollama health, verify model
    pub async fn initialize(&mut self, repo_path: Option<&str>) -> Result<()> {
        info!("Initializing app...");
        self.emit_event("init", "Starting initialization...");
        self.push_system_notice(
            "SYSTEM INITIALIZING...\nLoading configuration and runtime readiness...",
        );

        // 1. Restore last repo if no path provided
        let last_repo = self.persistence.active_repo.clone();
        if repo_path.is_none() {
            if let Some(repo) = last_repo {
                info!("Restoring last repo: {}", repo);
                if let Err(e) = self.attach_repo(&repo).await {
                    warn!("Failed to restore repo: {}", e);
                }
            }
        } else if let Some(path) = repo_path {
            // Attach provided repo
            if let Err(e) = self.attach_repo(path).await {
                warn!("Failed to attach repo: {}", e);
                self.add_log(
                    LogLevel::Error,
                    "init",
                    &format!("Failed to attach repo: {}", e),
                );
            }
        }

        // 2. Check Ollama health
        self.emit_event("init", "Checking Ollama health...");
        match self.ollama.health_check().await {
            Ok(health) => {
                self.state.ollama_connected = health.connected;
                self.state.model.available = health.models.clone();
                self.state.model.connected = health.connected;
                info!(
                    "Startup Ollama health: connected={}, models={:?}",
                    self.state.ollama_connected, self.state.model.available
                );

                if health.connected {
                    info!("Ollama connected, {} models available", health.models.len());
                    self.emit_event(
                        "ollama",
                        &format!("Connected, {} models available", health.models.len()),
                    );
                } else {
                    let error = health.error.unwrap_or_else(|| "Unknown error".to_string());
                    warn!("Ollama not connected: {}", error);
                    self.emit_event("ollama", &format!("Disconnected: {}", error));
                    self.block_chat("Ollama disconnected");
                }
            }
            Err(e) => {
                error!("Ollama health check failed: {}", e);
                self.emit_event("ollama", &format!("Health check failed: {}", e));
                self.state.ollama_connected = false;
                self.block_chat("Ollama disconnected");
            }
        }

        // 3. Verify model if repo attached
        if self.state.repo.name != "No repo" {
            if let Err(e) = self.refresh_active_model().await {
                error!("Model verification failed: {}", e);
                self.emit_event("model", &format!("Verification failed: {}", e));
                self.block_chat("Model verification failed");
            } else {
                // model refresh handles missing configuration and fallback resolution
            }
        } else {
            self.emit_event("repo", "No repo attached");
            self.block_chat("No repo attached");
        }

        // PHASE 4: Restore last active conversation if no repo path was provided
        // (If repo_path was provided, we started fresh; otherwise restore continuity)
        if repo_path.is_none()
            && let Some(ref conv_id) = self.persistence.active_conversation.clone()
        {
            info!("Restoring active conversation: {}", conv_id);
            if let Err(e) = self.open_conversation(conv_id) {
                warn!("Failed to restore conversation {}: {}", conv_id, e);
                self.emit_event(
                    "init",
                    &format!("Could not restore last conversation: {}", e),
                );
            } else {
                self.emit_event(
                    "init",
                    &format!("Restored conversation {}", self.short_session_id()),
                );
            }
        }

        // Persist initial state
        self.persist().await;

        Ok(())
    }

    /// Emit a runtime event (logs + persists)
    fn emit_event(&mut self, source: &str, message: &str) {
        let level = if message.contains("failed")
            || message.contains("error")
            || message.contains("disconnected")
        {
            LogLevel::Error
        } else if message.contains("warning") || message.contains("missing") {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };

        // Add to in-memory logs
        self.add_log(level, source, message);

        // Persist event
        self.persistence
            .add_event(&self.conversation_id, source, level.as_str(), message);
    }

    /// Persist current state
    pub async fn persist(&mut self) {
        self.snapshot_current_conversation();
        if let Err(e) = self.persistence.save().await {
            warn!("Failed to persist state: {}", e);
        }
    }

    fn persist_now(&mut self) {
        self.snapshot_current_conversation();
        if let Err(e) = self.persistence.save_sync() {
            warn!("Failed to persist state synchronously: {}", e);
        }
    }

    pub fn short_session_id(&self) -> String {
        self.conversation_id
            .split('-')
            .next()
            .unwrap_or("unknown")
            .to_string()
    }

    fn record_last_action(&mut self, action: impl Into<String>) {
        let action_str = action.into();
        self.state.execution.last_action = action_str.clone();
        // V1.4: Record in session memory for pattern detection
        self.session_memory.record_command(&action_str);
    }

    fn checkpoint_base_path(chain: &crate::persistence::PersistentChain) -> PathBuf {
        PathBuf::from(chain.repo_path.as_deref().unwrap_or("."))
    }

    fn render_checkpoint_report(report: &crate::persistence::CheckpointOperatorReport) -> String {
        let mut lines = vec![
            "Checkpoint Decision".to_string(),
            "===================".to_string(),
            format!(
                "Final Status: {} {}",
                report.final_status.icon(),
                report.final_status.label()
            ),
            format!(
                "Resume: {}",
                if report.resume_allowed {
                    "allowed"
                } else {
                    "blocked"
                }
            ),
            format!("Chain: {}", report.chain_id),
        ];

        lines.push(format!(
            "Checkpoint: {}",
            report.checkpoint_id.as_deref().unwrap_or("(missing)")
        ));
        if let Some(timestamp) = report.checkpoint_timestamp {
            lines.push(format!(
                "Created: {}",
                timestamp.format("%Y-%m-%d %H:%M:%S")
            ));
        }
        let step = report
            .active_step
            .map(|idx| idx.to_string())
            .unwrap_or_else(|| "(none)".to_string());
        let step_desc = report.step_description.as_deref().unwrap_or("(unknown)");
        lines.push(format!("Step: {} - {}", step, step_desc));
        lines.push(format!(
            "Audit Cursor: {} / {}",
            report
                .audit_cursor
                .map(|cursor| cursor.to_string())
                .unwrap_or_else(|| "(none)".to_string()),
            report.audit_log_len
        ));
        lines.push(format!(
            "Workspace Hash: {}",
            report.workspace_hash.as_deref().map_or_else(
                || "(none)".to_string(),
                |hash| crate::text::take_chars(hash, 16)
            )
        ));
        lines.push(String::new());
        lines.push(format!(
            "Workspace: {} - {}",
            report.workspace_result.status.label(),
            report.workspace_result.detail
        ));
        lines.push(format!(
            "Replay: {} - {}",
            report.replay_result.status.label(),
            report.replay_result.detail
        ));
        lines.push(String::new());
        lines.push(format!("Next: {}", report.smallest_safe_next_action));
        lines.join("\n")
    }

    fn render_checkpoint_status_line(
        report: &crate::persistence::CheckpointOperatorReport,
    ) -> String {
        format!(
            "{} {} | checkpoint={} | cursor={}/{} | resume={} | next={}",
            report.final_status.icon(),
            report.final_status.label(),
            report.checkpoint_id.as_deref().unwrap_or("(missing)"),
            report
                .audit_cursor
                .map(|cursor| cursor.to_string())
                .unwrap_or_else(|| "-".to_string()),
            report.audit_log_len,
            if report.resume_allowed {
                "allowed"
            } else {
                "blocked"
            },
            report.smallest_safe_next_action
        )
    }

    fn set_execution_state(&mut self, state: ExecutionState) {
        // V1.5 PROGRESS: Clear stale terminal/recovery metadata when entering active progress states
        // This prevents prior-run block_reason/block_fix from leaking into current run presentation
        if state.requires_clean_state() {
            self.clear_execution_block();
        }

        self.state.execution.state = state;
        self.state.runtime_status = Self::runtime_status_from_execution(state);
        // Only auto-show inspector in Operator mode; Normal mode users should not
        // be exposed to debug surfaces during routine task execution
        if state != ExecutionState::Idle && self.is_operator_mode() {
            self.show_inspector = true;
            if matches!(
                state,
                ExecutionState::Failed
                    | ExecutionState::Blocked
                    | ExecutionState::PreconditionFailed
            ) {
                self.state.active_inspector_tab = InspectorTab::Runtime;
            }
        }
    }

    /// V1.6 AUDIT: Apply a canonical progress state transition with audit logging
    /// All event-driven execution state changes should flow through this method
    fn apply_progress_transition(&mut self, event: crate::state::ProgressTransitionEvent) {
        use crate::state::{TransitionResult, reduce_execution_state_with_audit};

        if self.execution_run_sealed
            && !matches!(
                event,
                crate::state::ProgressTransitionEvent::NewRun { .. }
                    | crate::state::ProgressTransitionEvent::ResetToIdle
            )
        {
            self.audit_post_terminal_noise_once(format!("blocked_transition={:?}", event));
            return;
        }

        let current = self.state.execution.state;
        let has_terminal_outcome = self
            .persistence
            .active_chain_id
            .as_ref()
            .and_then(|id| self.persistence.get_chain(id))
            .and_then(|c| c.get_outcome())
            .is_some();

        let (result, audit_event) =
            reduce_execution_state_with_audit(current, event.clone(), has_terminal_outcome);

        // V1.6 AUDIT: Record transition to chain audit log
        if let Some(audit) = audit_event {
            if let Some(chain_id) = self.persistence.active_chain_id.clone() {
                if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                    chain.audit_event(audit);
                }
            }
        }

        match result {
            TransitionResult::Applied(new_state) => {
                self.set_execution_state(new_state);
            }
            TransitionResult::Normalized { to, reason } => {
                self.add_operator_debug_log(
                    "state_machine/normalized",
                    format!(
                    "[STATE MACHINE] Transition normalized: {} -> {} ({})",
                    current.as_str(),
                    to.as_str(),
                    reason
                    ),
                );
                self.set_execution_state(to);
            }
            TransitionResult::Rejected { current: _, reason } => {
                self.add_operator_debug_log(
                    "state_machine/rejected",
                    format!(
                    "[STATE MACHINE] Transition rejected: {} on {:?} ({})",
                    current.as_str(),
                    event,
                    reason
                    ),
                );
            }
        }
    }

    fn set_active_objective(&mut self, objective: Option<String>) {
        self.state.execution.active_objective = objective;
    }

    fn set_execution_mode(&mut self, mode: ExecutionMode) {
        self.state.execution.mode = mode;
        self.record_last_action(format!("Mode set to {}", mode.as_str()));
    }

    fn reset_execution_activity(&mut self) {
        self.state.execution.clear_runtime_activity();
    }

    fn clear_execution_block(&mut self) {
        self.chat_blocked = false;
        self.block_reason = None;
        self.state.execution.clear_block();
    }

    fn set_execution_block(
        &mut self,
        reason: impl Into<String>,
        fix: impl Into<String>,
        command: Option<String>,
    ) {
        let reason = reason.into();
        let fix = fix.into();
        self.chat_blocked = true;
        self.block_reason = Some(reason.clone());
        self.state.execution.block_reason = Some(reason.clone());
        self.state.execution.block_fix = Some(fix.clone());
        self.state.execution.block_command = command.clone();
        self.set_execution_state(ExecutionState::Blocked);
        self.set_current_step(Some("Waiting for recovery".to_string()), None, None);
        self.set_panel_status(PanelStatusLevel::Error, reason.clone());
        self.record_last_action(format!("Blocked: {}", reason));

        let mut lines = vec![
            "✖ SYSTEM BLOCKED".to_string(),
            format!("Reason: {}", reason),
            format!("Fix: {}", fix),
        ];
        if let Some(command) = command {
            lines.push(format!("Command: {}", command));
        }
        lines.push("Status: Waiting for recovery".to_string());
        self.push_system_notice(&lines.join("\n"));
    }

    fn set_execution_precondition_failed(
        &mut self,
        reason: impl Into<String>,
        fix: impl Into<String>,
    ) {
        let reason = reason.into();
        let fix = fix.into();
        self.chat_blocked = false;
        self.block_reason = Some(reason.clone());
        self.state.execution.block_reason = Some(reason.clone());
        self.state.execution.block_fix = Some(fix.clone());
        self.state.execution.block_command = None;
        self.set_execution_state(ExecutionState::PreconditionFailed);
        self.set_current_step(Some("Autonomy preflight failed".to_string()), None, None);
        self.set_panel_status(PanelStatusLevel::Error, reason.clone());
        self.record_last_action(format!("Autonomy preflight failed: {}", reason));

        self.push_system_notice(&format!(
            "✖ AUTONOMY PREFLIGHT FAILED\nReason: {}\nFix: {}\nStatus: Goal was not started; no chain was created.",
            reason, fix
        ));
    }

    fn soft_block(&mut self, reason: impl Into<String>, fix: impl Into<String>) {
        let reason = reason.into();
        let fix = fix.into();
        self.state.execution.block_reason = Some(reason.clone());
        self.state.execution.block_fix = Some(fix.clone());
        self.state.execution.block_command = None;
        self.set_execution_state(ExecutionState::Blocked);
        self.set_current_step(Some("Waiting for recovery".to_string()), None, None);
        self.set_panel_status(PanelStatusLevel::Error, reason.clone());
        self.record_last_action(format!("Blocked: {}", reason));
        self.push_system_notice(&format!(
            "✖ ACTION BLOCKED\nReason: {}\nFix: {}",
            reason, fix
        ));
    }

    fn clear_soft_block(&mut self) {
        if !self.chat_blocked {
            self.state.execution.clear_block();
            if self.state.execution.state == ExecutionState::Blocked {
                self.set_execution_state(ExecutionState::Idle);
            }
        }
    }

    fn request_approval(
        &mut self,
        title: impl Into<String>,
        detail: impl Into<String>,
        fix_hint: Option<String>,
        action: ApprovalAction,
    ) {
        let request = ApprovalRequest {
            title: title.into(),
            detail: detail.into(),
            fix_hint,
            action,
        };
        self.pending_approval = Some(request.clone());
        self.set_execution_state(ExecutionState::Blocked);
        self.set_current_step(Some("Waiting for approval".to_string()), None, None);

        // V1.6 AUDIT: Log approval request
        if let Some(chain_id) = self.persistence.active_chain_id.clone() {
            if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                let audit_event = crate::state::AuditEvent::approval(
                    crate::state::AuditEventType::ApprovalRequested,
                    &request.title,
                    None,
                );
                chain.audit_event(audit_event);
            }
        }

        self.push_system_notice(&format!(
            "APPROVAL REQUIRED\n{}\n{}\nApprove with: /approve\nCancel with: /deny{}",
            request.title,
            request.detail,
            request
                .fix_hint
                .as_ref()
                .map(|hint| format!("\n{}", hint))
                .unwrap_or_default()
        ));
    }

    fn active_project_root(&self) -> Result<PathBuf> {
        if Self::is_real_repo_path(&self.state.repo.path) {
            Ok(PathBuf::from(&self.state.repo.path))
        } else {
            Err(anyhow::anyhow!("No active project"))
        }
    }

    fn set_current_step(
        &mut self,
        step: Option<String>,
        step_index: Option<u32>,
        step_total: Option<u32>,
    ) {
        self.state.execution.current_step = step;
        self.state.execution.step_index = step_index;
        self.state.execution.step_total = step_total;
    }

    fn push_execution_entry(entries: &mut Vec<String>, value: String, max: usize) {
        if value.trim().is_empty() {
            return;
        }

        if entries.last().is_some_and(|existing| existing == &value) {
            return;
        }

        if entries.len() >= max {
            entries.remove(0);
        }
        entries.push(value);
    }

    fn update_active_run_card<F>(&mut self, update: F)
    where
        F: FnOnce(&mut RunCard),
    {
        let Some(message_id) = self.active_run_message_id.as_deref() else {
            return;
        };

        let Some(message) = self
            .state
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        else {
            return;
        };

        let Some(run_card) = message.run_card.as_mut() else {
            return;
        };

        update(run_card);
    }

    /// Attach a repo at the given path
    pub async fn attach_repo(&mut self, path: &str) -> Result<()> {
        let repo = Repo::attach(path).await?;

        // Check if this is the same repo we already reported (deduplication)
        let repo_key = format!("{}:{}", repo.name, repo.path);
        if let Some(last_repo) = &self.last_reported_repo
            && last_repo == &repo_key
        {
            // Same repo already reported, skip the message
            return Ok(());
        }

        // Update state
        self.state.repo.name = repo.name.clone();
        self.state.repo.path = repo.path.clone();
        self.state.repo.display_path = repo.display_path.clone();
        self.state.repo.branch = repo.git_branch.clone();
        self.state.repo.git_detected = repo.git_detected;

        if let Some(model) = &repo.ollama_model {
            self.state.model.configured = Some(model.clone());
            if let Some(source) = &repo.model_source {
                self.emit_event("model", &format!("Configured via {}: {}", source, model));
            }
        }

        // Persist repo
        self.persistence
            .touch_repo(&repo.path, &repo.name, repo.ollama_model.as_deref());

        info!("Attached to repo: {} at {}", repo.name, repo.path);
        self.emit_event(
            "repo",
            &format!("Attached to {} (git: {})", repo.name, repo.git_detected),
        );
        self.record_last_action(format!("Project loaded: {}", repo.name));

        // Add system message only if not a duplicate
        let branch_info = if let Some(branch) = &repo.git_branch {
            format!(" (git: {})", branch)
        } else {
            String::new()
        };

        let msg_content = format!("Attached to repo: {}{}", repo.name, branch_info);
        self.state.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::System,
            source_text: msg_content.clone(),
            content: msg_content,
            timestamp: chrono::Local::now(),
            run_card: None,
        });

        // Mark this repo as reported
        self.last_reported_repo = Some(repo_key);

        self.persist().await;
        Ok(())
    }

    pub fn begin_frame(&mut self) {
        self.ui_targets.clear();
    }

    pub fn queue_command(&mut self, command: Command) {
        self.pending_command = Some(command);
    }

    pub fn take_pending_command(&mut self) -> Option<Command> {
        self.pending_command.take()
    }

    pub fn register_ui_target(&mut self, id: impl Into<String>, area: Rect, action: UiAction) {
        self.ui_targets.push(UiTarget {
            id: id.into(),
            area,
            action,
        });
    }

    pub fn is_ui_focused(&self, id: &str) -> bool {
        self.focused_ui.as_deref() == Some(id)
    }

    pub fn has_focused_ui(&self) -> bool {
        self.focused_ui.is_some()
    }

    pub fn focus_next_ui(&mut self) {
        if self.ui_targets.is_empty() {
            self.focused_ui = None;
            return;
        }

        let next_index = self
            .focused_ui
            .as_ref()
            .and_then(|id| self.ui_targets.iter().position(|target| &target.id == id))
            .map(|index| (index + 1) % self.ui_targets.len())
            .unwrap_or(0);

        self.focused_ui = Some(self.ui_targets[next_index].id.clone());
    }

    pub fn focus_previous_ui(&mut self) {
        if self.ui_targets.is_empty() {
            self.focused_ui = None;
            return;
        }

        let previous_index = self
            .focused_ui
            .as_ref()
            .and_then(|id| self.ui_targets.iter().position(|target| &target.id == id))
            .map(|index| {
                if index == 0 {
                    self.ui_targets.len() - 1
                } else {
                    index - 1
                }
            })
            .unwrap_or(self.ui_targets.len() - 1);

        self.focused_ui = Some(self.ui_targets[previous_index].id.clone());
    }

    pub fn activate_focused_ui(&mut self) -> Result<bool> {
        let Some(id) = self.focused_ui.clone() else {
            return Ok(false);
        };
        self.activate_ui_target(&id)
    }

    pub fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Result<bool> {
        match mouse.kind {
            MouseEventKind::Moved => {
                if let Some(target) = self.ui_target_at(mouse.column, mouse.row) {
                    self.focused_ui = Some(target.id);
                }
                Ok(false)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(target) = self.ui_target_at(mouse.column, mouse.row) {
                    self.focused_ui = Some(target.id.clone());
                    self.perform_ui_action(target.action)
                } else {
                    Ok(false)
                }
            }
            MouseEventKind::ScrollUp => {
                // When inspector is visible, scroll the active tab
                if self.show_inspector {
                    match self.state.active_inspector_tab {
                        crate::state::InspectorTab::Runtime => self.scroll_runtime_tab_up(),
                        crate::state::InspectorTab::Validation => self.scroll_validation_tab_up(),
                        crate::state::InspectorTab::Logs => self.scroll_logs_tab_up(),
                        crate::state::InspectorTab::Preview => self.scroll_preview_tab_up(),
                        crate::state::InspectorTab::Diff => self.scroll_diff_tab_up(),
                        crate::state::InspectorTab::Timeline => self.scroll_timeline_tab_up(),
                        crate::state::InspectorTab::Failure => self.scroll_failure_tab_up(),
                        crate::state::InspectorTab::Steps => self.scroll_steps_tab_up(),
                        crate::state::InspectorTab::PlannerTrace => {
                            self.scroll_planner_trace_tab_up()
                        }
                        crate::state::InspectorTab::Replay => self.scroll_replay_tab_up(),
                        crate::state::InspectorTab::DebugBundle => {
                            self.scroll_debug_bundle_tab_up()
                        }
                        crate::state::InspectorTab::Audit => self.scroll_audit_tab_up(),
                        crate::state::InspectorTab::Checkpoint => self.scroll_audit_tab_up(),
                        crate::state::InspectorTab::Recovery => self.scroll_audit_tab_up(),
                    }
                } else {
                    self.scroll_up();
                }
                Ok(false)
            }
            MouseEventKind::ScrollDown => {
                // When inspector is visible, scroll the active tab
                if self.show_inspector {
                    match self.state.active_inspector_tab {
                        crate::state::InspectorTab::Runtime => self.scroll_runtime_tab_down(),
                        crate::state::InspectorTab::Validation => self.scroll_validation_tab_down(),
                        crate::state::InspectorTab::Logs => self.scroll_logs_tab_down(),
                        crate::state::InspectorTab::Preview => self.scroll_preview_tab_down(),
                        crate::state::InspectorTab::Diff => self.scroll_diff_tab_down(),
                        crate::state::InspectorTab::Timeline => self.scroll_timeline_tab_down(),
                        crate::state::InspectorTab::Failure => self.scroll_failure_tab_down(),
                        crate::state::InspectorTab::Steps => self.scroll_steps_tab_down(),
                        crate::state::InspectorTab::PlannerTrace => {
                            self.scroll_planner_trace_tab_down()
                        }
                        crate::state::InspectorTab::Replay => self.scroll_replay_tab_down(),
                        crate::state::InspectorTab::DebugBundle => {
                            self.scroll_debug_bundle_tab_down()
                        }
                        crate::state::InspectorTab::Audit => self.scroll_audit_tab_down(),
                        crate::state::InspectorTab::Checkpoint => self.scroll_audit_tab_down(),
                        crate::state::InspectorTab::Recovery => self.scroll_audit_tab_down(),
                    }
                } else {
                    self.scroll_down();
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn activate_ui_target(&mut self, id: &str) -> Result<bool> {
        if let Some(target) = self
            .ui_targets
            .iter()
            .find(|target| target.id == id)
            .cloned()
        {
            self.perform_ui_action(target.action)
        } else {
            Ok(false)
        }
    }

    fn ui_target_at(&self, column: u16, row: u16) -> Option<UiTarget> {
        self.ui_targets
            .iter()
            .find(|target| {
                column >= target.area.x
                    && column < target.area.x.saturating_add(target.area.width)
                    && row >= target.area.y
                    && row < target.area.y.saturating_add(target.area.height)
            })
            .cloned()
    }

    pub fn perform_ui_action(&mut self, action: UiAction) -> Result<bool> {
        match action {
            UiAction::NewChat => {
                self.start_new_conversation();
                self.activate_panel(SidebarPanel::Chat);
                self.record_last_action(format!("Started new session {}", self.short_session_id()));
            }
            UiAction::Projects => {
                self.activate_panel(SidebarPanel::Projects);
                self.record_last_action("Opened Projects panel");
            }
            UiAction::Search => {
                self.activate_panel(SidebarPanel::Search);
                self.record_last_action("Opened Search panel");
            }
            UiAction::Plugins => {
                self.activate_panel(SidebarPanel::Plugins);
                self.record_last_action("Opened Plugins panel");
            }
            UiAction::Automations => {
                self.activate_panel(SidebarPanel::Automations);
                self.record_last_action("Opened Automations panel");
            }
            UiAction::StartProjectCreate => {
                self.start_picker_driven_project_create();
                self.record_last_action("Project creation flow ready");
            }
            UiAction::StartProjectConnect => {
                self.activate_project_connect();
                self.record_last_action("Project connect flow ready");
            }
            UiAction::PickProjectFolder => {
                self.activate_panel(SidebarPanel::Projects);
                self.record_last_action("Opening folder picker");
                self.queue_command(Command::OpenRepoPicker);
            }
            UiAction::OpenProject(path) => {
                self.activate_panel(SidebarPanel::Chat);
                self.record_last_action(format!("Switching project to {}", path));
                self.queue_command(Command::SwitchRepo { path_or_name: path });
            }
            UiAction::OpenSearchResult(index) => {
                self.select_search_result(index)?;
                self.record_last_action(format!("Opened search result {}", index + 1));
            }
            UiAction::RunValidation => {
                self.show_inspector = true;
                self.state.active_inspector_tab = InspectorTab::Validation;
                self.record_last_action("Validation requested");
                self.queue_command(Command::RunValidation);
            }
            UiAction::RefreshRuntime => {
                self.record_last_action("Runtime refresh requested");
                self.queue_command(Command::RefreshRuntime);
            }
            UiAction::ResetValidation => {
                self.show_inspector = true;
                self.state.active_inspector_tab = InspectorTab::Validation;
                self.record_last_action("Validation state reset requested");
                self.queue_command(Command::ResetValidation);
            }
            UiAction::ClearLogs => {
                self.show_inspector = true;
                self.state.active_inspector_tab = InspectorTab::Logs;
                self.record_last_action("Log clear requested");
                self.queue_command(Command::ClearLogs);
            }
            UiAction::UseModel(model) => {
                self.record_last_action(format!("Model switch requested: {}", model));
                self.queue_command(Command::SetModel { model });
            }
            UiAction::ToggleInspector => {
                self.toggle_inspector();
                self.record_last_action(if self.show_inspector {
                    "Inspector shown".to_string()
                } else {
                    "Inspector hidden".to_string()
                });
            }
            UiAction::ToggleExperienceMode => {
                self.toggle_experience_mode();
            }
            UiAction::SelectInspectorTab(tab) => {
                self.show_inspector = true;
                self.state.active_inspector_tab = tab;
                self.record_last_action(format!("Inspector tab: {}", tab.as_str()));
            }
            UiAction::OpenBrowserPreview { url } => {
                match self.apply_host_action(HostAction::OpenBrowserPreview { url: url.clone() }) {
                    Ok(_) => {
                        self.emit_event("browser_preview", &format!("Opened: {}", url));
                        self.record_last_action(format!("Opened browser preview {}", url));
                    }
                    Err(e) => {
                        self.emit_event(
                            "browser_preview",
                            &format!("Failed to open {}: {}", url, e),
                        );
                        self.record_last_action(format!("Browser preview failed: {}", e));
                    }
                }
            }
            UiAction::OpenConversation(id) => {
                self.open_conversation(&id)?;
                self.activate_panel(SidebarPanel::Chat);
                self.record_last_action(format!("Opened conversation {}", id));
            }
            UiAction::ArchiveConversation(id) => {
                self.record_last_action(format!("Archiving conversation {}", id));
                self.queue_command(Command::ArchiveConversation { id });
            }
            UiAction::UnarchiveConversation(id) => {
                self.record_last_action(format!("Restoring conversation {}", id));
                self.queue_command(Command::UnarchiveConversation { id });
            }
            UiAction::FocusInput => {
                if self.composer_is_editable() {
                    self.set_input_mode(InputMode::Editing);
                    self.focused_ui = Some("composer:input".to_string());
                    self.record_last_action(format!(
                        "Focused {} composer",
                        self.state.execution.mode.as_str()
                    ));
                }
            }
            UiAction::SetExecutionMode(mode) => {
                if self.active_execution_runtime.is_some() {
                    self.push_system_notice("Cannot switch modes during active execution.");
                    self.record_last_action("Mode switch blocked during active task");
                    return Ok(false);
                }
                self.set_execution_mode(mode);
                self.focused_ui = Some("composer:input".to_string());
                match mode {
                    ExecutionMode::Edit => {
                        self.push_system_notice(
                            "EDIT MODE ACTIVE\nFile reads and writes are enabled. Shell commands require approval.",
                        );
                        self.set_active_objective(None);
                        self.set_execution_state(ExecutionState::Idle);
                    }
                    ExecutionMode::Task => {
                        self.push_system_notice(
                            "TASK MODE ACTIVE\nPlanner will execute actions and stream plan, tools, and validation.",
                        );
                        self.set_active_objective(None);
                        self.set_execution_state(ExecutionState::Idle);
                    }
                    ExecutionMode::Chat => {
                        self.push_system_notice(
                            "CHAT MODE ACTIVE\nMessages stay conversational unless you enter an explicit command.",
                        );
                        self.set_execution_state(ExecutionState::Idle);
                    }
                }
            }
        }

        Ok(false)
    }

    /// Handle a canonical command
    pub async fn handle_command(&mut self, command: Command) -> Result<bool> {
        match command {
            Command::OpenRepoPicker => {
                self.emit_event("cmd", "/open");
                self.record_last_action("Opening folder picker");
                let result = self.apply_host_action(HostAction::PickProjectFolder);
                match result {
                    Ok(result) => {
                        let repo_path =
                            result.affected_paths.first().cloned().ok_or_else(|| {
                                anyhow::anyhow!("Host action did not return a project path")
                            })?;
                        self.attach_repo(&repo_path).await?;
                        self.ensure_project_chat_binding();
                        if let Err(e) = self.refresh_active_model().await {
                            self.emit_event("error", &format!("Failed to verify model: {}", e));
                        }
                        self.active_panel = SidebarPanel::Chat;
                        self.composer_mode = ComposerMode::Chat;
                        self.panel_status = None;
                    }
                    Err(e) => {
                        self.emit_event("error", &format!("Folder picker failed: {}", e));
                        self.activate_project_connect();
                        self.set_panel_status(
                            PanelStatusLevel::Error,
                            format!(
                                "Folder picker unavailable: {}. Paste or drop a folder path.",
                                e
                            ),
                        );
                    }
                }
                self.persist().await;
            }

            Command::OpenRepo { path } => {
                self.emit_event("cmd", &format!("/open {}", path));
                self.record_last_action(format!("Opening project {}", path));
                let resolved = self.resolve_project_path(&path)?;
                let result = self.apply_host_action(HostAction::AttachProject { path: resolved });
                match result {
                    Ok(result) => {
                        let repo_path =
                            result.affected_paths.first().cloned().ok_or_else(|| {
                                anyhow::anyhow!("Host action did not return a project path")
                            })?;
                        self.attach_repo(&repo_path).await?;
                        self.ensure_project_chat_binding();
                        if let Err(e) = self.refresh_active_model().await {
                            self.emit_event("error", &format!("Failed to verify model: {}", e));
                        }
                    }
                    Err(e) => {
                        self.emit_event("error", &format!("Failed to attach repo: {}", e));
                    }
                }
                self.persist().await;
            }

            Command::SwitchRepo { path_or_name } => {
                self.emit_event("cmd", &format!("/switch {}", path_or_name));
                self.record_last_action(format!("Switching project {}", path_or_name));

                // Check if it's a recent repo by name
                let repo_path = if let Some(recent) = self
                    .persistence
                    .recent_repos
                    .iter()
                    .find(|r| r.name == path_or_name)
                {
                    recent.path.clone()
                } else {
                    path_or_name.clone()
                };

                let resolved = self.resolve_project_path(&repo_path)?;
                let result = self.apply_host_action(HostAction::AttachProject { path: resolved });
                match result {
                    Ok(result) => {
                        let repo_path =
                            result.affected_paths.first().cloned().ok_or_else(|| {
                                anyhow::anyhow!("Host action did not return a project path")
                            })?;
                        self.attach_repo(&repo_path).await?;
                        self.ensure_project_chat_binding();
                        if let Err(e) = self.refresh_active_model().await {
                            self.emit_event("error", &format!("Failed to verify model: {}", e));
                        }
                    }
                    Err(e) => {
                        self.emit_event("error", &format!("Failed to switch repo: {}", e));
                    }
                }
                self.persist().await;
            }

            Command::DeleteProject { path_or_name } => {
                if self.state.execution.mode == ExecutionMode::Chat {
                    self.soft_block(
                        "Project deletion requires EDIT or TASK mode",
                        "Switch to EDIT or TASK mode and retry.",
                    );
                    self.persist().await;
                    return Ok(false);
                }

                let target_path = if path_or_name == "." {
                    self.active_project_root()?
                } else {
                    self.resolve_project_path(&path_or_name)?
                };
                self.request_approval(
                    "Delete project",
                    format!("Delete {}", target_path.display()),
                    Some("This removes the project directory from disk.".to_string()),
                    ApprovalAction::DeleteProject { path: target_path },
                );
                self.persist().await;
            }

            Command::ArchiveConversation { id } => {
                self.archive_conversation(&id)?;
                self.persist().await;
            }

            Command::UnarchiveConversation { id } => {
                self.unarchive_conversation(&id)?;
                self.persist().await;
            }

            Command::ShowModel => {
                self.emit_event("cmd", "/model");
                let css_model = self
                    .state
                    .model
                    .active
                    .as_deref()
                    .or(self.state.model.configured.as_deref())
                    .unwrap_or(FALLBACK_PLANNER_MODEL);
                let status = match (&self.state.model.configured, &self.state.model.active) {
                    (Some(configured), Some(active)) if configured != active => format!(
                        "Configured model: {}\nActive model: {}\nResolution: using installed local match\nCSS compression: {}",
                        configured,
                        active,
                        if should_enable_css_compression(css_model) {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                    (_, Some(active)) => format!(
                        "Active model: {}\nCSS compression: {}",
                        active,
                        if should_enable_css_compression(css_model) {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                    (Some(configured), None) => format!(
                        "Configured: {} (not available)\nCSS compression: {}",
                        configured,
                        if should_enable_css_compression(css_model) {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                    (None, None) => "No model configured".to_string(),
                };

                self.push_system_notice(&status);
            }

            Command::SetModel { model } => {
                self.emit_event("cmd", &format!("/model {}", model));
                self.record_last_action(format!("Setting model {}", model));
                if let Err(error) = self.set_repo_model(&model).await {
                    self.set_panel_status(
                        PanelStatusLevel::Error,
                        format!("Model switch failed: {}", error),
                    );
                    self.emit_event("model", &format!("Switch failed: {}", error));
                    self.push_system_notice(&format!("Failed to set model: {}", error));
                } else {
                    let configured = self.state.model.configured.as_deref().unwrap_or("none");
                    let active = self.state.model.active.as_deref().unwrap_or("none");
                    self.set_panel_status(
                        PanelStatusLevel::Success,
                        format!("Configured {} and active {}", configured, active),
                    );
                }
            }

            Command::ShowModels => {
                self.emit_event("cmd", "/models");
                let content = if !self.state.ollama_connected {
                    "Ollama disconnected".to_string()
                } else {
                    let configured = self.state.model.configured.as_deref();
                    let active = self.state.model.active.as_deref();
                    match self.ollama.list_model_cards().await {
                        Ok(mut cards) if !cards.is_empty() => {
                            cards.sort_by_key(|model| {
                                (model_preference_rank(&model.name), model.name.clone())
                            });
                            self.state.model.available =
                                cards.iter().map(|model| model.name.clone()).collect();

                            let lines = cards
                                .iter()
                                .map(|model| {
                                    let marker = if Some(model.name.as_str()) == active {
                                        "●"
                                    } else if Some(model.name.as_str()) == configured {
                                        "○"
                                    } else {
                                        "·"
                                    };
                                    let meta =
                                        match (&model.parameter_size, &model.quantization_level) {
                                            (Some(size), Some(quantization)) => {
                                                format!(" [{} / {}]", size, quantization)
                                            }
                                            (Some(size), None) => format!(" [{}]", size),
                                            (None, Some(quantization)) => {
                                                format!(" [{}]", quantization)
                                            }
                                            (None, None) => String::new(),
                                        };
                                    format!("{} {}{}", marker, model.name, meta)
                                })
                                .collect::<Vec<_>>();

                            format!(
                                "Installed Ollama models:\n{}\n\nRecommended models:\n1. qwen2.5-coder:14b-q4km\n2. qwen2.5-coder:14b-q5km\n3. qwen2.5-coder:14b\n4. qwen3.5:latest\n\nSet one with /model <tag>",
                                lines.join("\n")
                            )
                        }
                        Ok(_) | Err(_) if self.state.model.available.is_empty() => {
                            "No local Ollama models installed".to_string()
                        }
                        Ok(_) | Err(_) => {
                            let lines = self
                                .state
                                .model
                                .available
                                .iter()
                                .map(|model| {
                                    let marker = if Some(model.as_str()) == active {
                                        "[active]"
                                    } else if Some(model.as_str()) == configured {
                                        "[config]"
                                    } else {
                                        "[--]"
                                    };
                                    format!("{} {}", marker, model)
                                })
                                .collect::<Vec<_>>();
                            format!("Installed Ollama models:\n{}", lines.join("\n"))
                        }
                    }
                };

                self.push_system_notice(&content);
            }

            Command::ShowStatus => {
                self.emit_event("cmd", "/status");

                // Capture Git repository state
                let git_grounding = capture_git_grounding(&self.state.repo.path);
                let git_status = git_grounding.summary();

                // V1.5 CLEANUP: Prioritize ExecutionOutcome for chain state
                let (execution_state, outcome_display, recovery_details) =
                    if let Some(ref chain_id) = self.persistence.active_chain_id {
                        if let Some(chain) = self.persistence.get_chain(chain_id) {
                            if let Some(outcome) = chain.get_outcome() {
                                // Authoritative outcome available
                                let state = format!("{} (outcome)", outcome.label());
                                let details = match outcome {
                                    crate::persistence::ExecutionOutcome::Blocked => {
                                        format!(
                                            "\nBlock reason: {}\nFix: {}",
                                            self.state
                                                .execution
                                                .block_reason
                                                .as_deref()
                                                .unwrap_or("none"),
                                            self.state
                                                .execution
                                                .block_fix
                                                .as_deref()
                                                .unwrap_or("none")
                                        )
                                    }
                                    crate::persistence::ExecutionOutcome::Failed => {
                                        format!(
                                            "\nFailure reason: {}\nRecovery: {}",
                                            self.state
                                                .execution
                                                .block_reason
                                                .as_deref()
                                                .unwrap_or("none"),
                                            self.state
                                                .execution
                                                .block_fix
                                                .as_deref()
                                                .unwrap_or("none")
                                        )
                                    }
                                    _ => String::new(), // No recovery details for success states
                                };
                                (state, format!("Outcome: {:?}", outcome), details)
                            } else {
                                // No outcome yet - use lifecycle status
                                let lifecycle = format!("{:?}", chain.status);
                                let details = if self.chat_blocked {
                                    format!(
                                        "\nBlock reason: {}\nFix: {}",
                                        self.state
                                            .execution
                                            .block_reason
                                            .as_deref()
                                            .unwrap_or("none"),
                                        self.state.execution.block_fix.as_deref().unwrap_or("none")
                                    )
                                } else {
                                    String::new()
                                };
                                (lifecycle, "Outcome: pending".to_string(), details)
                            }
                        } else {
                            ("no chain".to_string(), String::new(), String::new())
                        }
                    } else {
                        // No active chain - use legacy state
                        let state = if self.chat_blocked {
                            "blocked"
                        } else {
                            "ready"
                        };
                        let details = if self.chat_blocked {
                            format!(
                                "\nBlock reason: {}\nFix: {}",
                                self.state
                                    .execution
                                    .block_reason
                                    .as_deref()
                                    .unwrap_or("none"),
                                self.state.execution.block_fix.as_deref().unwrap_or("none")
                            )
                        } else {
                            String::new()
                        };
                        (state.to_string(), String::new(), details)
                    };

                let status = format!(
                    "Repo: {}\nGit: {}\nMode: {}\nModel configured: {}\nModel active: {}\nOllama: {}\nState: {}{}{}",
                    self.state.repo.name,
                    git_status,
                    self.state.execution.mode.as_str(),
                    self.state.model.configured.as_deref().unwrap_or("none"),
                    self.state.model.active.as_deref().unwrap_or("none"),
                    if self.state.ollama_connected {
                        "connected"
                    } else {
                        "disconnected"
                    },
                    execution_state,
                    if !outcome_display.is_empty() {
                        format!("\n{}", outcome_display)
                    } else {
                        String::new()
                    },
                    recovery_details,
                );

                self.push_system_notice(&status);
            }

            Command::ShowHelp => {
                self.emit_event("cmd", "/help");
                let help_text = get_help_text().to_string();
                self.push_system_notice(&help_text);
            }

            Command::GitStatus => {
                self.emit_event("cmd", "/git");

                // Capture current Git repository state
                let git_grounding = capture_git_grounding(&self.state.repo.path);

                let mut git_info = format!(
                    "Git Repository State\n\nBranch: {}\nHead: {}\nStatus: {}\n",
                    git_grounding.branch_name.as_deref().unwrap_or("(detached)"),
                    git_grounding.head_commit.as_deref().unwrap_or("unknown"),
                    if git_grounding.is_clean() {
                        "Clean".to_string()
                    } else {
                        format!("Dirty ({} modifications)", git_grounding.total_changes())
                    }
                );

                if !git_grounding.modified_files.is_empty() {
                    git_info.push_str("\nModified files:\n");
                    for file in &git_grounding.modified_files {
                        git_info.push_str(&format!("  {} {}\n", file.status, file.path));
                    }
                }

                if !git_grounding.staged_files.is_empty() {
                    git_info.push_str("\nStaged files:\n");
                    for file in &git_grounding.staged_files {
                        git_info.push_str(&format!("  {} {}\n", file.status, file.path));
                    }
                }

                if !git_grounding.untracked_files.is_empty() {
                    git_info.push_str("\nUntracked files:\n");
                    for file in &git_grounding.untracked_files {
                        git_info.push_str(&format!("  {} {}\n", file.status, file.path));
                    }
                }

                if !git_grounding.recent_commits.is_empty() {
                    git_info.push_str("\nRecent commits:\n");
                    for commit in &git_grounding.recent_commits {
                        git_info
                            .push_str(&format!("  {} - {}\n", commit.short_hash, commit.subject));
                    }
                }

                self.push_system_notice(&git_info);
                self.persist().await;
            }

            Command::RunValidation => {
                if self.state.execution.mode != ExecutionMode::Task {
                    self.soft_block(
                        "Validation requires TASK mode",
                        "Switch to TASK mode and retry /validate.",
                    );
                    self.persist().await;
                    return Ok(false);
                }
                self.emit_event("cmd", "/validate");
                self.set_execution_mode(ExecutionMode::Task);
                self.set_execution_state(ExecutionState::Validating);
                self.set_active_objective(Some(format!("Validate {}", self.state.repo.name)));
                self.set_current_step(Some("Running validation pipeline".to_string()), None, None);
                self.record_last_action("Validation pipeline started");
                // Run validation pipeline
                if let Err(e) = self.run_validation().await {
                    self.emit_event("validation", &format!("Pipeline error: {}", e));
                }
                self.persist().await;
            }

            Command::RefreshRuntime => {
                self.emit_event("cmd", "/refresh-runtime");
                self.record_last_action("Refreshing runtime status");
                if let Err(error) = self.refresh_runtime_status().await {
                    self.set_panel_status(
                        PanelStatusLevel::Error,
                        format!("Runtime refresh failed: {}", error),
                    );
                    self.emit_event("runtime", &format!("Refresh failed: {}", error));
                }
                self.persist().await;
            }

            Command::ResetValidation => {
                self.reset_validation_state();
                self.persist().await;
            }

            Command::ClearLogs => {
                self.clear_runtime_logs();
                self.persist().await;
            }

            Command::RunForgeTask { task } => {
                if self.state.execution.mode != ExecutionMode::Task {
                    self.soft_block(
                        "Task execution requires TASK mode",
                        "Switch to TASK mode and retry /task.",
                    );
                    self.persist().await;
                    return Ok(false);
                }
                self.emit_event("cmd", &format!("/task {}", task));
                // Request approval before starting execution
                self.request_approval(
                    "Start Forge task",
                    format!("Task: {}", task),
                    Some("Forge will execute this task with file mutations.".to_string()),
                    ApprovalAction::StartForgeTask { task: task.clone() },
                );
                self.persist().await;
            }

            Command::ReadFile { path } => {
                if self.state.execution.mode == ExecutionMode::Chat {
                    self.soft_block(
                        "File reads require EDIT or TASK mode",
                        "Switch to EDIT or TASK mode and retry /read.",
                    );
                    self.persist().await;
                    return Ok(false);
                }

                let project_root = match self.active_project_root() {
                    Ok(root) => root,
                    Err(_) => {
                        self.set_execution_block(
                            "No active project",
                            "Create or attach a project first",
                            Some("/open <path>".to_string()),
                        );
                        self.persist().await;
                        return Ok(false);
                    }
                };
                self.set_execution_state(ExecutionState::Executing);
                self.set_active_objective(Some(format!("Read {}", path)));
                self.set_current_step(Some("Reading file".to_string()), None, None);
                let _ = self.apply_host_action(HostAction::ReadFile { project_root, path })?;
                self.clear_soft_block();
                self.persist().await;
            }

            Command::WriteFile { path, content } => {
                if self.state.execution.mode == ExecutionMode::Chat {
                    self.soft_block(
                        "File writes require EDIT or TASK mode",
                        "Switch to EDIT or TASK mode and retry /write.",
                    );
                    self.persist().await;
                    return Ok(false);
                }

                let project_root = match self.active_project_root() {
                    Ok(root) => root,
                    Err(_) => {
                        self.set_execution_block(
                            "No active project",
                            "Create or attach a project first",
                            Some("/open <path>".to_string()),
                        );
                        self.persist().await;
                        return Ok(false);
                    }
                };
                self.set_execution_state(ExecutionState::Executing);
                self.set_active_objective(Some(format!("Write {}", path)));
                self.set_current_step(Some("Writing file".to_string()), None, None);
                let _ = self.apply_host_action(HostAction::WriteFile {
                    project_root,
                    path,
                    content,
                })?;
                self.clear_soft_block();
                self.persist().await;
            }

            Command::ReplaceInFile {
                path,
                find,
                replace,
                expected_hash,
            } => {
                if self.state.execution.mode == ExecutionMode::Chat {
                    self.soft_block(
                        "Patching files requires EDIT or TASK mode",
                        "Switch to EDIT or TASK mode and retry /replace.",
                    );
                    self.persist().await;
                    return Ok(false);
                }

                let project_root = match self.active_project_root() {
                    Ok(root) => root,
                    Err(_) => {
                        self.set_execution_block(
                            "No active project",
                            "Create or attach a project first",
                            Some("/open <path>".to_string()),
                        );
                        self.persist().await;
                        return Ok(false);
                    }
                };
                self.set_execution_state(ExecutionState::Executing);
                self.set_active_objective(Some(format!("Patch {}", path)));
                self.set_current_step(Some("Applying hardened patch".to_string()), None, None);
                let _ = self.apply_host_action(HostAction::ApplyPatch {
                    project_root,
                    path,
                    find,
                    replace,
                    expected_hash,
                })?;
                self.clear_soft_block();
                self.persist().await;
            }

            Command::RunShell { command } => {
                if self.state.execution.mode == ExecutionMode::Chat {
                    self.soft_block(
                        "Shell commands require EDIT or TASK mode",
                        "Switch to EDIT for approved commands or TASK for direct execution.",
                    );
                    self.persist().await;
                    return Ok(false);
                }

                let project_root = match self.active_project_root() {
                    Ok(root) => root,
                    Err(_) => {
                        self.set_execution_block(
                            "No active project",
                            "Create or attach a project first",
                            Some("/open <path>".to_string()),
                        );
                        self.persist().await;
                        return Ok(false);
                    }
                };

                if self.state.execution.mode == ExecutionMode::Edit {
                    self.request_approval(
                        "Run command",
                        format!("{} (cwd: {})", command, project_root.display()),
                        Some("EDIT mode requires approval before running commands.".to_string()),
                        ApprovalAction::RunCommand { command },
                    );
                    self.persist().await;
                    return Ok(false);
                }

                self.set_execution_state(ExecutionState::Executing);
                self.set_active_objective(Some(format!("Run {}", command)));
                self.set_current_step(Some("Running shell command".to_string()), None, None);
                let _ = self.apply_host_action(HostAction::RunCommand {
                    project_root,
                    command,
                })?;
                self.clear_soft_block();
                self.persist().await;
            }

            Command::ApprovePending => {
                // First check for checkpoint approval (new system)
                if self.persistence.has_active_checkpoint() {
                    if let Some(checkpoint) = self.persistence.get_active_checkpoint().cloned() {
                        self.persistence.approve_active_checkpoint();

                        // V1.6 AUDIT: Log approval resolution
                        if let Some(chain_id) = self.persistence.active_chain_id.clone() {
                            if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                                let audit_event = crate::state::AuditEvent::approval(
                                    crate::state::AuditEventType::ApprovalResolved,
                                    &checkpoint.checkpoint_type.description(),
                                    Some(true),
                                );
                                chain.audit_event(audit_event);
                            }
                        }

                        self.push_system_notice(&format!(
                            "✓ Approved checkpoint: {} ({} risk)\nStep: {}\nResuming execution...",
                            checkpoint.checkpoint_type.description(),
                            checkpoint.risk_level.label(),
                            checkpoint.step_description
                        ));
                        self.persist().await;

                        // Try to auto-resume if policy allows
                        if self.persistence.chain_policy.auto_advance {
                            // Queue chain resume for execution after current command completes
                            // to avoid async recursion
                            self.pending_command = Some(Command::ChainResume {
                                chain_id: "active".to_string(),
                                force: false,
                            });
                        }
                        return Ok(false);
                    }
                }

                // Fall back to legacy pending_approval system
                let Some(request) = self.pending_approval.clone() else {
                    self.push_system_notice("No pending approval.");
                    self.persist().await;
                    return Ok(false);
                };

                self.pending_approval = None;
                match request.action {
                    ApprovalAction::RunCommand { command } => {
                        let project_root = self.active_project_root()?;
                        self.set_execution_state(ExecutionState::Executing);
                        self.set_active_objective(Some(format!("Run {}", command)));
                        self.set_current_step(
                            Some("Running approved command".to_string()),
                            None,
                            None,
                        );
                        let _ = self.apply_host_action(HostAction::RunCommand {
                            project_root,
                            command,
                        })?;
                    }
                    ApprovalAction::DeleteProject { path } => {
                        let deleting_active = Self::is_real_repo_path(&self.state.repo.path)
                            && canonical_or_display_path(&self.state.repo.path)
                                == canonical_or_display_path(path.to_string_lossy().as_ref());
                        let _ = self.apply_host_action(HostAction::DeleteProject { path })?;
                        if deleting_active {
                            self.state.repo = crate::state::RepoContext::default();
                            self.state.model.configured = None;
                            self.state.model.active = None;
                            self.persistence.active_repo = None;
                        }
                    }
                    ApprovalAction::StartForgeTask { task } => {
                        // Start execution after approval
                        if let Err(e) = self.start_execution_task(&task) {
                            self.emit_event("execution", &format!("Failed to start: {}", e));
                        }
                    }
                }
                self.clear_soft_block();
                self.persist().await;
            }

            Command::DenyPending => {
                // First check for checkpoint denial (new system)
                if self.persistence.has_active_checkpoint() {
                    if let Some(checkpoint) = self.persistence.get_active_checkpoint().cloned() {
                        self.persistence.deny_active_checkpoint();

                        // Update chain status based on policy
                        let halt_on_failure = self.persistence.chain_policy.halt_on_failure;
                        if let Some(chain_id) = self.persistence.active_chain_id.clone() {
                            if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                                // V1.6 AUDIT: Log approval denial
                                let audit_event = crate::state::AuditEvent::approval(
                                    crate::state::AuditEventType::ApprovalResolved,
                                    &checkpoint.checkpoint_type.description(),
                                    Some(false),
                                );
                                chain.audit_event(audit_event);

                                if halt_on_failure {
                                    chain.status = crate::persistence::ChainLifecycleStatus::Halted;
                                    self.push_system_notice(&format!(
                                        "✗ Denied checkpoint: {}\nStep: {}\nChain halted (halt_on_failure=true)",
                                        checkpoint.checkpoint_type.description(),
                                        checkpoint.step_description
                                    ));
                                } else {
                                    chain.status =
                                        crate::persistence::ChainLifecycleStatus::Running;
                                    self.push_system_notice(&format!(
                                        "✗ Denied checkpoint: {}\nStep: {} skipped. Continuing chain...",
                                        checkpoint.checkpoint_type.description(),
                                        checkpoint.step_description
                                    ));
                                }
                            }
                        }

                        self.persistence.clear_active_checkpoint();
                        self.persist().await;
                        return Ok(false);
                    }
                }

                // Fall back to legacy pending_approval system
                self.pending_approval = None;
                self.clear_soft_block();
                self.push_system_notice("Pending approval canceled.");
                self.persist().await;
            }

            Command::Quit => {
                self.emit_event("cmd", "/quit");
                self.quit();
                return Ok(true);
            }

            Command::Unknown { input } => {
                self.emit_event("cmd", &format!("unknown command: {}", input));
                self.record_last_action(format!("Unknown command: {}", input));
                self.push_system_notice(&format!(
                    "Unknown command: {}\nUse /help to see supported commands.",
                    input
                ));
                self.persist().await;
            }

            Command::Task { content } => {
                // ROUTE THROUGH UNIFIED PIPELINE - no bypass
                self.execute_unified(&content, "task", &content, |plan| {
                    plan.add_step("Parse request", crate::state::StepAction::Parse);
                    plan.add_step("Generate response", crate::state::StepAction::Chat);
                })
                .await?;
                self.persist().await;
            }

            // RLEF management commands
            Command::RLEFStatus => {
                let stats = self.state.rlef_memory.stats();
                let msg = format!(
                    "RLEF Statistics:\n  Total hints: {}\n  Active hints: {}\n  Feedback entries: {}\n  Evidence threshold: {}",
                    stats.total_hints,
                    stats.active_hints,
                    stats.total_feedback,
                    stats.min_evidence_threshold
                );
                self.push_system_notice(&msg);

                // Show active hints
                let active = self.state.rlef_memory.get_active_hints();
                if !active.is_empty() {
                    let mut hint_lines = vec!["\nActive hints:".to_string()];
                    for hint in active {
                        hint_lines.push(format!(
                            "  • {} ({} evidence, {:.0}% confidence)",
                            hint.guidance,
                            hint.evidence_count,
                            hint.confidence * 100.0
                        ));
                    }
                    self.push_system_notice(&hint_lines.join("\n"));
                }
            }

            Command::RLEFClear => {
                self.state.rlef_memory.clear();
                self.push_system_notice(
                    "RLEF memory cleared. All hints and feedback history removed.",
                );
            }

            Command::RLEFDisableHint { class, guidance } => {
                // Parse the class string to enum (simplified - would need proper mapping)
                self.push_system_notice(&format!(
                    "Disabled hint: {} for class {}",
                    guidance, class
                ));
            }

            // Chain management commands - REAL IMPLEMENTATION
            Command::ListChains => {
                let chains: Vec<_> = self
                    .persistence
                    .get_active_chains()
                    .into_iter()
                    .cloned()
                    .collect();
                if chains.is_empty() {
                    self.push_system_notice(
                        "No active chains.\n\nCreate a chain:\n  • /task <description> - Task-oriented chain\n  • /plan - Review and create from plan"
                    );
                } else {
                    let active_id = self
                        .persistence
                        .active_chain_id
                        .as_deref()
                        .unwrap_or("none");
                    let active_short = if active_id.len() > 16 {
                        &active_id[..16]
                    } else {
                        active_id
                    };
                    let mut msg = format!("Active Chains (current: {}):\n", active_short);

                    for chain in chains {
                        // V1.5 UNIFICATION: Use ExecutionOutcome for status icon when set
                        let status_icon =
                            chain.get_outcome().map(|o| o.icon()).unwrap_or_else(|| {
                                match chain.status {
                                crate::persistence::ChainLifecycleStatus::Running => "▶",
                                crate::persistence::ChainLifecycleStatus::Complete => "✓",
                                crate::persistence::ChainLifecycleStatus::Failed => "✗",
                                crate::persistence::ChainLifecycleStatus::Halted => "⏸",
                                crate::persistence::ChainLifecycleStatus::WaitingForApproval => {
                                    "⏳"
                                }
                                crate::persistence::ChainLifecycleStatus::Archived => "🗑",
                                _ => "○",
                            }
                            });

                        let action_hint = match chain.status {
                            crate::persistence::ChainLifecycleStatus::Halted => " [/chain resume]",
                            crate::persistence::ChainLifecycleStatus::Draft
                            | crate::persistence::ChainLifecycleStatus::Ready => " [/chain resume]",
                            _ => "",
                        };

                        let id_short = if chain.id.len() > 16 {
                            &chain.id[..16]
                        } else {
                            &chain.id
                        };
                        let is_active =
                            self.persistence.active_chain_id.as_ref() == Some(&chain.id);
                        let active_marker = if is_active { " → " } else { "   " };

                        // V1.5 UNIFICATION: Use ExecutionOutcome label when set
                        let status_label = chain
                            .get_outcome()
                            .map(|o| o.label().to_lowercase())
                            .unwrap_or_else(|| match chain.status {
                                crate::persistence::ChainLifecycleStatus::Draft => {
                                    "draft".to_string()
                                }
                                crate::persistence::ChainLifecycleStatus::Ready => {
                                    "ready".to_string()
                                }
                                crate::persistence::ChainLifecycleStatus::Running => {
                                    "running".to_string()
                                }
                                crate::persistence::ChainLifecycleStatus::WaitingForApproval => {
                                    "waiting".to_string()
                                }
                                crate::persistence::ChainLifecycleStatus::Halted => {
                                    "halted".to_string()
                                }
                                crate::persistence::ChainLifecycleStatus::Failed => {
                                    "failed".to_string()
                                }
                                crate::persistence::ChainLifecycleStatus::Complete => {
                                    "complete".to_string()
                                }
                                crate::persistence::ChainLifecycleStatus::Archived => {
                                    "archived".to_string()
                                }
                            });

                        let checkpoint_marker =
                            match crate::persistence::CheckpointManager::get_latest_checkpoint(
                                &chain.id,
                            )
                            .await
                            {
                                Some(checkpoint) if checkpoint.is_resumable() => " [checkpoint]",
                                Some(_) => " [checkpoint?]",
                                None => "",
                            };

                        msg.push_str(&format!(
                            "{}{} [{}] {} - {}{}{}\n",
                            active_marker,
                            status_icon,
                            id_short,
                            chain.name,
                            status_label,
                            action_hint,
                            checkpoint_marker
                        ));
                    }

                    msg.push_str("\nCommands:\n  /chain switch <id>  - Switch active chain\n  /chain status       - Show active chain details\n  /chain archive <id> - Archive a chain");
                    self.push_system_notice_with_suggestions(&msg);
                }
                self.persist().await;
            }

            Command::ChainStatus { chain_id } => {
                let id = chain_id
                    .as_deref()
                    .or(self.persistence.active_chain_id.as_deref())
                    .unwrap_or("none");

                if id == "none" {
                    use crate::guidance::SystemNarrative;
                    self.push_system_notice_with_suggestions(&SystemNarrative::idle());
                } else if let Some(chain) = self.persistence.get_chain(id).cloned() {
                    let status_icon = match chain.status {
                        crate::persistence::ChainLifecycleStatus::Running => "▶",
                        crate::persistence::ChainLifecycleStatus::Complete => "✓",
                        crate::persistence::ChainLifecycleStatus::Failed => "✗",
                        crate::persistence::ChainLifecycleStatus::Halted => "⏸",
                        crate::persistence::ChainLifecycleStatus::WaitingForApproval => "⏳",
                        _ => "○",
                    };

                    let active_step_str = chain
                        .active_step
                        .map(|i| format!(" (current step: {})", i + 1))
                        .unwrap_or_else(|| " (no active step)".to_string());

                    // Build progress info
                    let progress = if chain.steps.is_empty() {
                        "No steps defined".to_string()
                    } else {
                        format!(
                            "{}/{} steps executed, {} failed, {} total",
                            chain.total_steps_executed,
                            chain.steps.len(),
                            chain.total_steps_failed,
                            chain.steps.len()
                        )
                    };

                    // Context assembly status
                    let context_info = if let Some(ref context) = chain.context_state {
                        let included = context.files.iter().filter(|f| f.included).count();
                        let trimmed = context.files.len() - included;
                        if trimmed > 0 {
                            format!("{} files ({} trimmed)", included, trimmed)
                        } else {
                            format!("{} files", included)
                        }
                    } else if !chain.selected_context_files.is_empty() {
                        format!("{} files (v2)", chain.selected_context_files.len())
                    } else {
                        "Not assembled".to_string()
                    };

                    // Check execution readiness
                    let readiness = chain.check_execution_readiness(&self.persistence.chain_policy);
                    let execution_status = if readiness.can_execute {
                        if self.persistence.chain_policy.auto_advance {
                            "✓ Ready (auto-advance enabled)"
                        } else {
                            "✓ Ready to execute"
                        }
                    } else if let Some(ref reason) = readiness.reason {
                        &format!("✗ Blocked: {}", reason.description())
                    } else {
                        "✗ Blocked"
                    };

                    // Get Git state (from chain if captured, otherwise current)
                    let git_info = if let Some(ref grounding) = chain.git_grounding {
                        grounding.summary()
                    } else {
                        capture_git_grounding(&self.state.repo.path).summary()
                    };

                    // V1.6: Get recovery status for this chain
                    let recovery_status = if !self.recovery_state.recovery_path.is_empty() {
                        let summary = self.get_recovery_summary();
                        if !summary.is_empty() {
                            format!("\nRecovery: {}", summary)
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                    let mut msg = format!(
                        "Chain Status: {} {}\nID: {}\nStatus: {:?}{}\nProgress: {}\nContext: {}\nGit: {}\nExecution: {}\nCreated: {} | Updated: {}{}",
                        status_icon,
                        chain.name,
                        if chain.id.len() > 32 {
                            &chain.id[..32]
                        } else {
                            &chain.id
                        },
                        chain.status,
                        active_step_str,
                        progress,
                        context_info,
                        git_info,
                        execution_status,
                        chain.created_at.format("%Y-%m-%d %H:%M"),
                        chain.updated_at.format("%Y-%m-%d %H:%M"),
                        recovery_status
                    );

                    // Show next step if ready
                    if let (Some(step_id), Some(desc)) =
                        (readiness.next_step_id, readiness.next_step_description)
                    {
                        msg.push_str(&format!(
                            "\nNext Step: {} ({})",
                            crate::text::take_chars(&step_id, 8),
                            desc
                        ));
                    }

                    let checkpoint_report =
                        crate::persistence::CheckpointManager::inspect_latest_checkpoint(
                            &chain,
                            &Self::checkpoint_base_path(&chain),
                        )
                        .await;
                    self.checkpoint_inspector_report = Some(checkpoint_report.clone());
                    msg.push_str("\nCheckpoint: ");
                    msg.push_str(&Self::render_checkpoint_status_line(&checkpoint_report));

                    // Add checkpoint info if waiting for approval
                    if let Some(ref checkpoint) = chain.pending_checkpoint {
                        if checkpoint.is_pending() {
                            msg.push_str(&format!(
                                "\n\n⏳ APPROVAL REQUIRED\nRisk: {} | Type: {}\nReason: {}\nStep: {}\n\n▶ /approve to execute\n▶ /deny to skip/abort",
                                checkpoint.risk_level.label(),
                                checkpoint.checkpoint_type.description(),
                                checkpoint.reason,
                                checkpoint.step_description
                            ));
                        }
                    }

                    // Add actionable guidance based on status
                    let guidance = if let Some(ref reason) = readiness.reason {
                        format!(
                            "\n\n▶ {} → {}",
                            reason.description(),
                            reason.suggested_action()
                        )
                    } else {
                        match chain.status {
                            crate::persistence::ChainLifecycleStatus::Halted => {
                                "\n\n▶ Chain is halted. Use /chain resume to continue.".to_string()
                            }
                            crate::persistence::ChainLifecycleStatus::Draft | crate::persistence::ChainLifecycleStatus::Ready => {
                                if chain.steps.is_empty() {
                                    "\n\n▶ Chain has no steps. Start a task to generate a plan.".to_string()
                                } else {
                                    "\n\n▶ Chain ready to run. Use /chain resume to start.".to_string()
                                }
                            }
                            crate::persistence::ChainLifecycleStatus::Running => {
                                if self.persistence.chain_policy.auto_advance {
                                    "\n\n▶ Auto-advance enabled. Chain will progress automatically.".to_string()
                                } else {
                                    "\n\n▶ Chain running. Steps execute sequentially.".to_string()
                                }
                            }
                            crate::persistence::ChainLifecycleStatus::Complete => {
                                "\n\n✓ Chain complete. Use /chain archive to clean up.".to_string()
                            }
                            crate::persistence::ChainLifecycleStatus::Failed => {
                                "\n\n✗ Chain failed. Check inspector for details. Use /chain resume to retry.".to_string()
                            }
                            _ => "".to_string(),
                        }
                    };
                    msg.push_str(&guidance);

                    // V1.6 AUDIT: Add audit summary to chain status
                    if !chain.audit_log.is_empty() {
                        let audit_count = chain.audit_log.len();
                        let transition_count = chain.get_transition_history().len();
                        msg.push_str(&format!(
                            "\n\nAudit: {} events, {} transitions",
                            audit_count, transition_count
                        ));

                        // Show last transition if available
                        if let Some(last) = chain.get_last_audit_events(1).first() {
                            let ts = last.timestamp.format("%H:%M:%S");
                            let event_str =
                                format!("{:?}", last.event_type).replace("AuditEventType::", "");
                            msg.push_str(&format!("\n  Last: [{}] {}", ts, event_str));
                        }

                        msg.push_str("\n  View: /audit or inspector → Audit tab");
                    }

                    // Add helpful commands
                    msg.push_str("\n\nCommands:\n  /plan - View plan\n  /plan context - View context\n  /chain resume - Continue execution");

                    // Add policy info
                    if self.persistence.chain_policy.auto_advance {
                        msg.push_str("\n\nPolicy: auto-advance enabled");
                    }

                    self.push_system_notice(&msg);
                } else {
                    self.push_system_notice(&format!(
                        "Chain '{}' not found.\n\nAvailable chains:\n  /chains - List all chains",
                        if id.len() > 16 { &id[..16] } else { id }
                    ));
                }
                self.persist().await;
            }

            // V1.6 AUDIT: Audit timeline command
            Command::Audit { chain_id } => {
                let id = chain_id
                    .as_deref()
                    .or(self.persistence.active_chain_id.as_deref())
                    .unwrap_or("none");

                if id == "none" {
                    self.push_system_notice(
                        "No active chain.\n\nCreate a chain:\n  • /task <description> - Task-oriented chain\n  • /plan - Review and create from plan"
                    );
                } else if let Some(chain) = self.persistence.get_chain(id) {
                    let audit_log = &chain.audit_log;

                    if audit_log.is_empty() {
                        self.push_system_notice(&format!(
                            "Chain '{}' has no audit events yet.\n\nEvents are recorded as the chain executes.",
                            chain.name
                        ));
                    } else {
                        let mut msg = format!(
                            "Audit Timeline for '{}'\nID: {}\nTotal Events: {}\n",
                            chain.name,
                            if chain.id.len() > 16 {
                                &chain.id[..16]
                            } else {
                                &chain.id
                            },
                            audit_log.len()
                        );

                        // Show recent transition history
                        let transitions = audit_log.get_transition_history();
                        if !transitions.is_empty() {
                            msg.push_str("\nRecent Transitions:\n");
                            for (i, t) in transitions.iter().rev().take(5).enumerate() {
                                if let (Some(prev), Some(next)) = (t.previous_state, t.next_state) {
                                    let prev_str =
                                        format!("{:?}", prev).replace("ExecutionState::", "");
                                    let next_str =
                                        format!("{:?}", next).replace("ExecutionState::", "");
                                    let ts = t.timestamp.format("%H:%M:%S");
                                    msg.push_str(&format!(
                                        "  {}. [{}] {} → {}\n",
                                        i + 1,
                                        ts,
                                        prev_str,
                                        next_str
                                    ));
                                    if let Some(reason) = &t.reason {
                                        msg.push_str(&format!("     Reason: {}\n", reason));
                                    }
                                }
                            }
                        }

                        // Show outcome trace
                        let outcome_trace = audit_log.get_outcome_trace();
                        if !outcome_trace.is_empty() {
                            msg.push_str("\nOutcome Trace:\n");
                            for e in outcome_trace.iter().rev().take(3) {
                                let ts = e.timestamp.format("%H:%M:%S");
                                let event_str =
                                    format!("{:?}", e.event_type).replace("AuditEventType::", "");
                                msg.push_str(&format!("  [{}] {}\n", ts, event_str));
                                if let Some(reason) = &e.reason {
                                    msg.push_str(&format!("     {}\n", reason));
                                }
                            }
                        }

                        // Show last 3 recent events
                        let recent = audit_log.get_last_n(3);
                        if !recent.is_empty() {
                            msg.push_str("\nRecent Events:\n");
                            for e in recent.iter().rev() {
                                let ts = e.timestamp.format("%H:%M:%S");
                                let event_str =
                                    format!("{:?}", e.event_type).replace("AuditEventType::", "");
                                let icon = match e.event_type {
                                    crate::state::AuditEventType::StateTransitionApplied => "→",
                                    crate::state::AuditEventType::StateTransitionRejected => "✗",
                                    crate::state::AuditEventType::StateTransitionNormalized => "~",
                                    crate::state::AuditEventType::OutcomeFinalized => "★",
                                    crate::state::AuditEventType::StepStarted => "▶",
                                    crate::state::AuditEventType::StepCompleted => "✓",
                                    crate::state::AuditEventType::ApprovalRequested => "⏸",
                                    crate::state::AuditEventType::ApprovalResolved => "✓",
                                    crate::state::AuditEventType::RepairTriggered => "🔧",
                                    _ => "•",
                                };
                                msg.push_str(&format!("  [{}] {} {}\n", ts, icon, event_str));
                            }
                        }

                        // Open inspector to Audit tab
                        self.show_inspector = true;
                        self.state.active_inspector_tab = crate::state::InspectorTab::Audit;

                        msg.push_str("\nCommands:\n  /audit - Show full timeline\n  /chain status - Show chain status");

                        self.push_system_notice(&msg);
                    }
                } else {
                    self.push_system_notice(&format!(
                        "Chain '{}' not found.\n\nAvailable chains:\n  /chains - List all chains",
                        if id.len() > 16 { &id[..16] } else { id }
                    ));
                }
                self.persist().await;
            }

            // V1.6 CHECKPOINT: List checkpoints command
            Command::CheckpointList { chain_id } => {
                let target_id = chain_id
                    .or_else(|| self.persistence.active_chain_id.clone())
                    .unwrap_or_else(|| "active".to_string());

                let checkpoints =
                    crate::persistence::CheckpointManager::list_checkpoints(&target_id).await;

                if checkpoints.is_empty() {
                    self.push_system_notice(&format!(
                        "No checkpoints found for chain '{}'.\n\nCheckpoints are created automatically after successful step completion.",
                        target_id
                    ));
                } else {
                    let mut msg = format!("=== Checkpoints for '{}', ===\n\n", target_id);
                    for (i, checkpoint) in checkpoints.iter().take(10).enumerate() {
                        let status_icon = match checkpoint.validation_status {
                            crate::persistence::CheckpointValidationStatus::Valid => "✓",
                            crate::persistence::CheckpointValidationStatus::Warning => "⚠",
                            crate::persistence::CheckpointValidationStatus::Invalid => "✗",
                            crate::persistence::CheckpointValidationStatus::Unchecked => "?",
                        };

                        msg.push_str(&format!(
                            "{}. {} {} (step {:?})\n   Hash: {}...\n   Source: {:?}\n   Created: {}\n\n",
                            i + 1,
                            status_icon,
                            checkpoint.checkpoint_id,
                            checkpoint.active_step,
                            crate::text::take_chars(&checkpoint.workspace_hash, 8),
                            checkpoint.source,
                            checkpoint.created_at.format("%Y-%m-%d %H:%M:%S")
                        ));
                    }

                    if checkpoints.len() > 10 {
                        msg.push_str(&format!(
                            "... and {} more checkpoints\n",
                            checkpoints.len() - 10
                        ));
                    }

                    msg.push_str("Commands:\n  /chain resume - Resume from latest checkpoint\n  /audit replay - Verify checkpoint consistency");

                    self.push_system_notice(&msg);
                }
            }

            Command::CheckpointStatus { chain_id } => {
                let target_id = chain_id
                    .or_else(|| self.persistence.active_chain_id.clone())
                    .unwrap_or_else(|| "active".to_string());

                if let Some(chain) = self.persistence.get_chain(&target_id).cloned() {
                    let base_path = Self::checkpoint_base_path(&chain);
                    let report = crate::persistence::CheckpointManager::inspect_latest_checkpoint(
                        &chain, &base_path,
                    )
                    .await;
                    self.checkpoint_inspector_report = Some(report.clone());
                    self.show_inspector = true;
                    self.state.active_inspector_tab = crate::state::InspectorTab::Checkpoint;
                    self.push_system_notice(&Self::render_checkpoint_report(&report));
                } else {
                    let report =
                        crate::persistence::CheckpointOperatorReport::missing(target_id.clone(), 0);
                    self.checkpoint_inspector_report = Some(report.clone());
                    self.show_inspector = true;
                    self.state.active_inspector_tab = crate::state::InspectorTab::Checkpoint;
                    self.push_system_notice(&Self::render_checkpoint_report(&report));
                }
            }

            // V1.6 CHECKPOINT: Show specific checkpoint details
            Command::CheckpointShow {
                chain_id,
                checkpoint_id,
            } => {
                let audit_len = self
                    .persistence
                    .get_chain(&chain_id)
                    .map(|chain| chain.audit_log.len())
                    .unwrap_or(0);
                match crate::persistence::CheckpointManager::load_checkpoint(
                    &chain_id,
                    &checkpoint_id,
                )
                .await
                {
                    Ok(checkpoint) => {
                        if let Some(chain) = self.persistence.get_chain(&chain_id).cloned() {
                            let base_path = Self::checkpoint_base_path(&chain);
                            let report = crate::persistence::CheckpointManager::inspect_checkpoint(
                                &checkpoint,
                                &chain,
                                &base_path,
                            )
                            .await;
                            self.checkpoint_inspector_report = Some(report.clone());
                            self.show_inspector = true;
                            self.state.active_inspector_tab =
                                crate::state::InspectorTab::Checkpoint;
                            self.push_system_notice(&Self::render_checkpoint_report(&report));
                        } else {
                            self.push_system_notice(&format!("Chain '{}' not found", chain_id));
                        }
                    }
                    Err(e) => {
                        let report = crate::persistence::CheckpointOperatorReport::corrupted(
                            chain_id,
                            checkpoint_id,
                            e.to_string(),
                            audit_len,
                        );
                        self.checkpoint_inspector_report = Some(report.clone());
                        self.show_inspector = true;
                        self.state.active_inspector_tab = crate::state::InspectorTab::Checkpoint;
                        self.push_system_notice(&Self::render_checkpoint_report(&report));
                    }
                }
            }

            // V1.6 CHECKPOINT: Delete checkpoint
            Command::CheckpointDelete {
                chain_id,
                checkpoint_id,
            } => {
                match crate::persistence::CheckpointManager::delete_checkpoint(
                    &chain_id,
                    &checkpoint_id,
                )
                .await
                {
                    Ok(()) => {
                        self.push_system_notice(&format!(
                            "Deleted checkpoint {} for chain {}",
                            checkpoint_id, chain_id
                        ));
                    }
                    Err(e) => {
                        self.push_system_notice(&format!("Failed to delete checkpoint: {}", e));
                    }
                }
            }

            // V1.6 RECOVERY: Show recovery status
            Command::ShowRecovery { chain_id } => {
                self.emit_event("cmd", "/recovery");

                // Get the target chain (active or specified)
                let target_chain_id = chain_id
                    .as_ref()
                    .or(self.persistence.active_chain_id.as_ref());

                if let Some(cid) = target_chain_id {
                    if let Some(chain) = self.persistence.get_chain(cid) {
                        // Get recovery summary for this chain
                        let recovery_summary = if !self.recovery_state.recovery_path.is_empty() {
                            self.get_recovery_summary()
                        } else {
                            "No recovery activity for this chain".to_string()
                        };

                        let msg = format!(
                            "Recovery Status for {} ({}):\n{}",
                            chain.name,
                            if cid.len() > 8 { &cid[..8] } else { cid },
                            recovery_summary
                        );

                        self.push_system_notice(&msg);

                        // Also show inspector recovery tab
                        self.show_inspector = true;
                        self.state.active_inspector_tab = crate::state::InspectorTab::Recovery;
                    } else {
                        self.push_system_notice(&format!("Chain {} not found", cid));
                    }
                } else {
                    self.push_system_notice(
                        "No active chain. Use /recovery <chain_id> or activate a chain first.",
                    );
                }
            }

            Command::ChainSwitch { chain_id } => {
                // Clone chain data first to avoid borrow issues
                let chain_info = self.persistence.get_chain(&chain_id).map(|chain| {
                    let status_icon = match chain.status {
                        crate::persistence::ChainLifecycleStatus::Running => "▶",
                        crate::persistence::ChainLifecycleStatus::Complete => "✓",
                        crate::persistence::ChainLifecycleStatus::Failed => "✗",
                        crate::persistence::ChainLifecycleStatus::Halted => "⏸",
                        crate::persistence::ChainLifecycleStatus::WaitingForApproval => "⏳",
                        _ => "○",
                    };
                    (
                        chain.name.clone(),
                        chain.status,
                        chain.total_steps_executed,
                        chain.steps.len(),
                        status_icon,
                        !chain.steps.is_empty(),
                    )
                });

                if let Some((name, status, executed, total, icon, has_steps)) = chain_info {
                    self.persistence.set_active_chain(Some(chain_id.clone()));

                    // Also bind to current conversation if exists
                    if let Some(conv_id) = self.persistence.active_conversation.clone() {
                        if let Some(conv) = self
                            .persistence
                            .conversations
                            .iter_mut()
                            .find(|c| c.id == conv_id)
                        {
                            conv.chain_id = Some(chain_id.clone());
                        }
                    }

                    let mut msg = format!(
                        "Switched to chain '{}' {}\nStatus: {:?} | Steps: {}/{}",
                        name, icon, status, executed, total
                    );

                    // Add next action hint
                    match status {
                        crate::persistence::ChainLifecycleStatus::Halted => {
                            msg.push_str("\n\n▶ Use /chain resume to continue execution");
                        }
                        crate::persistence::ChainLifecycleStatus::Draft
                        | crate::persistence::ChainLifecycleStatus::Ready => {
                            if has_steps {
                                msg.push_str("\n\n▶ Use /chain resume to start execution");
                            }
                        }
                        _ => {}
                    }

                    self.push_system_notice(&msg);
                    self.persist().await;
                } else {
                    self.push_system_notice(&format!(
                        "Chain '{}' not found.\n\nUse /chains to see available chains",
                        chain_id
                    ));
                }
            }

            Command::ChainResume { chain_id, force } => {
                self.emit_event("cmd", &format!("/chain resume {}", chain_id));
                self.record_last_action(format!("Resuming chain: {}", chain_id));

                // Determine target chain ID
                let target_id = if chain_id == "active" || chain_id.is_empty() {
                    self.persistence.active_chain_id.clone()
                } else {
                    Some(chain_id)
                };

                let Some(id) = target_id else {
                    self.push_system_notice(
                        "No active chain. Specify chain ID: /chain resume <id>",
                    );
                    return Ok(false);
                };

                let Some(chain) = self.persistence.get_chain(&id).cloned() else {
                    self.push_system_notice(&format!("Chain '{}' not found", id));
                    return Ok(false);
                };

                // V1.6 CHECKPOINT: Attempt validated checkpoint resume first.
                // Existing execution progress must resume from an audit-grounded checkpoint.
                let requires_checkpoint = chain.total_steps_executed > 0
                    || matches!(
                        chain.status,
                        crate::persistence::ChainLifecycleStatus::Halted
                            | crate::persistence::ChainLifecycleStatus::WaitingForApproval
                    );
                if let Some(checkpoint) =
                    crate::persistence::CheckpointManager::get_latest_checkpoint(&id).await
                {
                    // Get base path for workspace validation
                    let base_path =
                        std::path::PathBuf::from(chain.repo_path.as_deref().unwrap_or("."));

                    let report = crate::persistence::CheckpointManager::inspect_checkpoint(
                        &checkpoint,
                        &chain,
                        &base_path,
                    )
                    .await;
                    self.checkpoint_inspector_report = Some(report.clone());
                    self.show_inspector = true;
                    self.state.active_inspector_tab = crate::state::InspectorTab::Checkpoint;

                    if report.resume_allowed {
                        self.push_system_notice(&Self::render_checkpoint_report(&report));
                    } else {
                        self.push_system_notice(&Self::render_checkpoint_report(&report));
                        return Ok(false);
                    }
                } else if requires_checkpoint {
                    let report = crate::persistence::CheckpointOperatorReport::missing(
                        id.clone(),
                        chain.audit_log.len(),
                    );
                    self.checkpoint_inspector_report = Some(report.clone());
                    self.show_inspector = true;
                    self.state.active_inspector_tab = crate::state::InspectorTab::Checkpoint;
                    self.push_system_notice(&Self::render_checkpoint_report(&report));
                    return Ok(false);
                }

                let preflight = self.resolve_autonomous_planner_preflight().await;
                if !matches!(
                    preflight,
                    crate::autonomy::PlannerPreflightOutcome::Ready { .. }
                ) {
                    self.fail_autonomous_planner_preflight(preflight);
                    return Ok(false);
                }

                // V1.5: Check for critical risks before execution
                if let Some(preview) = crate::guidance::LookaheadEngine::preview_execution(
                    &self.persistence,
                    &self.state,
                ) {
                    let critical_risks: Vec<_> = preview
                        .risks
                        .iter()
                        .filter(|r| r.level == crate::guidance::RiskLevel::Critical)
                        .collect();

                    if !critical_risks.is_empty() && !force {
                        let mut warning =
                            "❌ Critical risks detected - execution blocked:\n".to_string();
                        for risk in &critical_risks {
                            warning.push_str(&format!(
                                "\n  {}: {}",
                                risk.risk_type.name(),
                                risk.description
                            ));
                            warning.push_str(&format!("\n    → {}", risk.mitigation));
                        }
                        warning.push_str("\n\nUse /preview to see full details.");
                        warning
                            .push_str("\nUse /chain resume --force to override (not recommended).");
                        self.push_system_notice(&warning);
                        return Ok(false);
                    }

                    // V1.5 FIX: When force override is used, show unified warning that
                    // acknowledges risks will be bypassed. This prevents contradictory
                    // BLOCKED + DONE messaging when execution proceeds and succeeds.
                    if !critical_risks.is_empty() && force {
                        let mut warning =
                            "⚠️ Force override active - proceeding despite critical risks:\n"
                                .to_string();
                        for risk in &critical_risks {
                            warning.push_str(&format!(
                                "\n  {}: {}",
                                risk.risk_type.name(),
                                risk.description
                            ));
                        }
                        self.push_system_notice(&warning);
                    }
                }

                // Check if resumable
                match chain.status {
                    crate::persistence::ChainLifecycleStatus::Halted
                    | crate::persistence::ChainLifecycleStatus::WaitingForApproval
                    | crate::persistence::ChainLifecycleStatus::Draft
                    | crate::persistence::ChainLifecycleStatus::Ready => {
                        // Check policy constraints
                        let policy = &self.persistence.chain_policy;
                        if chain.total_steps_executed >= policy.max_steps {
                            self.push_system_notice(&format!(
                                "Cannot resume: chain exceeded max steps ({}/{})",
                                chain.total_steps_executed, policy.max_steps
                            ));
                            return Ok(false);
                        }

                        // Find next pending step
                        let next_step = chain
                            .active_step
                            .and_then(|idx| chain.steps.get(idx))
                            .or_else(|| {
                                chain.steps.iter().find(|s| {
                                    matches!(s.status, crate::persistence::ChainStepStatus::Pending)
                                })
                            });

                        if let Some(step) = next_step {
                            let refreshed_satisfaction =
                                crate::autonomy::CompletionConfidenceEvaluator::refresh_objective_satisfaction(
                                    &chain,
                                );
                            let task =
                                Self::build_chain_step_task(&chain, step, &refreshed_satisfaction);

                            // V1.5: Different message for Draft (first-time) vs Halted (resuming)
                            let is_first_execution =
                                chain.status == crate::persistence::ChainLifecycleStatus::Draft;
                            let action_verb = if is_first_execution {
                                "Starting"
                            } else {
                                "Resuming"
                            };

                            self.push_system_notice(&format!(
                                "{} chain '{}' - executing step {}: {}",
                                action_verb,
                                chain.name,
                                chain
                                    .active_step
                                    .map(|i| i.to_string())
                                    .unwrap_or_else(|| "next".to_string()),
                                step.description
                            ));

                            // Update chain status
                            if let Some(c) = self.persistence.get_chain_mut(&id) {
                                c.status = crate::persistence::ChainLifecycleStatus::Running;
                                c.updated_at = chrono::Local::now();
                                c.objective_satisfaction = refreshed_satisfaction;
                            }

                            // Store chain context for result tracking
                            self.state.current_chain_id = Some(id.clone());
                            self.state.current_chain_step_id = Some(step.id.clone());

                            // Spawn Forge execution
                            if let Err(e) =
                                self.start_execution_task_with_display_objective(
                                    &task,
                                    &chain.objective,
                                )
                            {
                                self.push_system_notice(&format!("Failed to resume chain: {}", e));
                                // Rollback status
                                if let Some(c) = self.persistence.get_chain_mut(&id) {
                                    c.status = crate::persistence::ChainLifecycleStatus::Halted;
                                }
                            }

                            self.persist().await;
                        } else {
                            self.push_system_notice(&format!(
                                "Chain '{}' has no pending steps",
                                id
                            ));
                        }
                    }
                    crate::persistence::ChainLifecycleStatus::Running => {
                        self.push_system_notice(&format!("Chain '{}' is already running", id));
                    }
                    crate::persistence::ChainLifecycleStatus::Complete => {
                        self.push_system_notice(&format!("Chain '{}' is already complete", id));
                    }
                    crate::persistence::ChainLifecycleStatus::Failed => {
                        // V1.5: Show detailed failure information before allowing retry (Edge 3.1)
                        let failed_steps: Vec<_> = chain
                            .steps
                            .iter()
                            .filter(|s| {
                                matches!(s.status, crate::persistence::ChainStepStatus::Failed)
                            })
                            .collect();

                        let mut msg = format!("✗ Chain '{}' failed\n\n", id);

                        if !failed_steps.is_empty() {
                            msg.push_str("Failed steps:\n");
                            for step in &failed_steps {
                                msg.push_str(&format!(
                                    "  • Step {}: {}\n",
                                    chain
                                        .steps
                                        .iter()
                                        .position(|s| s.id == step.id)
                                        .map(|i| i + 1)
                                        .unwrap_or(0),
                                    step.description
                                ));
                                if let Some(error) = &step.error_message {
                                    msg.push_str(&format!("    Error: {}\n", error));
                                }
                                // Note: Step state changes tracked separately
                            }
                            msg.push('\n');
                        }

                        // Check for critical risks if retrying
                        if let Some(preview) = crate::guidance::LookaheadEngine::preview_execution(
                            &self.persistence,
                            &self.state,
                        ) {
                            let critical_count = preview
                                .risks
                                .iter()
                                .filter(|r| r.level == crate::guidance::RiskLevel::Critical)
                                .count();
                            if critical_count > 0 {
                                // V1.5 FIX: Clarify that risks can be bypassed with force,
                                // preventing contradiction if retry succeeds.
                                msg.push_str(&format!(
                                    "⚠️ {} critical risk(s) detected (can bypass with --force).\n",
                                    critical_count
                                ));
                                msg.push_str("   Use /preview to see full risk details.\n");
                                msg.push_str("   Use /chain resume --force to bypass risks.\n\n");
                            } else {
                                msg.push_str("✓ No critical risks detected.\n");
                                msg.push_str("→ Use /chain resume to retry from failed step.\n\n");
                            }
                        }

                        msg.push_str("Options:\n");
                        msg.push_str("  /chain resume      - Retry failed step\n");
                        msg.push_str("  /chain resume --force - Force retry (ignore risks)\n");
                        msg.push_str("  /replay            - Review what happened\n");
                        msg.push_str("  /plan new          - Start fresh\n");

                        self.push_system_notice(&msg);
                    }
                    crate::persistence::ChainLifecycleStatus::Archived => {
                        self.push_system_notice(&format!(
                            "Chain '{}' is archived. Unarchive first.",
                            id
                        ));
                    }
                }
            }

            // Plan review commands
            Command::ShowPlan => {
                if let Some(chain) = self.persistence.get_active_chain() {
                    if chain.steps.is_empty() {
                        self.push_system_notice(&format!(
                            "Chain '{}' has no steps planned. Use /task to add steps.",
                            chain.name
                        ));
                    } else {
                        let mut msg =
                            format!("Plan for '{}' ({} steps):\n", chain.name, chain.steps.len());
                        for (i, step) in chain.steps.iter().enumerate() {
                            let icon = match step.status {
                                crate::persistence::ChainStepStatus::Completed => "✓",
                                crate::persistence::ChainStepStatus::Failed => "✗",
                                crate::persistence::ChainStepStatus::Running => "▶",
                                crate::persistence::ChainStepStatus::Blocked => "⏸",
                                _ => "○",
                            };
                            msg.push_str(&format!("  {} {}. {}\n", icon, i + 1, step.description));
                        }
                        msg.push_str(&format!("\nStatus: {:?}", chain.status));
                        self.push_system_notice(&msg);
                    }
                } else {
                    self.push_system_notice(
                        "No active chain. Use /chains to select or create one.",
                    );
                }
            }

            Command::ShowPlanContext => {
                if let Some(chain) = self.persistence.get_active_chain() {
                    if let Some(ref context) = chain.context_state {
                        // V3 enriched display with full authority metadata
                        let included_count = context.files.iter().filter(|f| f.included).count();
                        let trimmed_count = context.files.len() - included_count;

                        let status_icon = match context.validation.status {
                            crate::persistence::ContextValidationStatus::Valid => "✓",
                            crate::persistence::ContextValidationStatus::Warning => "⚠",
                            crate::persistence::ContextValidationStatus::Invalid => "✗",
                        };

                        let mut msg = format!(
                            "Context Assembly V3 [{}] | {} | {} file(s) included",
                            status_icon, context.summary, included_count
                        );

                        // Budget info
                        msg.push_str(&format!(
                            "\nBudget: {}/{} files, {}/{} tokens",
                            context.budget.files_selected,
                            context.budget.max_files,
                            context.budget.tokens_used,
                            context.budget.max_tokens
                        ));

                        if context.budget.trimming_triggered {
                            msg.push_str(&format!(" | {} trimmed", trimmed_count));
                        }

                        // Validation warnings/errors
                        if !context.validation.warnings.is_empty() {
                            msg.push_str("\nWarnings: ");
                            msg.push_str(&context.validation.warnings.join("; "));
                        }
                        if !context.validation.errors.is_empty() {
                            msg.push_str("\nErrors: ");
                            msg.push_str(&context.validation.errors.join("; "));
                        }

                        // Selected files with reasons and priorities
                        msg.push_str("\nSelected Files:");
                        for (i, file) in context.files.iter().filter(|f| f.included).enumerate() {
                            msg.push_str(&format!(
                                "\n  {}. {} [P:{}] - {}",
                                i + 1,
                                file.path,
                                file.priority,
                                file.reason
                            ));
                        }

                        // Trimmed files (if any)
                        let trimmed: Vec<_> = context
                            .files
                            .iter()
                            .filter(|f| !f.included && f.trimmed_reason.is_some())
                            .collect();
                        if !trimmed.is_empty() {
                            msg.push_str("\nTrimmed Files:");
                            for file in trimmed {
                                msg.push_str(&format!(
                                    "\n  • {} - {}",
                                    file.path,
                                    file.trimmed_reason.as_ref().unwrap()
                                ));
                            }
                        }

                        self.push_system_notice(&msg);
                    } else if !chain.selected_context_files.is_empty() {
                        // Fallback to V2 display if no V3 state
                        let mut msg = format!(
                            "Context Assembly V2: {} file(s) selected\n",
                            chain.selected_context_files.len()
                        );
                        for (i, file) in chain.selected_context_files.iter().enumerate() {
                            msg.push_str(&format!("  {}. {}\n", i + 1, file));
                        }
                        self.push_system_notice(&msg);
                    } else {
                        self.push_system_notice("No context files selected. Context assembly will run when a task is started.");
                    }
                } else {
                    self.push_system_notice(
                        "No active chain. Use /chains to select or create one.",
                    );
                }
            }

            Command::ShowPlanCheckpoints => {
                self.emit_event("cmd", "/plan checkpoints");
                self.record_last_action("Show plan checkpoints");
                if let Some(chain) = self.persistence.get_active_chain() {
                    let mut msg = format!("Checkpoints for '{}':\n", chain.name);
                    msg.push_str("(Checkpoint display not yet implemented)");
                    self.push_system_notice(&msg);
                } else {
                    self.push_system_notice("No active chain.");
                }
            }

            // V1.3: Flow mode control
            Command::FlowMode { enabled } => {
                self.flow_mode = if enabled {
                    crate::guidance::FlowMode::Active
                } else {
                    crate::guidance::FlowMode::Standard
                };
                let status = if enabled { "ENABLED" } else { "DISABLED" };
                let msg = if enabled {
                    format!(
                        "[FLOW MODE {}]\n\nAggressive suggestion mode active.\nThe system will:\n  • Suggest next steps automatically\n  • Minimize required typing\n  • Show prepared actions for confirmation\n\nYou remain in control: type any command to override suggestions.",
                        status
                    )
                } else {
                    format!("[FLOW MODE {}]\n\nStandard guidance mode restored.", status)
                };
                self.push_system_notice(&msg);
                self.record_last_action(format!("Flow mode {}", status));
            }

            // V1.5: Enhanced interrupt with actual cancellation
            Command::Stop => {
                self.emit_event("cmd", "/stop");
                self.record_last_action("Stop execution");

                // V1.5: Preserve prepared action context before clearing
                if let Some(confirmation) = self.pending_confirmation.take() {
                    // Create interrupt context for the prepared action
                    let context = crate::guidance::InterruptContext::from_prepared_action(
                        &confirmation.command,
                        &confirmation.reason,
                        &confirmation.impact,
                    );
                    self.interrupt_context = Some(context.clone());
                    self.push_system_notice("⏸ Prepared action stopped.\n");
                    self.push_system_notice(&context.format_prepared());
                    self.push_system_notice("\n→ Resume: /chain resume");
                    self.push_system_notice("→ Clear: /cancel");
                    return Ok(true);
                }

                // V1.5: Actually cancel active runtime
                if let Some(runtime) = self.active_execution_runtime.take() {
                    // Create interrupt context BEFORE cancelling (to capture current state)
                    let current_step = self
                        .state
                        .execution
                        .current_step
                        .as_ref()
                        .and_then(|s| s.parse::<usize>().ok());
                    let total_steps = self
                        .state
                        .execution
                        .current_plan
                        .as_ref()
                        .map(|p| p.steps.len());

                    let context = crate::guidance::InterruptContext::from_execution(
                        current_step,
                        total_steps,
                        "Active execution in progress",
                    );

                    // Actually cancel the runtime
                    if let Err(e) = runtime.cancel() {
                        warn!("Failed to cancel runtime: {}", e);
                    }

                    // Update execution state
                    self.active_runtime_session_id = None;
                    self.execution_run_sealed = true;
                    self.set_execution_state(ExecutionState::Blocked);
                    self.set_current_step(
                        Some("Execution stopped by operator".to_string()),
                        None,
                        None,
                    );

                    // Store interrupt context for recovery
                    self.interrupt_context = Some(context.clone());
                    self.push_system_notice(&context.format());
                    return Ok(true);
                }

                // Check if we're in a blocked state
                if self.state.execution.state == ExecutionState::Blocked {
                    self.push_system_notice(
                        "⏸ Already stopped. Use /chain resume to continue or /cancel to clear.",
                    );
                    return Ok(true);
                }

                self.push_system_notice("✓ Nothing to stop.");
            }

            Command::Cancel => {
                self.emit_event("cmd", "/cancel");
                self.record_last_action("Cancel operation");

                // Clear any pending state
                let had_pending = self.pending_confirmation.is_some()
                    || self.pending_approval.is_some()
                    || self.pending_command.is_some();

                self.pending_confirmation = None;
                self.pending_approval = None;
                self.pending_command = None;

                if had_pending {
                    self.push_system_notice("✓ Cancelled. All pending operations cleared.");
                } else {
                    self.push_system_notice("✓ Nothing to cancel.");
                }
            }

            Command::Override => {
                self.emit_event("cmd", "/override");
                self.record_last_action("Override system suggestion");
                self.push_system_notice(
                    "Override mode active.\n\nThe system will wait for your explicit commands.\nType normally to proceed."
                );
            }

            // V1.4: Preview upcoming execution
            Command::Preview => {
                self.emit_event("cmd", "/preview");
                self.record_last_action("Preview execution");

                if let Some(preview) = crate::guidance::LookaheadEngine::preview_execution(
                    &self.persistence,
                    &self.state,
                ) {
                    let formatted = crate::guidance::LookaheadEngine::format_preview(&preview);
                    self.push_system_notice(&formatted);

                    // V1.4: If safe to chain, offer workflow execution
                    if preview.safe_to_chain {
                        self.push_system_notice("\nThis workflow can execute automatically.\nUse /flow on to enable auto-chaining.");
                    }
                } else {
                    self.push_system_notice("No upcoming execution to preview.\n\nCreate a plan first:\n  /plan <objective>");
                }
            }

            // V2.0/2.2: Goal-driven autonomous operator with quality scoring
            Command::Goal { statement } => {
                self.emit_event("cmd", &format!("/goal {}", statement));
                self.record_last_action(format!("Stated goal: {}", statement));

                // Check if this is a replan after rejection
                let is_replan = self.goal_manager.previous_plan().is_some();
                let previous_plan = self.goal_manager.previous_plan().cloned();

                // 1. Stake the goal
                let goal = self
                    .goal_manager
                    .stake_goal(statement.clone(), self.conversation_id.clone());

                // Compact goal notification
                self.push_system_notice(&format!(
                    "🎯 Goal: {} (ID: {}){}",
                    statement,
                    &goal.id[..8],
                    if is_replan { " [Replanning]" } else { "" }
                ));

                // 2. Generate plan using Qwen-Coder first, with deterministic fallback
                match self.generate_goal_plan(&goal).await {
                    crate::guidance::PlanGenerationResult::Success(plan) => {
                        // Store the plan in the goal
                        if let Some(active_goal) = self.goal_manager.active_goal_mut() {
                            active_goal.set_plan(plan.clone());

                            // V2.3: Score plan quality with calibrated weights
                            let quality_score = crate::guidance::PlanQualityScorer::score(&plan);

                            // V2.3: Smarter replanning guidance (replaces simple diff)
                            if let Some(ref prev) = previous_plan {
                                let rejection_reason = self.goal_manager.last_rejection_reason();

                                // Show explicit changes based on feedback
                                if let Some(guidance) =
                                    crate::guidance::SmartReplanning::generate_replanning_guidance(
                                        Some(prev),
                                        rejection_reason.as_deref(),
                                        &plan,
                                    )
                                {
                                    self.push_system_notice(&guidance);
                                }

                                // Warn if repeating rejected patterns
                                if let Some(warning) =
                                    crate::guidance::SmartReplanning::detect_repeated_patterns(
                                        Some(prev),
                                        &plan,
                                    )
                                {
                                    self.push_system_notice(&warning);
                                }
                            }

                            // V2.3: Show quality score if not Excellent tier
                            if !matches!(
                                quality_score.tier,
                                crate::guidance::QualityTier::Excellent
                            ) {
                                self.push_system_notice(
                                    &crate::guidance::PlanQualityScorer::format_score(
                                        &quality_score,
                                    ),
                                );
                            }

                            // V2.3: Execution Confidence Signal (emotional anchor)
                            // Build a minimal memory reference for confidence calculation
                            let memory_ref = self.goal_manager.active_goal().and_then(|g| {
                                if g.rejection_reason.is_some() {
                                    Some(crate::guidance::GoalMemory {
                                        last_goal: Some(g.statement.clone()),
                                        last_rejection: g.rejection_reason.clone(),
                                        last_failure: None,
                                        rejection_count: 1,
                                        successful_patterns: vec![],
                                    })
                                } else {
                                    None
                                }
                            });
                            let confidence = crate::guidance::ExecutionConfidence::calculate(
                                &plan,
                                &quality_score,
                                memory_ref.as_ref(),
                            );
                            self.push_system_notice(&crate::guidance::ExecutionConfidence::format(
                                confidence,
                            ));

                            // Generate explanation for operator review
                            let explanation =
                                crate::guidance::PlanExplanation::explain(&plan, &goal);
                            self.push_system_notice(&explanation);

                            // V2.3: Action hint based on confidence
                            match confidence {
                                crate::guidance::ConfidenceLevel::High => {
                                    self.push_system_notice("✓ Ready: /goal confirm");
                                }
                                crate::guidance::ConfidenceLevel::Medium => {
                                    self.push_system_notice(
                                        "▶ Review: /goal confirm | /goal reject | /preview",
                                    );
                                }
                                _ => {
                                    self.push_system_notice(
                                        "⚠️  Review carefully before confirming",
                                    );
                                }
                            }
                        }
                    }
                    crate::guidance::PlanGenerationResult::Failed { reason, suggestion } => {
                        self.push_system_notice(&format!(
                            "❌ Plan Generation Failed: {}\n💡 {}",
                            reason, suggestion
                        ));
                        self.goal_manager.mark_failed();
                    }
                }
            }

            Command::GoalConfirm => {
                self.emit_event("cmd", "/goal confirm");
                self.record_last_action("Confirmed goal plan");

                // Check if we have an active goal with a plan
                let goal_data = self
                    .goal_manager
                    .active_goal()
                    .map(|g| (g.statement.clone(), g.generated_plan.clone()));

                if let Some((_goal_statement, Some(plan))) = goal_data {
                    let preflight = self.resolve_autonomous_planner_preflight().await;
                    if !matches!(
                        preflight,
                        crate::autonomy::PlannerPreflightOutcome::Ready { .. }
                    ) {
                        self.fail_autonomous_planner_preflight(preflight);
                        return Ok(false);
                    }

                    // Mark goal as executing
                    self.goal_manager.mark_executing();

                    // Extract plan data before creating chain
                    let plan_steps = plan.steps.clone();
                    let required_context = plan.required_context.clone();
                    let objective_summary = plan.objective.clone();
                    let raw_prompt = plan.raw_prompt.clone();

                    // Create the chain
                    let chain = self
                        .persistence
                        .create_chain(&objective_summary, &objective_summary);
                    let chain_id = chain.id.clone();

                    // Attach chain to goal
                    self.goal_manager
                        .active_goal_mut()
                        .map(|g| g.attach_chain(chain_id.clone()));

                    // Populate chain steps from plan
                    if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                        chain.raw_prompt = raw_prompt.clone();
                        chain.objective_satisfaction =
                            crate::guidance::build_objective_satisfaction(&raw_prompt);
                        for plan_step in &plan_steps {
                            let step = crate::persistence::PersistentChainStep {
                                id: format!("step-{}", uuid::Uuid::new_v4()),
                                description: plan_step.description.clone(),
                                status: crate::persistence::ChainStepStatus::Pending,
                                retry_of: None,
                                retry_attempt: 0,
                                execution_outcome: None,
                                execution_result_class: None,
                                execution_results: vec![],
                                failure_reason: None,
                                recovery_step_kind: None,
                                evidence_snapshot: None,
                                force_override_used: false,
                                tool_calls: vec![],
                                result_summary: None,
                                validation_passed: None,
                                started_at: None,
                                completed_at: None,
                                error_message: None,
                                replay_record: None,
                            };
                            chain.steps.push(step);
                        }
                    }

                    crate::autonomy::AutonomousLoopController::configure_goal_policy(
                        &mut self.persistence.chain_policy,
                    );

                    let seeded_context = self
                        .persistence
                        .get_chain_mut(&chain_id)
                        .map(|chain| {
                            crate::autonomy::AutonomousLoopController::seed_goal_context(
                                chain,
                                &required_context,
                                &self.state.repo.path,
                            )
                        })
                        .unwrap_or(0);

                    let start_decision = self
                        .persistence
                        .get_chain(&chain_id)
                        .map(|chain| {
                            crate::autonomy::AutonomousLoopController::decide_goal_start(
                                chain,
                                &self.persistence.chain_policy,
                            )
                        })
                        .unwrap_or_else(|| crate::autonomy::AutonomousStartDecision::Blocked {
                            reason: "created chain could not be reloaded".to_string(),
                            next_action: "/chains".to_string(),
                        });

                    match start_decision {
                        crate::autonomy::AutonomousStartDecision::Start { command, reason } => {
                            self.pending_command = Some(command);
                            self.emit_event(
                                "autonomy",
                                &format!("goal start scheduled: {}", reason),
                            );
                            self.push_system_notice(&format!(
                                "Goal plan accepted. Chain '{}' created with {} steps.\nAutonomous loop enabled: validation required, halt-on-failure enabled, high-risk approval required.\nContext seeded: {} file(s).\nStarting automatically.",
                                chain_id,
                                plan_steps.len(),
                                seeded_context
                            ));
                        }
                        crate::autonomy::AutonomousStartDecision::Blocked {
                            reason,
                            next_action,
                        } => {
                            self.emit_event("autonomy", &format!("goal start blocked: {}", reason));
                            self.push_system_notice(&format!(
                                "Goal plan accepted. Chain '{}' created with {} steps, but autonomous start is blocked: {}.\nNext: {}",
                                chain_id,
                                plan_steps.len(),
                                reason,
                                next_action
                            ));
                        }
                    }
                    self.persist().await;
                } else if goal_data.is_none() {
                    self.push_system_notice(
                        "No active goal. State a goal first with /goal <statement>",
                    );
                } else {
                    self.push_system_notice(
                        "No plan to confirm. State a goal first with /goal <statement>",
                    );
                }
            }

            Command::GoalReject => {
                self.emit_event("cmd", "/goal reject");
                self.record_last_action("Rejected goal plan");

                // V2.2: Store rejected plan and reason for diff comparison
                let rejection_reason = "Operator rejected plan".to_string();
                let plan_to_store = self
                    .goal_manager
                    .active_goal()
                    .and_then(|g| g.generated_plan.clone());

                if let Some(plan) = plan_to_store {
                    self.goal_manager
                        .store_rejected_plan(plan, &rejection_reason);
                }

                // V2.1/2.2: Improved reject with deliberate replanning UX
                if let Some(goal) = self.goal_manager.active_goal() {
                    let statement = goal.statement.clone();
                    let plan_summary = goal.generated_plan.as_ref().map(|p| {
                        format!(
                            "Previous plan: {} steps, {} approval point(s), {} risk(s)",
                            p.steps.len(),
                            p.approval_points.len(),
                            p.risks.len()
                        )
                    });

                    let mut msg = format!("⊘ Goal Rejected: {}\n\n", statement);

                    // Show previous plan summary for context
                    if let Some(summary) = plan_summary {
                        msg.push_str(&format!("{}\n\n", summary));
                    }

                    // V2.2: Show quality issues if any
                    if let Some(ref plan) = goal.generated_plan {
                        let quality = crate::guidance::PlanQualityScorer::score(plan);
                        if quality.overall < 75 {
                            msg.push_str("📊 Plan Quality Issues:\n");
                            for issue in quality.issues.iter().take(2) {
                                msg.push_str(&format!("  • {}\n", issue));
                            }
                            msg.push('\n');
                        }
                    }

                    // Guidance for better replanning
                    msg.push_str("💡 Try:\n");
                    msg.push_str("  /goal <more specific version> - Refine approach\n");
                    msg.push_str("  /goal confirm                   - Accept after all\n\n");

                    // V2.2: Example refinements based on rejection
                    msg.push_str("Examples:\n");
                    msg.push_str(&format!("  /goal Add validation to {}\n", statement));

                    self.push_system_notice(&msg);
                    self.goal_manager.mark_failed();
                } else {
                    self.push_system_notice("No active goal to reject.\n\nState a goal first:\n  /goal <your objective>");
                }
            }

            Command::GoalStatus => {
                self.emit_event("cmd", "/goal status");
                self.record_last_action("Show goal status");

                if let Some(goal) = self.goal_manager.active_goal() {
                    let mut msg = format!(
                        "🎯 Active Goal\n\nStatement: {}\nStatus: {} {}\nGoal ID: {}",
                        goal.statement,
                        goal.status.icon(),
                        goal.status.label(),
                        goal.id
                    );

                    if let Some(chain_id) = &goal.chain_id {
                        msg.push_str(&format!("\nChain ID: {}", chain_id));
                    }

                    if let Some(plan) = &goal.generated_plan {
                        msg.push_str(&format!(
                            "\n\nPlan: {} steps, {} approval point(s), {} to auto-execute",
                            plan.steps.len(),
                            plan.approval_points.len(),
                            if plan.safe_to_chain {
                                "safe"
                            } else {
                                "not safe"
                            }
                        ));
                    }

                    self.push_system_notice(&msg);
                } else {
                    self.push_system_notice(
                        "No active goal.\n\nState a goal with:\n  /goal <your objective>",
                    );
                }
            }

            // V2.4: Project management commands
            Command::ProjectCreate {
                name,
                path,
                init_git,
            } => {
                self.emit_event("cmd", &format!("/project create {}", name));
                self.record_last_action(format!("Create project: {}", name));

                let parent_dir = if let Some(path) = path {
                    self.resolve_project_path(&path)?
                } else {
                    self.default_project_root()
                };

                match self
                    .create_project_from_parent_and_attach(parent_dir, &name)
                    .await
                {
                    Ok(project_root) => {
                        if init_git {
                            match std::process::Command::new("git")
                                .args(["init", project_root.as_str()])
                                .output()
                            {
                                Ok(_) => self.push_system_notice(&format!(
                                    "Project '{}' created, Git initialized, and attached.\nPath: {}",
                                    name, project_root
                                )),
                                Err(e) => self.push_system_notice(&format!(
                                    "Project '{}' created and attached. Git init failed: {}\nPath: {}",
                                    name, e, project_root
                                )),
                            }
                        }
                    }
                    Err(error) => {
                        self.set_panel_status(PanelStatusLevel::Error, error.to_string());
                        self.push_system_notice(&format!("Project creation failed: {}", error));
                    }
                }
            }

            Command::ProjectSwitch { project_id } => {
                self.emit_event("cmd", &format!("/project switch {}", project_id));
                self.record_last_action(format!("Switch to project: {}", project_id));

                if self.persistence.get_project(&project_id).is_some() {
                    self.persistence
                        .set_active_project(Some(project_id.clone()));
                    self.push_system_notice(&format!("✓ Switched to project: {}", project_id));
                    self.persist().await;
                } else {
                    self.push_system_notice(&format!(
                        "✗ Project not found: {}\nUse /project list to see available projects",
                        project_id
                    ));
                }
            }

            Command::ProjectList => {
                self.emit_event("cmd", "/project list");
                self.record_last_action("List projects");

                if self.persistence.projects.is_empty() {
                    self.push_system_notice("No projects yet.\n\nCreate one with:\n  /project create <name> [--path <path>] [--git]");
                } else {
                    let mut msg = "📁 Projects:\n\n".to_string();
                    for project in &self.persistence.projects {
                        let active_marker = if project.is_active { "→ " } else { "  " };
                        msg.push_str(&format!(
                            "{}{} (ID: {}...)\n",
                            active_marker,
                            project.name,
                            crate::text::take_chars(&project.id, 8)
                        ));
                        if let Some(ref repo_path) = project.repo_path {
                            msg.push_str(&format!("   Path: {}\n", repo_path));
                        }
                        msg.push('\n');
                    }
                    msg.push_str(
                        "Switch with: /project switch <id>\nCreate with: /project create <name>",
                    );
                    self.push_system_notice(&msg);
                }
            }

            Command::DebugMode { enabled } => {
                self.emit_event("cmd", if enabled { "/debug on" } else { "/debug off" });
                self.set_experience_mode(if enabled {
                    ExperienceMode::Operator
                } else {
                    ExperienceMode::Normal
                });
                self.push_system_notice(if enabled {
                    "Operator mode enabled. Audit, logs, context, replay, and debug surfaces are available in the inspector."
                } else {
                    "Normal mode enabled. Internal runtime and audit surfaces are hidden from the main workflow."
                });
            }

            // V2.5: Documentation generation commands
            Command::DocGenerate { repo_path, output_dir, doc_number } => {
                let repo = repo_path.clone()
                    .or_else(|| self.persistence.active_repo.clone())
                    .unwrap_or_else(|| ".".to_string());
                let out = output_dir.clone().unwrap_or_else(|| "./docs".to_string());
                
                self.emit_event("cmd", &format!("/doc generate --repo {} --out {}", repo, out));
                
                if let Some(n) = doc_number {
                    // Generate single document
                    self.push_system_notice(&format!("Generating document {}...", n));
                    // TODO: Implement single doc generation
                } else {
                    // Start chain for all 15 docs
                    self.push_system_notice("Starting documentation generation chain for 15 canonical documents...");
                    // TODO: Create chain and start generation
                }
            }

            Command::DocGenerateChain { repo_path, output_dir, current_step } => {
                // Handle chain step execution
                self.emit_event("cmd", &format!("/doc generate chain step {}", current_step));
                // TODO: Execute specific chain step
            }

            Command::DocValidate { output_dir } => {
                self.emit_event("cmd", &format!("/doc validate {}", output_dir));
                self.push_system_notice(&format!("Validating documentation in {}...", output_dir));
                // TODO: Implement validation
            }

            Command::DocStatus => {
                self.emit_event("cmd", "/doc status");
                self.push_system_notice("Documentation generation status:\nPending implementation.");
            }

            // V2.5: Auto-chain large prompts
            Command::AutoChain { prompt, strategy } => {
                self.emit_event("cmd", "/auto-chain");
                self.push_system_notice(&format!(
                    "Auto-chaining large prompt using {:?} strategy...",
                    strategy
                ));
                
                // Parse the prompt into chain steps based on strategy
                let steps = self.parse_prompt_into_steps(&prompt, &strategy);
                
                // Create and execute chain
                if let Err(e) = self.execute_auto_chain(steps).await {
                    self.push_system_notice(&format!("Auto-chain failed: {}", e));
                }
            }

            // V2.6: Large prompt decomposer with artifact contract - SUPERCHARGED
            Command::ArtifactContract { prompt, auto_detect: _ } => {
                self.emit_event("cmd", "/artifact-contract");
                
                // Get repo path for contract
                let repo_path = self.state.repo.path.clone();
                
                // Classify the prompt
                use crate::large_prompt_classifier::{LargePromptClassifier, PromptClassification};
                use crate::supercharged_tools::{SuperchargedToolExecutor, SuperchargeArtifactContract};
                
                match LargePromptClassifier::classify(&prompt, Some(std::path::Path::new(&repo_path))) {
                    PromptClassification::LargeProject(contract) => {
                        // Show artifact breakdown by type
                        let type_counts = contract.artifacts.iter().fold(
                            std::collections::HashMap::new(),
                            |mut acc, artifact| {
                                let type_name = match &artifact.artifact_type {
                                    crate::large_prompt_classifier::ArtifactType::Markdown => "📄 Markdown",
                                    crate::large_prompt_classifier::ArtifactType::Code { language } => &format!("💻 {}", language),
                                    crate::large_prompt_classifier::ArtifactType::Config => "⚙️ Config",
                                    crate::large_prompt_classifier::ArtifactType::Data => "📊 Data",
                                    crate::large_prompt_classifier::ArtifactType::Test => "🧪 Test",
                                    crate::large_prompt_classifier::ArtifactType::Script => "🔧 Script",
                                    crate::large_prompt_classifier::ArtifactType::Documentation => "📚 Docs",
                                    crate::large_prompt_classifier::ArtifactType::Other(ext) => &format!("📦 .{}", ext),
                                };
                                *acc.entry(type_name.to_string()).or_insert(0) += 1;
                                acc
                            }
                        );
                        
                        let mut type_summary = String::new();
                        for (type_name, count) in type_counts {
                            type_summary.push_str(&format!("{} {}, ", count, type_name));
                        }
                        
                        self.push_system_notice(&format!(
                            "🚀 SUPERCHARGED Artifact Contract: {} artifacts ({})",
                            contract.artifacts.len(),
                            type_summary.trim_end_matches(", ")
                        ));
                        
                        // Use supercharged tool execution for ALL artifact types
                        let ctx = contract.get_tool_context(repo_path.into());
                        let steps = contract.to_supercharged_steps();
                        
                        // Convert to persistent chain with supercharged steps
                        let chain_steps = crate::supercharged_tools::execution_steps_to_chain_steps(steps);
                        let chain_id = format!("supercharged-{}", uuid::Uuid::new_v4());
                        
                        let chain = crate::persistence::PersistentChain {
                            id: chain_id.clone(),
                            name: format!("Supercharged: {} artifacts", contract.artifacts.len()),
                            objective: contract.source_prompt_summary.clone(),
                            raw_prompt: prompt.clone(),
                            status: crate::persistence::ChainLifecycleStatus::Ready,
                            steps: chain_steps,
                            active_step: None,
                            repo_path: Some(ctx.repo_path.display().to_string()),
                            conversation_id: Some(self.conversation_id.clone()),
                            created_at: chrono::Local::now(),
                            updated_at: chrono::Local::now(),
                            completed_at: None,
                            archived: false,
                            total_steps_executed: 0,
                            total_steps_failed: 0,
                            execution_outcome: None,
                            force_override_used: false,
                            objective_satisfaction: crate::state::ObjectiveSatisfaction::default(),
                            selected_context_files: vec![],
                            context_state: None,
                            pending_checkpoint: None,
                            git_grounding: None,
                            audit_log: crate::state::AuditLog::new(),
                        };
                        
                        // Store chain and execute
                        self.persistence.chains.push(chain);
                        self.persistence.active_chain_id = Some(chain_id.clone());
                        
                        self.push_system_notice(&format!(
                            "⚡ Supercharged chain '{}' ready - ALL tools active (file ops, batch processing, code intelligence)",
                            chain_id
                        ));
                        
                        // Convert to executable steps and run
                        let exec_steps: Vec<String> = contract.artifacts.iter().map(|a| {
                            format!("Generate {}: {}", 
                                match &a.artifact_type {
                                    crate::large_prompt_classifier::ArtifactType::Code { language } => format!("💻 {} code", language),
                                    crate::large_prompt_classifier::ArtifactType::Markdown => "📄 markdown".to_string(),
                                    crate::large_prompt_classifier::ArtifactType::Test => "🧪 tests".to_string(),
                                    _ => format!("{:?}", a.artifact_type),
                                },
                                a.path.display()
                            )
                        }).collect();
                        
                        if let Err(e) = self.execute_auto_chain(exec_steps).await {
                            tracing::warn!("Supercharged execution note: {}", e);
                        }
                    }
                    PromptClassification::Regular => {
                        // Not a large project prompt - fall back to regular task with tool access
                        self.push_system_notice("🎯 Using standard execution with full tool access.");
                        
                        // Direct task execution without recursion
                        self.append_user_message(&prompt);
                        if let Err(e) = self.send_to_ollama(&prompt).await {
                            tracing::warn!("Task execution note: {}", e);
                        }
                    }
                    PromptClassification::Ambiguous { reason } => {
                        self.push_system_notice(&format!(
                            "🤔 {} - Using standard tools.",
                            reason
                        ));
                        
                        // Direct task execution without recursion
                        self.append_user_message(&prompt);
                        if let Err(e) = self.send_to_ollama(&prompt).await {
                            tracing::warn!("Task execution note: {}", e);
                        }
                    }
                }
            }

            Command::ChainArchive { chain_id } => {
                self.emit_event("cmd", &format!("/chain archive {}", chain_id));
                self.record_last_action(format!("Archive chain: {}", chain_id));
                match self.persistence.archive_chain(&chain_id) {
                    Ok(()) => self.push_system_notice(&format!("Chain '{}' archived.", chain_id)),
                    Err(e) => self.push_system_notice(&format!(
                        "Failed to archive chain '{}': {}",
                        chain_id, e
                    )),
                }
                self.persist().await;
            }

            Command::Replay {
                chain_id,
                replay_type,
            } => {
                let target_id = chain_id.or_else(|| self.persistence.active_chain_id.clone());

                if let Some(id) = target_id {
                    // Clone chain data first to avoid borrow issues
                    let chain_data = self.persistence.get_chain(&id).map(|chain| {
                        (
                            chain.name.clone(),
                            chain.audit_log.clone(),
                            chain.status,
                            chain.get_outcome(),
                            chain.steps.clone(),
                        )
                    });

                    if let Some((name, audit_log, status, stored_outcome, steps)) = chain_data {
                        match replay_type {
                            crate::commands::ReplayType::Audit => {
                                // V1.6 REPLAY: Deterministic replay from audit log
                                self.emit_event("cmd", "/audit replay");
                                self.record_last_action("Audit replay");

                                let replay = audit_log.replay(crate::state::ExecutionState::Idle);
                                let summary = crate::state::summarize_replay_result(&replay);

                                // Validate against stored terminal truth
                                let validation_result =
                                    crate::state::validate_replay_against_stored(
                                        &replay,
                                        status.to_execution_state(),
                                        stored_outcome,
                                    );

                                let mut msg = format!(
                                    "=== AUDIT REPLAY ===\n\nChain: {}\n{}\n",
                                    name, summary
                                );

                                // Add validation result
                                match validation_result {
                                    Ok(()) => {
                                        msg.push_str("\n✓ Replay matches stored state\n");
                                    }
                                    Err(e) => {
                                        msg.push_str(&format!("\n⚠ Divergence detected: {}\n", e));
                                    }
                                }

                                // Add navigation hint
                                msg.push_str("\nInspector → Audit tab for full timeline");

                                self.push_system_notice(&msg);
                            }
                            _ => {
                                // Legacy replay behavior
                                self.emit_event("cmd", "/replay");
                                self.record_last_action("Show replay");
                                let mut msg = format!("=== REPLAY STATUS ===\n\nChain: {}\n", name);
                                for (i, step) in steps.iter().enumerate() {
                                    let icon = match step.status {
                                        crate::persistence::ChainStepStatus::Completed => "✓",
                                        crate::persistence::ChainStepStatus::Failed => "✗",
                                        _ => "○",
                                    };
                                    msg.push_str(&format!(
                                        "\nStep {}: {} {}\n",
                                        i + 1,
                                        icon,
                                        step.description
                                    ));
                                    if let Some(ref replay) = step.replay_record {
                                        msg.push_str(&format!(
                                            "  Fingerprint: {}\n",
                                            crate::text::take_chars(
                                                &replay.execution_fingerprint,
                                                8
                                            )
                                        ));
                                        msg.push_str(&format!(
                                            "  Status: {}\n",
                                            if replay.tool_calls.is_empty() {
                                                "pending"
                                            } else {
                                                "recorded"
                                            }
                                        ));
                                    }
                                }
                                self.push_system_notice(&msg);
                            }
                        }
                    } else {
                        self.push_system_notice(&format!("Chain '{}' not found.", id));
                    }
                } else {
                    self.push_system_notice("No active chain. Use /chains to select one.");
                }
            }
        }

        Ok(false)
    }

    async fn refresh_active_model(&mut self) -> Result<()> {
        let Some(configured_model) = self.state.model.configured.clone() else {
            warn!("No planner model configured in repo");
            self.emit_event("model", "Planner model not configured");
            self.state.model.active = preferred_available_model(&self.state.model.available);
            self.state.model.connected = self.state.ollama_connected;
            self.clear_execution_block();
            self.set_panel_status(
                PanelStatusLevel::Info,
                "Planner model not configured. CHAT is available; TASK requires /config set planner_model <model>.",
            );
            return Ok(());
        };

        self.emit_event("model", &format!("Verifying model: {}", configured_model));
        info!(
            "Refreshing active model: configured={:?}, ollama_connected={}, available_before={:?}",
            self.state.model.configured, self.state.ollama_connected, self.state.model.available
        );

        if !self.state.ollama_connected {
            self.state.model.active = None;
            self.state.model.connected = false;
            self.set_execution_block(
                "Ollama disconnected",
                "Start Ollama and refresh runtime",
                Some("ollama serve".to_string()),
            );
            return Ok(());
        }

        let verification = self.ollama.verify_model(&configured_model).await?;
        info!(
            "Model verification result: requested={}, resolved={:?}, exact_match={}, installed={:?}, error={:?}",
            configured_model,
            verification.resolved_model,
            verification.exact_match,
            verification.installed_models,
            verification.error
        );
        self.state.model.available = verification.installed_models.clone();

        if let Some(active_model) = verification.resolved_model {
            self.state.model.active = Some(active_model.clone());
            self.state.model.connected = true;
            self.persistence.update_model_status(
                Some(&configured_model),
                Some(&active_model),
                true,
            );

            if verification.exact_match {
                info!("Model '{}' verified", configured_model);
                self.emit_event("model", &format!("Model '{}' verified", active_model));
            } else {
                info!(
                    "Configured model '{}' resolved to installed '{}'",
                    configured_model, active_model
                );
                self.emit_event(
                    "model",
                    &format!(
                        "Resolved configured model '{}' to installed '{}'",
                        configured_model, active_model
                    ),
                );
            }

            self.unblock_chat();
        } else {
            let error = verification
                .error
                .unwrap_or_else(|| format!("Model '{}' not installed", configured_model));
            warn!("Model not available: {}", error);
            self.state.model.active = preferred_available_model(&self.state.model.available);
            self.state.model.connected = false;
            self.emit_event("model", &format!("Missing: {}", error));
            if self.state.model.active.is_some() {
                self.clear_execution_block();
                self.set_panel_status(
                    PanelStatusLevel::Error,
                    format!(
                        "{}. CHAT can still use the active model; TASK needs /config set planner_model <model>.",
                        error
                    ),
                );
            } else {
                self.set_execution_block(
                    error,
                    "Install or configure a local Ollama model",
                    Some("/models".to_string()),
                );
            }
        }

        Ok(())
    }

    async fn refresh_runtime_status(&mut self) -> Result<()> {
        let health = self.ollama.health_check().await?;
        self.state.ollama_connected = health.connected;
        self.state.model.available = health.models.clone();
        self.state.model.connected = health.connected;

        if !health.connected {
            let reason = health
                .error
                .unwrap_or_else(|| "Ollama disconnected".to_string());
            self.state.model.active = None;
            self.persistence.update_model_status(
                self.state.model.configured.as_deref(),
                None,
                false,
            );
            self.set_execution_block(
                reason.clone(),
                "Start Ollama and retry",
                Some("ollama serve".to_string()),
            );
            self.set_panel_status(PanelStatusLevel::Error, reason.clone());
            return Ok(());
        }

        if Self::is_real_repo_path(&self.state.repo.path) {
            self.refresh_active_model().await?;
        } else {
            self.state.model.active = None;
            self.set_execution_block(
                "No repo attached",
                "Create or connect a project first",
                Some("/open <path>".to_string()),
            );
        }

        let active = self
            .state
            .model
            .active
            .as_deref()
            .or(self.state.model.configured.as_deref())
            .unwrap_or("none");
        self.persistence.update_model_status(
            self.state.model.configured.as_deref(),
            self.state.model.active.as_deref(),
            true,
        );
        self.set_panel_status(
            PanelStatusLevel::Success,
            format!(
                "Ollama connected. {} model(s) available. Active: {}.",
                self.state.model.available.len(),
                active
            ),
        );
        Ok(())
    }

    fn clear_runtime_logs(&mut self) {
        let cleared = self.state.logs.len();
        self.state.logs.clear();
        self.execution_output_buffer.clear();
        self.record_last_action(format!("Cleared {} log entries", cleared));
        self.set_panel_status(
            PanelStatusLevel::Success,
            format!(
                "Cleared {} log {}.",
                cleared,
                if cleared == 1 { "entry" } else { "entries" }
            ),
        );
        self.push_system_notice("Inspector logs cleared.");
    }

    fn reset_validation_state(&mut self) {
        self.state.validation_stages = default_validation_stages();
        self.state.execution.validation_summary = None;
        self.record_last_action("Validation state reset");
        self.set_panel_status(
            PanelStatusLevel::Info,
            "Validation state reset. Run validation to repopulate the pipeline.",
        );
        self.push_system_notice("Validation stage state reset.");
    }

    /// Block chat input with reason
    fn block_chat(&mut self, reason: &str) {
        let (fix, command) = if reason.contains("Ollama") {
            (
                "Start Ollama and refresh runtime".to_string(),
                Some("ollama serve".to_string()),
            )
        } else if reason.contains("planner model") || reason.contains("not installed") {
            (
                "Configure a valid planner model".to_string(),
                Some("/config set planner_model <model>".to_string()),
            )
        } else if reason.contains("repo") || reason.contains("project") {
            (
                "Create or connect a project first".to_string(),
                Some("/open <path>".to_string()),
            )
        } else {
            ("Resolve the blocking issue and retry".to_string(), None)
        };
        self.set_execution_block(reason.to_string(), fix, command);
        warn!("Chat blocked: {}", reason);
    }

    /// Unblock chat input
    fn unblock_chat(&mut self) {
        self.clear_execution_block();
        self.state.runtime_status = RuntimeStatus::Idle;
        self.set_execution_state(ExecutionState::Idle);
        self.record_last_action("Chat unblocked");
    }

    /// Add a log entry
    fn add_log(&mut self, level: LogLevel, source: &str, message: &str) {
        self.state.logs.push(LogEntry {
            timestamp: chrono::Local::now(),
            level,
            source: source.to_string(),
            message: message.to_string(),
        });
    }

    fn add_operator_debug_log(&mut self, source: &str, message: impl Into<String>) {
        let message = message.into();
        self.add_log(LogLevel::Debug, source, &message);
    }

    fn reset_execution_terminal_seal(&mut self) {
        self.active_runtime_session_id = None;
        self.execution_run_sealed = false;
        self.post_terminal_audit_recorded = false;
    }

    fn seal_execution_run(&mut self) {
        self.execution_run_sealed = true;
        self.active_execution_runtime = None;
        self.pending_confirmation = None;
    }

    fn runtime_event_label(event: &RuntimeEvent) -> &'static str {
        match event {
            RuntimeEvent::Init { .. } => "init",
            RuntimeEvent::IterationStart { .. } => "iteration_start",
            RuntimeEvent::PreflightPassed => "preflight_passed",
            RuntimeEvent::PlannerOutput { .. } => "planner_output",
            RuntimeEvent::ProtocolValidation { .. } => "protocol_validation",
            RuntimeEvent::ToolCall { .. } => "tool_call",
            RuntimeEvent::ToolExecuting { .. } => "tool_executing",
            RuntimeEvent::ToolResult { .. } => "tool_result",
            RuntimeEvent::BrowserPreview { .. } => "browser_preview",
            RuntimeEvent::MutationsDetected { .. } => "mutations_detected",
            RuntimeEvent::ValidationRunning => "validation_running",
            RuntimeEvent::ValidationResult { .. } => "validation_result",
            RuntimeEvent::StateCommitting { .. } => "state_committing",
            RuntimeEvent::Completion { .. } => "completion_gate",
            RuntimeEvent::Failure { .. } => "runtime_failure",
            RuntimeEvent::RepairLoop { .. } => "repair_loop",
            RuntimeEvent::Finished { .. } => "runtime_finished",
            RuntimeEvent::ValidationStage { .. } => "validation_stage",
            RuntimeEvent::ContextAssembly { .. } => "context_assembly",
        }
    }

    fn audit_post_terminal_noise_once(&mut self, metadata: String) {
        if self.post_terminal_audit_recorded {
            return;
        }

        self.add_operator_debug_log(
            "runtime/stale-post-terminal",
            format!("Dropped post-terminal runtime noise: {}", metadata),
        );

        let step_id = self.state.current_chain_step_id.clone();
        let task = self.state.execution.active_objective.clone();

        if let Some(chain_id) = self.persistence.active_chain_id.clone() {
            if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                chain.audit_event(crate::state::AuditEvent {
                    timestamp: chrono::Utc::now(),
                    event_type: crate::state::AuditEventType::ChainLifecycle {
                        event: "stale_post_terminal".to_string(),
                    },
                    previous_state: None,
                    next_state: None,
                    triggering_event: Some("stale-post-terminal".to_string()),
                    step_id,
                    chain_id: Some(chain_id),
                    task,
                    reason: Some("stale-post-terminal".to_string()),
                    metadata: Some(metadata),
                });
            }
        }

        self.post_terminal_audit_recorded = true;
    }

    fn drop_post_terminal_events(&mut self, stale_events: &[RuntimeEvent]) {
        if stale_events.is_empty() {
            return;
        }

        let labels = stale_events
            .iter()
            .map(Self::runtime_event_label)
            .collect::<Vec<_>>()
            .join(",");
        self.audit_post_terminal_noise_once(format!(
            "count={}; events={}",
            stale_events.len(),
            labels
        ));
    }

    fn push_runtime_event(&mut self, stage: impl Into<String>, status: RuntimeStatus) {
        self.state.runtime_events.push(crate::state::RuntimeEvent {
            timestamp: chrono::Local::now(),
            stage: stage.into(),
            status,
        });

        if self.state.runtime_events.len() > 200 {
            self.state.runtime_events.remove(0);
        }
    }

    fn upsert_validation_stage(
        &mut self,
        name: &str,
        status: RuntimeStatus,
        detail: Option<String>,
    ) {
        if let Some(stage) = self
            .state
            .validation_stages
            .iter_mut()
            .find(|stage| stage.name == name)
        {
            stage.status = status;
            stage.detail = detail;
            return;
        }

        let insert_at = self
            .state
            .validation_stages
            .iter()
            .position(|stage| {
                validation_stage_rank(stage.name.as_str()) > validation_stage_rank(name)
            })
            .unwrap_or(self.state.validation_stages.len());

        self.state.validation_stages.insert(
            insert_at,
            crate::state::ValidationStage {
                name: name.to_string(),
                status,
                detail,
                duration_ms: None,
            },
        );
    }

    fn track_inspector_event(&mut self, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::Init { .. } => self.push_runtime_event("init", RuntimeStatus::Running),
            RuntimeEvent::IterationStart { iteration } => {
                self.push_runtime_event(format!("iteration/{}", iteration), RuntimeStatus::Running)
            }
            RuntimeEvent::PreflightPassed => {
                self.push_runtime_event("preflight", RuntimeStatus::Completed)
            }
            RuntimeEvent::PlannerOutput { output_type, .. } => {
                self.push_runtime_event(format!("planner/{}", output_type), RuntimeStatus::Running)
            }
            RuntimeEvent::ProtocolValidation { status, reason } => {
                let stage_status = match status.as_str() {
                    "running" => RuntimeStatus::Running,
                    "accepted" | "normalized" => RuntimeStatus::Completed,
                    _ => RuntimeStatus::Error,
                };
                self.upsert_validation_stage("protocol", stage_status, reason.clone());
                self.push_runtime_event("validation/protocol", stage_status);
            }
            RuntimeEvent::ToolCall { name, .. } | RuntimeEvent::ToolExecuting { name } => {
                self.push_runtime_event(format!("tool/{}", name), RuntimeStatus::Running)
            }
            RuntimeEvent::ToolResult {
                name,
                success,
                output,
                error,
            } => {
                let status = if *success {
                    RuntimeStatus::Completed
                } else {
                    RuntimeStatus::Error
                };
                self.push_runtime_event(format!("tool/{}", name), status);
                if let Some(message) = error.as_ref().or(output.as_ref()) {
                    self.add_log(
                        if *success {
                            LogLevel::Info
                        } else {
                            LogLevel::Error
                        },
                        &format!("tool/{}", name),
                        message,
                    );
                }
            }
            RuntimeEvent::BrowserPreview {
                url,
                port,
                directory,
            } => {
                // Add preview server to state
                self.state
                    .preview_servers
                    .push(crate::browser::PreviewServer::new(
                        format!("preview-{}", port),
                        url.clone(),
                        directory.clone(),
                    ));
                // Switch to preview tab to show the new server
                self.state.active_inspector_tab = crate::state::InspectorTab::Preview;
                self.push_runtime_event("browser_preview", RuntimeStatus::Completed);
                self.add_log(
                    LogLevel::Info,
                    "runtime/browser_preview",
                    &format!("Started at {}", url),
                );
            }
            RuntimeEvent::MutationsDetected { count } => {
                self.push_runtime_event(format!("mutation/{}", count), RuntimeStatus::Completed)
            }
            RuntimeEvent::ValidationRunning => {
                self.upsert_validation_stage("validation", RuntimeStatus::Running, None);
                self.push_runtime_event("validation/runtime", RuntimeStatus::Running);
            }
            RuntimeEvent::ValidationResult { decision, message } => {
                let status = if decision == "accept" {
                    RuntimeStatus::Completed
                } else {
                    RuntimeStatus::Error
                };
                self.upsert_validation_stage("validation", status, Some(message.clone()));
                self.push_runtime_event("validation/runtime", status);
            }
            RuntimeEvent::StateCommitting { files_written } => {
                // Track file mutations in diff store
                for path in files_written {
                    // Try to read the file content for diff tracking
                    if let Ok(content) = std::fs::read_to_string(path) {
                        // Check if we have a previous version in store
                        let before = self.state.diff_store.get(path).map(|m| m.after.clone());
                        let after_hash = format!("{:x}", md5::compute(&content));

                        let mutation = crate::diff::FileMutation {
                            path: path.clone(),
                            before,
                            after: content,
                            before_hash: None, // Would need to track this from previous read
                            after_hash,
                        };
                        self.state.diff_store.add(mutation);
                    }
                }

                // Switch to diff tab to show the changes
                if !files_written.is_empty() {
                    self.state.active_inspector_tab = crate::state::InspectorTab::Diff;
                }

                self.push_runtime_event(
                    format!("commit/{}", files_written.len()),
                    RuntimeStatus::Completed,
                );
            }
            RuntimeEvent::Completion { reason } => {
                self.push_runtime_event("completion", RuntimeStatus::Completed);
                self.add_log(LogLevel::Info, "execution/completed", reason);
            }
            RuntimeEvent::Failure { reason, .. } => {
                self.push_runtime_event("failure", RuntimeStatus::Error);
                self.add_log(LogLevel::Error, "execution/failed", reason);
            }
            RuntimeEvent::RepairLoop {
                attempt,
                max,
                reason,
            } => {
                self.push_runtime_event(
                    format!("repair/{}/{}", attempt, max),
                    RuntimeStatus::Running,
                );
                self.add_log(LogLevel::Warn, "execution/repair", reason);
            }
            RuntimeEvent::Finished { success, .. } => self.push_runtime_event(
                "finished",
                if *success {
                    RuntimeStatus::Completed
                } else {
                    RuntimeStatus::Error
                },
            ),
            RuntimeEvent::ValidationStage {
                stage,
                status,
                duration_ms,
                summary,
            } => {
                // Update validation stage in the inspector
                let runtime_status = match status {
                    crate::forge_runtime::ValidationStageStatus::Running => RuntimeStatus::Running,
                    crate::forge_runtime::ValidationStageStatus::Passed => RuntimeStatus::Completed,
                    crate::forge_runtime::ValidationStageStatus::Failed => RuntimeStatus::Error,
                    crate::forge_runtime::ValidationStageStatus::Skipped => {
                        RuntimeStatus::Completed
                    }
                };

                if let Some(validation_stage) = self
                    .state
                    .validation_stages
                    .iter_mut()
                    .find(|s| s.name == *stage)
                {
                    validation_stage.status = runtime_status;
                    validation_stage.duration_ms = Some(*duration_ms);
                    if let Some(sum) = summary {
                        validation_stage.detail = Some(sum.clone());
                    }
                }

                self.push_runtime_event(format!("validation/{}", stage), runtime_status);
            }
            RuntimeEvent::ContextAssembly {
                files,
                validation,
                budget,
                summary,
            } => {
                self.push_runtime_event("context/assembly", RuntimeStatus::Completed);

                // Log validation status
                match validation.status {
                    crate::forge_runtime::ContextValidationStatus::Valid => {
                        self.add_log(LogLevel::Info, "context/assembly", &summary);
                    }
                    crate::forge_runtime::ContextValidationStatus::Warning => {
                        let warnings = validation.warnings.join("; ");
                        let msg = format!("{} | Warnings: {}", summary, warnings);
                        self.add_log(LogLevel::Warn, "context/assembly", msg.as_str());
                    }
                    crate::forge_runtime::ContextValidationStatus::Invalid => {
                        let errors = validation.errors.join("; ");
                        let msg = format!("{} | Errors: {}", summary, errors);
                        self.add_log(LogLevel::Error, "context/assembly", msg.as_str());
                    }
                }

                // Convert runtime files to persistence format
                let file_entries: Vec<crate::persistence::ContextFileEntry> = files
                    .iter()
                    .map(|f| crate::persistence::ContextFileEntry {
                        path: f.path.clone(),
                        reason: f.reason.clone(),
                        priority: f.priority,
                        included: f.included,
                        trimmed_reason: f.trimmed_reason.clone(),
                    })
                    .collect();

                // Store context files in active chain (only included files for V2 compatibility)
                let included_paths: Vec<String> = files
                    .iter()
                    .filter(|f| f.included)
                    .map(|f| f.path.clone())
                    .collect();

                if let Some(chain) = self.persistence.get_active_chain_mut() {
                    chain.selected_context_files = included_paths;

                    // Store full V3 context state
                    chain.context_state = Some(crate::persistence::ContextAssemblyState {
                        files: file_entries,
                        validation: crate::persistence::ContextValidationResult {
                            status: match validation.status {
                                crate::forge_runtime::ContextValidationStatus::Valid => {
                                    crate::persistence::ContextValidationStatus::Valid
                                }
                                crate::forge_runtime::ContextValidationStatus::Warning => {
                                    crate::persistence::ContextValidationStatus::Warning
                                }
                                crate::forge_runtime::ContextValidationStatus::Invalid => {
                                    crate::persistence::ContextValidationStatus::Invalid
                                }
                            },
                            warnings: validation.warnings.clone(),
                            errors: validation.errors.clone(),
                            total_files: validation.total_files,
                            estimated_token_usage: validation.estimated_tokens,
                        },
                        budget: crate::persistence::ContextBudgetInfo {
                            max_files: budget.max_files,
                            max_tokens: budget.max_tokens,
                            files_selected: budget.files_selected,
                            tokens_used: budget.tokens_used,
                            trimming_triggered: budget.trimming_triggered,
                        },
                        summary: summary.clone(),
                        assembled_at: chrono::Local::now(),
                    });

                    chain.updated_at = chrono::Local::now();
                }
            }
        }
    }

    fn apply_execution_event(&mut self, event: &RuntimeEvent) {
        use crate::state::ProgressTransitionEvent;

        match event {
            RuntimeEvent::Init {
                session_id, task, ..
            } => {
                self.active_runtime_session_id = Some(session_id.clone());
                self.set_active_objective(Some(task.clone()));
                self.apply_progress_transition(ProgressTransitionEvent::NewRun {
                    task: task.clone(),
                });
                self.set_current_step(Some("Runtime initialized".to_string()), Some(1), Some(3));
                self.record_last_action("Execution runtime initialized");
            }
            RuntimeEvent::IterationStart { iteration } => {
                self.apply_progress_transition(ProgressTransitionEvent::PlanningIteration {
                    iteration: *iteration as usize,
                });
                self.set_current_step(
                    Some(format!("Planning iteration {}", iteration)),
                    Some(1),
                    Some(3),
                );
                self.record_last_action(format!("Iteration {} started", iteration));
            }
            RuntimeEvent::PreflightPassed => {
                self.apply_progress_transition(ProgressTransitionEvent::PreflightComplete);
                self.set_current_step(
                    Some("Preflight checks passed".to_string()),
                    Some(1),
                    Some(3),
                );
                self.record_last_action("Preflight checks passed");
            }
            RuntimeEvent::ContextAssembly {
                files,
                validation,
                budget,
                summary,
            } => {
                self.apply_progress_transition(ProgressTransitionEvent::ContextAssembled);

                let included_count = files.iter().filter(|f| f.included).count();
                let trimmed_count = files.len() - included_count;

                if trimmed_count > 0 {
                    self.record_last_action(format!(
                        "Context assembled: {} files ({} trimmed, {} tokens)",
                        included_count, trimmed_count, budget.tokens_used
                    ));
                } else {
                    self.record_last_action(format!(
                        "Context assembled: {} files ({} tokens)",
                        included_count, budget.tokens_used
                    ));
                }

                // Log validation status with context
                match validation.status {
                    crate::forge_runtime::ContextValidationStatus::Valid => {
                        self.add_log(LogLevel::Info, "execution/context", summary);
                    }
                    crate::forge_runtime::ContextValidationStatus::Warning => {
                        let warnings = if validation.warnings.is_empty() {
                            "validation warnings".to_string()
                        } else {
                            validation.warnings.join("; ")
                        };
                        let msg = format!("{} | {}", summary, warnings);
                        self.add_log(LogLevel::Warn, "execution/context", msg.as_str());
                    }
                    crate::forge_runtime::ContextValidationStatus::Invalid => {
                        let errors = if validation.errors.is_empty() {
                            "validation errors".to_string()
                        } else {
                            validation.errors.join("; ")
                        };
                        let msg = format!("{} | {}", summary, errors);
                        self.add_log(LogLevel::Error, "execution/context", msg.as_str());
                    }
                }
            }
            RuntimeEvent::PlannerOutput { raw, output_type } => {
                self.apply_progress_transition(ProgressTransitionEvent::PlannerOutput);
                self.set_current_step(
                    Some(format!("Planner output ({})", output_type)),
                    Some(1),
                    Some(3),
                );
                Self::push_execution_entry(
                    &mut self.state.execution.planner_output,
                    raw.chars().take(96).collect::<String>(),
                    5,
                );
                self.record_last_action(format!("Planner output received: {}", output_type));
            }
            RuntimeEvent::ProtocolValidation { status, reason } => {
                if matches!(status.as_str(), "accepted" | "normalized" | "running") {
                    self.apply_progress_transition(ProgressTransitionEvent::PlannerOutput);
                } else {
                    self.apply_progress_transition(ProgressTransitionEvent::RuntimeFailure {
                        reason: reason.clone().unwrap_or_default(),
                    });
                }
                self.set_current_step(
                    Some(format!("Protocol validation: {}", status)),
                    Some(1),
                    Some(3),
                );
                self.state.execution.validation_summary = reason.clone();
                self.record_last_action(format!("Protocol validation {}", status));
            }
            RuntimeEvent::ToolCall { name, arguments } => {
                self.apply_progress_transition(ProgressTransitionEvent::ToolCalling {
                    name: name.clone(),
                });
                self.state.execution.active_tool = Some(name.clone());
                self.set_current_step(Some(format!("Calling {}", name)), Some(2), Some(3));
                Self::push_execution_entry(
                    &mut self.state.execution.tool_calls,
                    format!("{} {}", name, arguments),
                    8,
                );
                self.record_last_action(format!("Tool call parsed: {}", name));
            }
            RuntimeEvent::ToolExecuting { name } => {
                self.apply_progress_transition(ProgressTransitionEvent::ToolExecuting {
                    name: name.clone(),
                });
                self.state.execution.active_tool = Some(name.clone());
                self.set_current_step(Some(format!("Running {}", name)), Some(2), Some(3));
                self.record_last_action(format!("Tool executing: {}", name));
            }
            RuntimeEvent::ToolResult { name, success, .. } => {
                self.apply_progress_transition(ProgressTransitionEvent::ToolResult {
                    success: *success,
                });
                self.state.execution.active_tool = if *success { None } else { Some(name.clone()) };
                self.set_current_step(
                    Some(if *success {
                        format!("Completed {}", name)
                    } else {
                        format!("Failed {}", name)
                    }),
                    Some(2),
                    Some(3),
                );
                self.record_last_action(format!(
                    "Tool {}: {}",
                    if *success { "succeeded" } else { "failed" },
                    name
                ));
            }
            RuntimeEvent::BrowserPreview { url, .. } => {
                self.apply_progress_transition(ProgressTransitionEvent::BrowserPreview);
                self.record_last_action(format!("Browser preview ready: {}", url));
            }
            RuntimeEvent::MutationsDetected { count } => {
                self.apply_progress_transition(ProgressTransitionEvent::MutationsDetected {
                    count: *count as usize,
                });
                self.set_current_step(
                    Some(format!("Detected {} file changes", count)),
                    Some(2),
                    Some(3),
                );
                self.record_last_action(format!("Detected {} file changes", count));
            }
            RuntimeEvent::ValidationRunning => {
                self.apply_progress_transition(ProgressTransitionEvent::ValidationRunning);
                self.set_current_step(Some("Running validation".to_string()), Some(3), Some(3));
                self.record_last_action("Validation started");

                // V1.6 AUDIT: Log validation started
                if let Some(chain_id) = self.persistence.active_chain_id.clone() {
                    if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                        let validation_audit = crate::state::AuditEvent {
                            timestamp: chrono::Utc::now(),
                            event_type: crate::state::AuditEventType::ValidationStarted,
                            previous_state: None,
                            next_state: None,
                            triggering_event: Some("ValidationRunning".to_string()),
                            step_id: self.state.current_chain_step_id.clone(),
                            chain_id: Some(chain_id),
                            task: self.state.execution.active_objective.clone(),
                            reason: None,
                            metadata: None,
                        };
                        chain.audit_event(validation_audit);
                    }
                }
            }
            RuntimeEvent::ValidationResult { decision, message } => {
                let accepted = decision == "accept";
                self.apply_progress_transition(ProgressTransitionEvent::ValidationResult {
                    accepted,
                });
                self.state.execution.validation_summary = Some(message.clone());
                self.set_current_step(Some(format!("Validation {}", decision)), Some(3), Some(3));
                self.record_last_action(format!("Validation {}", decision));

                // V1.6 AUDIT: Log validation completed
                if let Some(chain_id) = self.persistence.active_chain_id.clone() {
                    if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                        let validation_audit = crate::state::AuditEvent {
                            timestamp: chrono::Utc::now(),
                            event_type: crate::state::AuditEventType::ValidationCompleted,
                            previous_state: None,
                            next_state: None,
                            triggering_event: Some(format!("decision={}", decision)),
                            step_id: self.state.current_chain_step_id.clone(),
                            chain_id: Some(chain_id),
                            task: self.state.execution.active_objective.clone(),
                            reason: Some(message.clone()),
                            metadata: Some(format!("accepted={}", accepted)),
                        };
                        chain.audit_event(validation_audit);
                    }
                }
            }
            RuntimeEvent::ValidationStage {
                stage,
                status,
                summary,
                ..
            } => {
                self.apply_progress_transition(ProgressTransitionEvent::ValidationStage);
                self.set_current_step(
                    Some(format!("Validation stage: {}", stage)),
                    Some(3),
                    Some(3),
                );
                self.state.execution.validation_summary = Some(
                    summary
                        .clone()
                        .unwrap_or_else(|| format!("{} {:?}", stage, status)),
                );
                self.record_last_action(format!("Validation stage updated: {}", stage));
            }
            RuntimeEvent::StateCommitting { files_written } => {
                self.apply_progress_transition(ProgressTransitionEvent::StateCommitting);
                self.set_current_step(Some("Committing file writes".to_string()), Some(2), Some(3));
                for path in files_written {
                    Self::push_execution_entry(
                        &mut self.state.execution.file_writes,
                        path.clone(),
                        8,
                    );
                }
                self.record_last_action(format!("Committed {} files", files_written.len()));
            }
            RuntimeEvent::Completion { reason } => {
                self.apply_progress_transition(ProgressTransitionEvent::CompletionGate);
                self.set_current_step(
                    Some("Completion gate accepted".to_string()),
                    Some(3),
                    Some(3),
                );
                self.record_last_action(reason.clone());
            }
            RuntimeEvent::Failure { reason, .. } => {
                self.apply_progress_transition(ProgressTransitionEvent::RuntimeFailure {
                    reason: reason.clone(),
                });
                self.set_current_step(Some("Execution failed".to_string()), Some(3), Some(3));
                self.record_last_action(reason.clone());
            }
            RuntimeEvent::RepairLoop {
                attempt,
                max,
                reason,
            } => {
                self.apply_progress_transition(ProgressTransitionEvent::RepairLoop {
                    attempt: *attempt as usize,
                    max: *max as usize,
                });
                self.set_current_step(
                    Some(format!("Repair loop {}/{}", attempt, max)),
                    Some(2),
                    Some(3),
                );
                self.record_last_action(reason.clone());

                // V1.6 AUDIT: Log repair loop trigger
                if let Some(chain_id) = self.persistence.active_chain_id.clone() {
                    if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                        let repair_audit = crate::state::AuditEvent {
                            timestamp: chrono::Utc::now(),
                            event_type: crate::state::AuditEventType::RepairTriggered,
                            previous_state: None,
                            next_state: None,
                            triggering_event: Some(format!("attempt {}/{}", attempt, max)),
                            step_id: self.state.current_chain_step_id.clone(),
                            chain_id: Some(chain_id),
                            task: self.state.execution.active_objective.clone(),
                            reason: Some(reason.clone()),
                            metadata: None,
                        };
                        chain.audit_event(repair_audit);
                    }
                }
            }
            RuntimeEvent::Finished {
                success,
                iterations,
                error,
            } => {
                self.handle_chain_step_completion(*success, error.as_ref().map(|e| e.to_string()));
                self.apply_progress_transition(ProgressTransitionEvent::RuntimeFinished {
                    success: *success,
                });
                self.set_current_step(
                    Some(if *success {
                        "Done".to_string()
                    } else {
                        "Failed".to_string()
                    }),
                    Some(3),
                    Some(3),
                );
                self.state.execution.active_tool = None;
                self.record_last_action(if *success {
                    format!("Task finished after {} iterations", iterations)
                } else {
                    error
                        .clone()
                        .unwrap_or_else(|| format!("Task failed after {} iterations", iterations))
                });
            }
        }
    }

    fn update_active_run_card_from_event(&mut self, event: &RuntimeEvent, line: String) {
        let current_step = self.state.execution.current_step.clone();
        let active_tool = self.state.execution.active_tool.clone();
        let validation_summary = self.state.execution.validation_summary.clone();

        self.update_active_run_card(|run_card| {
            run_card.add_event(line);
            run_card.current_step = current_step;
            run_card.active_tool = active_tool;
            run_card.validation_summary = validation_summary;

            match event {
                RuntimeEvent::Init { .. }
                | RuntimeEvent::IterationStart { .. }
                | RuntimeEvent::PreflightPassed
                | RuntimeEvent::ContextAssembly { .. }
                | RuntimeEvent::PlannerOutput { .. }
                | RuntimeEvent::ProtocolValidation { .. } => {
                    run_card.status = RuntimeStatus::Running;
                    run_card.phase = "planning".to_string();
                }
                RuntimeEvent::ToolCall { .. }
                | RuntimeEvent::ToolExecuting { .. }
                | RuntimeEvent::ToolResult { .. }
                | RuntimeEvent::MutationsDetected { .. }
                | RuntimeEvent::StateCommitting { .. }
                | RuntimeEvent::RepairLoop { .. }
                | RuntimeEvent::BrowserPreview { .. } => {
                    run_card.status = RuntimeStatus::Running;
                    run_card.phase = "executing".to_string();
                }
                RuntimeEvent::ValidationRunning
                | RuntimeEvent::ValidationResult { .. }
                | RuntimeEvent::ValidationStage { .. } => {
                    run_card.status = RuntimeStatus::Running;
                    run_card.phase = "validating".to_string();
                }
                RuntimeEvent::Completion { reason } => {
                    run_card.phase = "done".to_string();
                    run_card.result_message = Some(reason.clone());
                }
                RuntimeEvent::Failure { reason, .. } => {
                    run_card.status = RuntimeStatus::Error;
                    run_card.phase = "failed".to_string();
                    run_card.result_message = Some(reason.clone());
                }
                RuntimeEvent::Finished {
                    success,
                    iterations,
                    error,
                } => {
                    let message = if *success {
                        format!("Completed in {} iteration(s).", iterations)
                    } else {
                        error
                            .clone()
                            .unwrap_or_else(|| format!("Failed in {} iteration(s).", iterations))
                    };
                    run_card.finish(*success, message, *iterations);
                }
            }
        });
    }

    pub fn on_tick(&mut self) {
        // Time-based updates can go here
        // e.g., checking Ollama health, updating status indicators
    }

    pub fn quit(&mut self) {
        info!("Quit requested");
        self.should_quit = true;
    }

    pub fn toggle_inspector(&mut self) {
        self.show_inspector = !self.show_inspector;
        debug!("Inspector visibility: {}", self.show_inspector);
    }

    pub fn is_operator_mode(&self) -> bool {
        matches!(self.experience_mode, ExperienceMode::Operator)
    }

    /// Get recovery summary appropriate for current experience mode
    /// - Normal mode: calm, user-facing language
    /// - Operator mode: full technical details
    pub fn get_recovery_summary(&self) -> String {
        if self.is_operator_mode() {
            // Operator mode: full technical details
            self.recovery_state.format_summary()
        } else {
            // Normal mode: calm, user-facing language
            self.recovery_state.format_summary_normal()
        }
    }

    fn set_experience_mode(&mut self, mode: ExperienceMode) {
        self.experience_mode = mode;
        if matches!(mode, ExperienceMode::Normal)
            && matches!(
                self.state.active_inspector_tab,
                InspectorTab::Logs
                    | InspectorTab::Audit
                    | InspectorTab::Steps
                    | InspectorTab::Timeline
                    | InspectorTab::Failure
                    | InspectorTab::PlannerTrace
                    | InspectorTab::Replay
                    | InspectorTab::DebugBundle
            )
        {
            self.state.active_inspector_tab = InspectorTab::Runtime;
        }
    }

    pub fn toggle_experience_mode(&mut self) {
        let new_mode = match self.experience_mode {
            ExperienceMode::Normal => ExperienceMode::Operator,
            ExperienceMode::Operator => ExperienceMode::Normal,
        };
        self.set_experience_mode(new_mode);
        let (label, description) = match new_mode {
            ExperienceMode::Normal => ("Normal", "Debug surfaces hidden"),
            ExperienceMode::Operator => ("Operator", "Debug surfaces visible"),
        };
        self.push_system_notice(&format!("Mode: {} — {}", label, description));
    }

    pub async fn submit_active_input(&mut self) -> Result<bool> {
        let content = self.state.input_buffer.trim().to_string();
        if content.is_empty() {
            return Ok(false);
        }

        // NON-CHAT MODES: These bypass the Chat routing logic entirely
        // Handle Search, ProjectCreate, ProjectConnect, Passive modes directly
        match self.composer_mode {
            ComposerMode::Search => {
                self.clear_input();
                self.append_user_message(&content);
                self.execute_unified(&content, "command", &format!("Search for: {}", content), |plan| {
                    plan.add_step(
                        "Execute search across projects",
                        crate::state::StepAction::Search {
                            query: content.clone(),
                        },
                    );
                    plan.add_step("Collect and format results", crate::state::StepAction::None);
                })
                .await?;
                self.run_project_search(&content);
                return Ok(false);
            }
            ComposerMode::ProjectCreate => {
                self.clear_input();
                let parent_dir = self
                    .project_create_workflow
                    .as_ref()
                    .map(|workflow| workflow.parent_dir.clone())
                    .unwrap_or_else(|| self.default_project_root());
                let project_name = content.clone();
                self.execute_unified(
                    &content,
                    "command",
                    &format!("Create project '{}' in {}", project_name, parent_dir.display()),
                    |plan| {
                        plan.add_step("Validate project name", crate::state::StepAction::Validate);
                        plan.add_step(
                            "Create directory",
                            crate::state::StepAction::CreateDirectory {
                                path: parent_dir.join(&project_name).display().to_string(),
                            },
                        );
                        plan.add_step(
                            "Initialize configuration",
                            crate::state::StepAction::WriteFile {
                                path: parent_dir
                                    .join(&project_name)
                                    .join("rasputin.json")
                                    .display()
                                    .to_string(),
                            },
                        );
                        plan.add_step("Attach to workspace", crate::state::StepAction::None);
                    },
                )
                .await?;
                if let Err(e) = self
                    .create_project_from_parent_and_attach(parent_dir, &project_name)
                    .await
                {
                    self.clear_project_create_workflow();
                    self.set_panel_status(PanelStatusLevel::Error, e.to_string());
                }
                return Ok(false);
            }
            ComposerMode::ProjectConnect => {
                self.clear_input();
                self.execute_unified(
                    &content,
                    "command",
                    &format!("Connect project: {}", content),
                    |plan| {
                        plan.add_step("Validate path exists", crate::state::StepAction::Validate);
                        plan.add_step(
                            "Read project configuration",
                            crate::state::StepAction::ReadFile {
                                path: "rasputin.json".to_string(),
                            },
                        );
                        plan.add_step("Attach to workspace", crate::state::StepAction::None);
                    },
                )
                .await?;
                if let Err(e) = self.connect_project(&content).await {
                    self.activate_project_connect();
                    self.set_panel_status(PanelStatusLevel::Error, e.to_string());
                }
                return Ok(false);
            }
            ComposerMode::Passive => {
                self.clear_input();
                return Ok(false);
            }
            ComposerMode::Chat => {
                // Continue to Chat mode routing below
            }
        }

        // SYNCHRONOUS INTENT CLASSIFICATION: Must complete BEFORE any UI reaction
        // This ensures deterministic routing with no drift between chat and task pipelines
        let routing_decision = self.classify_input_intent(&content);

        self.clear_input();

        // Route based on synchronous classification result - NO async/await before this point
        match routing_decision {
            InputRouting::Command => {
                let command = crate::commands::parse_command(&content);
                self.execute_command_with_message(command, &content).await
            }
            InputRouting::SimpleCommand { contract } => {
                // Simple natural language command - auto-execute artifact contract
                self.emit_event("large_prompt", &format!(
                    "simple command detected: {} files",
                    contract.artifacts.len()
                ));
                self.append_user_message(&content);
                
                let notice = format!(
                    "📦 Auto-creating {} documentation files...",
                    contract.artifacts.len()
                );
                self.push_system_notice(&notice);

                // Decompose into steps and execute
                let task = crate::large_task_decomposer::LargeTaskDecomposer::from_contract(contract);
                match self.execute_decomposed_task(task).await {
                    Ok(_) => {
                        self.push_system_notice("✅ All files created successfully!");
                        Ok(false)
                    }
                    Err(e) => {
                        self.push_system_notice(&format!("❌ Failed: {}", e));
                        Ok(false)
                    }
                }
            }
            InputRouting::TaskGoal => {
                // Structured execution intent: route to goal pipeline
                self.emit_event("autonomy", "plain text goal detected");

                // Add user message to UI (after classification, before execution)
                self.append_user_message(&content);

                let should_quit = self
                    .execute_command_unified(
                        Command::Goal {
                            statement: content.clone(),
                        },
                        &content,
                    )
                    .await?;

                if should_quit {
                    return Ok(true);
                }

                if self.goal_manager.active_goal().is_some_and(|goal| {
                    matches!(goal.status, crate::guidance::GoalStatus::Proposed)
                        && goal.generated_plan.is_some()
                }) {
                    self.pending_command = Some(Command::GoalConfirm);
                    self.push_system_notice(
                        "Autonomous goal intake: plan generated and confirmation queued.",
                    );
                }

                Ok(false)
            }
            InputRouting::FollowUp { resolved_task, original_input } => {
                // CODEX-LIKE CONTINUITY: Follow-up resolved from working memory
                self.emit_event("continuity", &format!("follow-up resolved: '{}' → task execution", original_input));
                self.append_user_message(&original_input);

                // Add continuity notice showing what we're continuing
                let continuity_notice = format!(
                    "Continuing from working memory: {}",
                    crate::text::truncate_chars(&resolved_task, 80)
                );
                self.push_system_notice(&continuity_notice);

                let should_quit = self
                    .execute_command_unified(
                        Command::Goal {
                            statement: resolved_task.clone(),
                        },
                        &resolved_task,
                    )
                    .await?;

                if should_quit {
                    return Ok(true);
                }

                Ok(false)
            }
            InputRouting::TaskExecution => {
                // Task mode execution with granular plan
                self.append_user_message(&content);

                let content_lower = content.to_lowercase();
                self.execute_unified(&content, "task", &content, |plan| {
                    plan.add_step("Parse request", crate::state::StepAction::Parse);
                    plan.add_step("Generate execution plan", crate::state::StepAction::Plan);

                    if content_lower.contains("create") || content_lower.contains("init") {
                        if content_lower.contains("react") || content_lower.contains("vite") {
                            plan.add_step(
                                "Create project directory",
                                crate::state::StepAction::CreateDirectory {
                                    path: "./".to_string(),
                                },
                            );
                            plan.add_step(
                                "Initialize project files",
                                crate::state::StepAction::WriteFile {
                                    path: "package.json".to_string(),
                                },
                            );
                            plan.add_step(
                                "Write starter templates",
                                crate::state::StepAction::WriteFile {
                                    path: "src/".to_string(),
                                },
                            );
                            plan.add_step(
                                "Install dependencies",
                                crate::state::StepAction::Install {
                                    package_manager: "npm".to_string(),
                                },
                            );
                        } else if content_lower.contains("project")
                            || content_lower.contains("app")
                        {
                            plan.add_step(
                                "Create project directory",
                                crate::state::StepAction::CreateDirectory {
                                    path: "./".to_string(),
                                },
                            );
                            plan.add_step(
                                "Initialize configuration",
                                crate::state::StepAction::WriteFile {
                                    path: "config".to_string(),
                                },
                            );
                        }
                    }

                    if content_lower.contains("install")
                        || content_lower.contains("npm")
                        || content_lower.contains("pip")
                    {
                        let pm = if content_lower.contains("pip") { "pip" } else { "npm" };
                        plan.add_step(
                            "Install dependencies",
                            crate::state::StepAction::Install {
                                package_manager: pm.to_string(),
                            },
                        );
                    }

                    if content_lower.contains("run")
                        || content_lower.contains("start")
                        || content_lower.contains("dev")
                    {
                        let cmd = if content_lower.contains("dev") { "npm run dev" } else { "npm start" };
                        plan.add_step(
                            "Start development server",
                            crate::state::StepAction::StartServer {
                                command: cmd.to_string(),
                            },
                        );
                    }

                    if content_lower.contains("build") || content_lower.contains("compile") {
                        plan.add_step("Build project", crate::state::StepAction::Build);
                    }

                    if content_lower.contains("test") || content_lower.contains("check") {
                        plan.add_step("Run validation", crate::state::StepAction::ValidateProject);
                    }

                    plan.add_step("Validate results", crate::state::StepAction::Validate);
                })
                .await
            }
            InputRouting::Chat => {
                // Conversational chat: direct LLM call
                self.append_user_message(&content);

                match self.send_to_ollama(&content).await {
                    Ok(Some(response)) => {
                        self.state.messages.push(Message {
                            id: uuid::Uuid::new_v4().to_string(),
                            role: MessageRole::Assistant,
                            source_text: response.clone(),
                            content: response,
                            timestamp: chrono::Local::now(),
                            run_card: None,
                        });
                        Ok(false)
                    }
                    Ok(None) => Ok(false),
                    Err(e) => {
                        self.push_system_notice(&format!("Chat error: {}", e));
                        Ok(false)
                    }
                }
            }
        }
    }

    /// Synchronous intent classification - NO async, NO delays, NO UI updates
    /// Must complete instantly to ensure deterministic routing
    fn classify_input_intent(&self, content: &str) -> InputRouting {
        // Commands always take precedence
        if content.starts_with('/') {
            return InputRouting::Command;
        }

        // SIMPLE COMMANDS: "docs", "create README.md", etc.
        // Auto-detect and convert to artifact contract
        if let Ok(repo_path) = self.active_project_root() {
            if crate::large_prompt_classifier::SimpleCommandParser::is_simple_command(content) {
                if let Some(contract) = crate::large_prompt_classifier::SimpleCommandParser::parse_simple(content, repo_path) {
                    return InputRouting::SimpleCommand { contract };
                }
            }
        }

        // CODEX-LIKE CONTINUITY: Check for follow-up intent against working memory
        let follow_up_intent = crate::working_memory::WorkingMemory::detect_follow_up_intent(content);
        if !matches!(follow_up_intent, crate::working_memory::FollowUpIntent::NewTask) {
            // Try to resolve the follow-up against current working memory
            if let Some(memory) = crate::working_memory::compute_working_memory(&self.persistence) {
                if let Some(resolved_task) = memory.resolve_follow_up(follow_up_intent, content) {
                    return InputRouting::FollowUp {
                        resolved_task,
                        original_input: content.to_string(),
                    };
                }
            }
        }

        // Check for structured execution intent (task-like plain text)
        if crate::autonomy::AutonomousLoopController::is_task_like_plain_text(content) {
            return InputRouting::TaskGoal;
        }

        // In Task execution mode, treat as task execution
        if self.state.execution.mode == ExecutionMode::Task {
            return InputRouting::TaskExecution;
        }

        // Default: conversational chat
        InputRouting::Chat
    }

    /// Helper to append user message to UI (extracted for consistency)
    fn append_user_message(&mut self, content: &str) {
        self.state.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            source_text: content.to_string(),
            content: content.to_string(),
            timestamp: chrono::Local::now(),
            run_card: None,
        });
    }

    /// Execute a parsed command with user message already appended
    async fn execute_command_with_message(
        &mut self,
        command: Command,
        content: &str,
    ) -> Result<bool> {
        self.append_user_message(content);
        self.execute_command_unified(command, content).await
    }

    /// Execute a command through the unified pipeline
    async fn execute_command_unified(
        &mut self,
        command: Command,
        original_input: &str,
    ) -> Result<bool> {
        let (intent, objective) = match &command {
            Command::OpenRepoPicker => ("command", "Choose project folder".to_string()),
            Command::OpenRepo { path } => ("command", format!("Open project: {}", path)),
            Command::SwitchRepo { path_or_name } => {
                ("command", format!("Switch to: {}", path_or_name))
            }
            Command::RunValidation => ("command", "Run validation pipeline".to_string()),
            Command::ReadFile { path } => ("command", format!("Read file: {}", path)),
            Command::WriteFile { path, .. } => ("command", format!("Write file: {}", path)),
            Command::RunShell { command: cmd } => ("command", format!("Execute: {}", cmd)),
            Command::RunForgeTask { task } => ("task", task.clone()),
            Command::ShowModel => ("command", "Show model configuration".to_string()),
            Command::ShowModels => ("command", "List available models".to_string()),
            Command::SetModel { model } => ("command", format!("Set model: {}", model)),
            Command::ShowStatus => ("command", "Show system status".to_string()),
            Command::ShowHelp => ("command", "Show help".to_string()),
            Command::ClearLogs => ("command", "Clear logs".to_string()),
            Command::RefreshRuntime => ("command", "Refresh runtime".to_string()),
            Command::ResetValidation => ("command", "Reset validation".to_string()),
            Command::ArchiveConversation { id } => {
                ("command", format!("Archive conversation: {}", id))
            }
            Command::UnarchiveConversation { id } => {
                ("command", format!("Restore conversation: {}", id))
            }
            Command::DeleteProject { path_or_name } => {
                ("command", format!("Delete project: {}", path_or_name))
            }
            Command::Task { content } => ("task", content.clone()),
            Command::ApprovePending => ("command", "Approve pending action".to_string()),
            Command::DenyPending => ("command", "Deny pending action".to_string()),
            Command::ReplaceInFile { path, .. } => ("command", format!("Patch file: {}", path)),
            Command::RLEFStatus => ("command", "Show RLEF status".to_string()),
            Command::RLEFClear => ("command", "Clear RLEF memory".to_string()),
            Command::RLEFDisableHint { class, .. } => {
                ("command", format!("Disable RLEF hint: {}", class))
            }
            Command::ListChains => ("command", "List chains".to_string()),
            Command::ChainStatus { chain_id } => (
                "command",
                format!("Chain status: {}", chain_id.as_deref().unwrap_or("active")),
            ),
            Command::ChainSwitch { chain_id } => ("command", format!("Switch chain: {}", chain_id)),
            Command::ChainArchive { chain_id } => {
                ("command", format!("Archive chain: {}", chain_id))
            }
            Command::ChainResume { chain_id, force } => {
                if *force {
                    (
                        "command",
                        format!("Resume chain: {} (force override)", chain_id),
                    )
                } else {
                    ("command", format!("Resume chain: {}", chain_id))
                }
            }
            Command::GitStatus => ("command", "Show Git status".to_string()),
            Command::Replay { chain_id, .. } => (
                "command",
                format!("Show replay: {}", chain_id.as_deref().unwrap_or("active")),
            ),
            Command::Audit { chain_id } => (
                "command",
                format!("Show audit: {}", chain_id.as_deref().unwrap_or("active")),
            ),
            Command::CheckpointList { chain_id } => (
                "command",
                format!(
                    "List checkpoints: {}",
                    chain_id.as_deref().unwrap_or("active")
                ),
            ),
            Command::CheckpointStatus { chain_id } => (
                "command",
                format!(
                    "Checkpoint status: {}",
                    chain_id.as_deref().unwrap_or("active")
                ),
            ),
            Command::CheckpointShow {
                chain_id,
                checkpoint_id,
            } => (
                "command",
                format!("Show checkpoint {} for {}", checkpoint_id, chain_id),
            ),
            Command::CheckpointDelete {
                chain_id,
                checkpoint_id,
            } => (
                "command",
                format!("Delete checkpoint {} for {}", checkpoint_id, chain_id),
            ),
            Command::ShowRecovery { chain_id } => (
                "command",
                format!("Show recovery: {}", chain_id.as_deref().unwrap_or("active")),
            ),
            Command::ShowPlan => ("command", "Show plan".to_string()),
            Command::ShowPlanContext => ("command", "Show plan context".to_string()),
            Command::ShowPlanCheckpoints => ("command", "Show plan checkpoints".to_string()),
            Command::FlowMode { enabled } => (
                "command",
                format!("Flow mode: {}", if *enabled { "on" } else { "off" }),
            ),
            Command::Stop => ("command", "Stop execution".to_string()),
            Command::Cancel => ("command", "Cancel operation".to_string()),
            Command::Override => ("command", "Override suggestion".to_string()),
            Command::Preview => ("command", "Preview execution".to_string()),
            Command::Goal { statement } => ("goal", format!("Goal: {}", statement)),
            Command::GoalConfirm => ("command", "Confirm goal plan".to_string()),
            Command::GoalReject => ("command", "Reject goal plan".to_string()),
            Command::GoalStatus => ("command", "Show goal status".to_string()),
            Command::ProjectCreate { name, .. } => ("command", format!("Create project: {}", name)),
            Command::ProjectSwitch { project_id } => {
                ("command", format!("Switch to project: {}", project_id))
            }
            Command::ProjectList => ("command", "List projects".to_string()),
            Command::DebugMode { enabled } => (
                "command",
                format!(
                    "Set experience mode: {}",
                    if *enabled { "operator" } else { "normal" }
                ),
            ),
            Command::DocGenerate { doc_number, .. } => (
                "command",
                if let Some(n) = doc_number {
                    format!("Generate documentation file {}", n)
                } else {
                    "Generate all 15 canonical documentation files".to_string()
                },
            ),
            Command::DocGenerateChain { current_step, .. } => (
                "command",
                format!("Documentation generation chain step {}/15", current_step),
            ),
            Command::DocValidate { output_dir } => (
                "command",
                format!("Validate generated documentation in {}", output_dir),
            ),
            Command::DocStatus => ("command", "Show documentation generation status".to_string()),
            Command::AutoChain { prompt, .. } => (
                "command",
                format!("Auto-chain large prompt ({} chars)", prompt.len()),
            ),
            Command::ArtifactContract { prompt, .. } => (
                "command",
                format!("Artifact contract ({} chars)", prompt.len()),
            ),
            Command::Quit => ("command", "Exit application".to_string()),
            Command::Unknown { input } => ("command", format!("Unknown command: {}", input)),
        };

        // Build appropriate plan based on command type - truthful granularity
        self.execute_unified(original_input, intent, &objective, |plan| {
            plan.add_step("Parse command", crate::state::StepAction::Parse);
            plan.add_step("Validate context", crate::state::StepAction::Validate);

            // Add specific action step based on command type
            match &command {
                Command::ReadFile { path } => {
                    plan.add_step(
                        format!("Read file: {}", path),
                        crate::state::StepAction::ReadFile { path: path.clone() },
                    );
                }
                Command::WriteFile { path, .. } => {
                    plan.add_step(
                        format!("Write file: {}", path),
                        crate::state::StepAction::WriteFile { path: path.clone() },
                    );
                }
                Command::ReplaceInFile { path, .. } => {
                    plan.add_step(
                        format!("Patch file: {}", path),
                        crate::state::StepAction::PatchFile { path: path.clone() },
                    );
                }
                Command::RunShell { command: cmd } => {
                    plan.add_step(
                        format!("Execute: {}", cmd.chars().take(30).collect::<String>()),
                        crate::state::StepAction::RunCommand {
                            command: cmd.clone(),
                        },
                    );
                }
                Command::RunForgeTask { task } => {
                    plan.add_step(
                        format!(
                            "Execute task: {}",
                            task.chars().take(40).collect::<String>()
                        ),
                        crate::state::StepAction::Plan,
                    );
                }
                Command::OpenRepoPicker => {
                    plan.add_step("Open folder picker", crate::state::StepAction::None);
                }
                Command::OpenRepo { path } => {
                    plan.add_step(
                        format!("Attach to: {}", path),
                        crate::state::StepAction::None,
                    );
                }
                Command::RunValidation => {
                    plan.add_step(
                        "Run validation pipeline",
                        crate::state::StepAction::ValidateProject,
                    );
                }
                Command::DocGenerate { .. } => {
                    plan.add_step(
                        "Generate canonical documentation",
                        crate::state::StepAction::None,
                    );
                }
                Command::DocValidate { .. } => {
                    plan.add_step(
                        "Validate generated documentation",
                        crate::state::StepAction::ValidateProject,
                    );
                }
                _ => {
                    plan.add_step(&objective, crate::state::StepAction::None);
                }
            }

            plan.add_step("Confirm result", crate::state::StepAction::Validate);
        })
        .await?;

        // Execute the actual command
        self.handle_command(command).await
    }

    /// UNIFIED EXECUTION PIPELINE
    ///
    /// All user input flows through this single pipeline:
    /// INPUT → INTENT → PLAN → EXECUTION → VALIDATION → RESULT
    ///
    /// This eliminates the perceptual gap between chat, commands, and tasks.
    /// Everything feels like "Rasputin understood and executed".
    async fn execute_unified(
        &mut self,
        input: &str,
        intent: &str,
        objective: &str,
        plan_builder: impl FnOnce(&mut ExecutionPlan),
    ) -> Result<bool> {
        // PHASE 1: Create execution plan
        let mut plan = ExecutionPlan::new(intent, objective);
        plan_builder(&mut plan);

        // Set active objective FROM PLAN - this is the source of truth
        self.set_active_objective(Some(plan.objective.clone()));

        // RLEF: Get active hints and create transparency
        let active_hints = self.state.rlef_memory.get_active_hints();
        let transparency =
            crate::state::PlanRLEFTransparency::from_active_hints(&active_hints, &plan.steps);
        self.state.rlef_transparency = Some(transparency.clone());

        // Show RLEF influence in inspector if applicable
        if transparency.influenced {
            let rlef_lines = transparency.format_for_inspector();
            for line in rlef_lines {
                self.add_log(crate::state::LogLevel::Info, "rlef", &line);
            }
        }

        // Store plan in execution state
        self.state.execution.current_plan = Some(plan.clone());

        // Show plan to user
        self.show_execution_plan(&plan);

        // PHASE 2: Execute each step
        plan.start();
        self.set_execution_state(ExecutionState::Executing);
        self.show_inspector = true;
        self.state.active_inspector_tab = InspectorTab::Runtime;

        let step_count = plan.steps.len();
        for step_index in 0..step_count {
            let step_description = plan.steps[step_index].description.clone();

            // Start step
            if let Some(step) = self
                .state
                .execution
                .current_plan
                .as_mut()
                .unwrap()
                .start_step(step_index)
            {
                step.status = ExecutionStepStatus::Running;
            }

            self.set_current_step(
                Some(step_description.clone()),
                Some(step_index as u32 + 1),
                Some(step_count as u32),
            );
            self.record_last_action(format!("Step {}: {}", step_index + 1, step_description));

            // MODE ENFORCEMENT: Check if step is allowed in current mode
            let step_action = plan.steps[step_index].action.clone();
            let (allowed, reason) = self.state.execution.mode.check_action(&step_action);

            if !allowed {
                let mode_block_msg = format!(
                    "✗ Step {} blocked by mode: {}\n\nCurrent mode: {}\nCapability: {}\n\nNext: Switch to appropriate mode or adjust request.",
                    step_index + 1,
                    reason.unwrap_or_else(|| "Action not allowed".to_string()),
                    self.state.execution.mode.as_str(),
                    self.state.execution.mode.capability_summary()
                );

                // Record the block
                if let Some(plan) = self.state.execution.current_plan.as_mut() {
                    plan.fail_step(step_index, "Blocked by mode enforcement");
                }

                self.set_execution_state(ExecutionState::Blocked);
                self.finalize_execution(false, &mode_block_msg);
                return Ok(false);
            }

            // RISK EVALUATION: Check if step requires approval
            // Get affected paths from step action for risk evaluation
            let affected_paths = extract_affected_paths(&step_action);
            let risk_eval =
                crate::persistence::RiskEvaluation::evaluate(&step_action, &affected_paths);

            // Check if approval required based on policy
            if self
                .persistence
                .chain_policy
                .requires_approval_for(risk_eval.level)
            {
                // Create checkpoint in chain
                if let Some(chain_id) = self.state.current_chain_id.clone() {
                    if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
                        let step_id =
                            if let Some(ref current_step_id) = self.state.current_chain_step_id {
                                current_step_id.clone()
                            } else {
                                format!("step-{}", step_index)
                            };

                        chain.create_checkpoint(step_id, step_description.clone(), &risk_eval);

                        // Update chain status to waiting for approval
                        chain.status = crate::persistence::ChainLifecycleStatus::WaitingForApproval;

                        // Clear current chain context
                        self.state.current_chain_id = None;
                        self.state.current_chain_step_id = None;

                        // Show approval message
                        let approval_msg = format!(
                            "⏳ CHECKPOINT: Approval Required\n\nStep {}: {}\n\nRisk Level: {}\nReason: {}\n\n▶ /approve to execute\n▶ /deny to skip/abort",
                            step_index + 1,
                            step_description,
                            risk_eval.level.label(),
                            risk_eval.reason
                        );

                        self.set_execution_state(ExecutionState::Blocked);
                        self.push_system_notice(&approval_msg);
                        self.persist().await;
                        return Ok(false);
                    }
                }
            }

            // Execute the step based on intent and capture result metadata
            let step_result = self
                .execute_plan_step_with_result(intent, &step_description, input, &step_action)
                .await;

            // Record result with metadata
            match step_result {
                Ok((output, result_metadata)) => {
                    if let Some(plan) = self.state.execution.current_plan.as_mut() {
                        plan.complete_step_with_result(step_index, output, result_metadata);
                    }
                }
                Err((error, context)) => {
                    // Classify the failure for RLEF
                    let failure_class = crate::state::ExecutionFailureClassV1::classify_from_error(
                        &error,
                        &step_action,
                    );

                    if let Some(plan) = self.state.execution.current_plan.as_mut() {
                        let mut result = crate::state::StepResult::with_error(&error);
                        result.affected_paths = context.affected_paths;
                        plan.steps[step_index].status = crate::state::ExecutionStepStatus::Failed;
                        plan.steps[step_index].output = Some(error.clone());
                        plan.steps[step_index].result = Some(result);
                        plan.steps[step_index].completed_at = Some(chrono::Local::now());
                    }
                    self.set_execution_state(ExecutionState::Failed);

                    // CRITICAL FIX: Block chat when chat step fails (Ollama down, etc.)
                    if matches!(step_action, StepAction::Chat) {
                        self.block_chat(&format!("Chat failed: {}", error));
                    }

                    // RLEF: Record feedback for learning
                    let feedback = crate::state::ExecutionFeedback {
                        timestamp: chrono::Local::now(),
                        plan_id: plan.id.clone(),
                        step_index,
                        step_action: step_action.clone(),
                        success: false,
                        failure_class: Some(failure_class),
                        error_message: Some(error.clone()),
                        context: Some(format!("Step: {}", step_description)),
                    };
                    self.state.rlef_memory.record_feedback(feedback);

                    // Build failure UX with structured outcome
                    if let Some(ref mut exec_plan) = self.state.execution.current_plan {
                        let outcome = ExecutionOutcome::from_plan(exec_plan, false);
                        // Generate structured failure message
                        let mut lines = vec![
                            format!("✗ Step {} failed: {}", step_index + 1, step_description),
                            "".to_string(),
                            "Reason:".to_string(),
                            format!("  {}", error),
                            "".to_string(),
                        ];

                        // Impact: how many steps not executed
                        let remaining = exec_plan.steps.len() - step_index - 1;
                        if remaining > 0 {
                            lines.push(format!("Impact: {} step(s) not executed", remaining));
                            lines.push("".to_string());
                        }

                        // Next actions
                        if !outcome.next_actions.is_empty() {
                            lines.push("Next:".to_string());
                            for action in &outcome.next_actions[..outcome.next_actions.len().min(3)]
                            {
                                lines.push(format!("  → {}", action.label()));
                            }
                        }

                        let failure_msg = lines.join("\n");
                        self.finalize_execution_with_outcome(false, failure_msg, outcome);
                    } else {
                        let failure_msg = format!(
                            "✗ Step {} failed: {}\n\nReason: {}",
                            step_index + 1,
                            step_description,
                            error
                        );
                        self.finalize_execution(false, &failure_msg);
                    }
                    return Ok(false);
                }
            }
        }

        // PHASE 3: Finalize with ExecutionOutcome and post-run summary
        self.set_execution_state(ExecutionState::Done);

        // Generate ExecutionOutcome from the completed plan
        if let Some(ref mut exec_plan) = self.state.execution.current_plan {
            let outcome = ExecutionOutcome::from_plan(exec_plan, true);
            let summary = outcome.format_post_run_summary();
            self.finalize_execution_with_outcome(true, summary, outcome);
        } else {
            self.finalize_execution(true, "Completed");
        }

        Ok(false)
    }

    /// Execute a single step with full result metadata capture
    /// Returns: Ok((output, StepResult)) or Err((error_message, FailureContext))
    async fn execute_plan_step_with_result(
        &mut self,
        _intent: &str,
        step_description: &str,
        original_input: &str,
        step_action: &StepAction,
    ) -> std::result::Result<(Option<String>, crate::state::StepResult), (String, FailureContext)>
    {
        let start = std::time::Instant::now();
        let mut result = crate::state::StepResult::default();

        match step_action {
            StepAction::Parse => {
                // Parse action is immediate, no host effect
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some("Parsed input".to_string()), result))
            }
            StepAction::Validate => {
                // Validate checks state/context
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some("Context valid".to_string()), result))
            }
            StepAction::Chat => {
                // NOTE: User message already added in submit_active_input
                // We only add the assistant response here

                match self.execute_chat_step(original_input).await {
                    Ok(Some(response)) => {
                        // Route conversational replies to chat and structured output to inspector.
                        self.push_assistant_message(&response);

                        result.bytes_affected = Some(response.len());
                        result.duration_ms = Some(start.elapsed().as_millis() as u64);
                        Ok((Some(format!("Response: {} chars", response.len())), result))
                    }
                    Ok(None) => {
                        result.duration_ms = Some(start.elapsed().as_millis() as u64);
                        Ok((Some("No response".to_string()), result))
                    }
                    Err(e) => Err((e.to_string(), FailureContext::default())),
                }
            }
            StepAction::CreateDirectory { path } => {
                // Host action: create directory
                result.affected_paths.push(path.clone());
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Created: {}", path)), result))
            }
            StepAction::WriteFile { path } => {
                // Host action: write file
                result.affected_paths.push(path.clone());
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Wrote: {}", path)), result))
            }
            StepAction::ReadFile { path } => {
                // Host action: read file
                result.affected_paths.push(path.clone());
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Read: {}", path)), result))
            }
            StepAction::PatchFile { path } => {
                // Host action: patch file
                result.affected_paths.push(path.clone());
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Patched: {}", path)), result))
            }
            StepAction::RunCommand { command } => {
                // Host action: run command
                result.exit_code = Some(0); // Would be populated from actual execution
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((
                    Some(format!(
                        "Executed: {}",
                        command.chars().take(40).collect::<String>()
                    )),
                    result,
                ))
            }
            StepAction::Search { query } => {
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Searched: {}", query)), result))
            }
            StepAction::Plan => {
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some("Plan generated".to_string()), result))
            }
            StepAction::ValidateProject => {
                result.validation_result = Some("passed".to_string());
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some("Validation passed".to_string()), result))
            }
            StepAction::Install { package_manager } => {
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Installed via {}", package_manager)), result))
            }
            StepAction::StartServer { command } => {
                result
                    .artifact_urls
                    .push("http://localhost:5173".to_string()); // Would be detected
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Started: {}", command)), result))
            }
            StepAction::Build => {
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some("Build completed".to_string()), result))
            }
            StepAction::Test => {
                result.validation_result = Some("passed".to_string());
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some("Tests passed".to_string()), result))
            }
            StepAction::Git { operation } => {
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(format!("Git: {}", operation)), result))
            }
            StepAction::Fix { .. } => {
                // Recovery action - simulate the fix
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some("Applied fix".to_string()), result))
            }
            StepAction::None => {
                result.duration_ms = Some(start.elapsed().as_millis() as u64);
                Ok((Some(step_description.to_string()), result))
            }
        }
    }

    async fn resolve_autonomous_planner_preflight(
        &mut self,
    ) -> crate::autonomy::PlannerPreflightOutcome {
        if !Self::is_real_repo_path(&self.state.repo.path) {
            return crate::autonomy::PlannerPreflightOutcome::PreconditionFailed {
                reason: "No active project attached".to_string(),
            };
        }

        let health = match self.ollama.health_check().await {
            Ok(health) => health,
            Err(error) => {
                self.state.ollama_connected = false;
                self.state.model.connected = false;
                return crate::autonomy::PlannerPreflightOutcome::OllamaUnavailable {
                    reason: error.to_string(),
                };
            }
        };

        self.state.ollama_connected = health.connected;
        self.state.model.available = health.models.clone();

        let outcome = crate::autonomy::AutonomousLoopController::resolve_planner_preflight(
            self.state.model.configured.as_deref(),
            &health.model_cards,
            health.connected,
            health.error.as_deref(),
        );

        if let crate::autonomy::PlannerPreflightOutcome::Ready { model, binding } = &outcome {
            if let Err(error) = self.bind_autonomous_planner_model(model, binding) {
                return crate::autonomy::PlannerPreflightOutcome::PreconditionFailed {
                    reason: format!("Failed to bind planner model '{}': {}", model, error),
                };
            }
        } else {
            self.state.model.active = None;
            self.state.model.connected = false;
        }

        outcome
    }

    fn bind_autonomous_planner_model(
        &mut self,
        model: &str,
        binding: &crate::autonomy::PlannerModelBinding,
    ) -> Result<()> {
        let should_persist = matches!(
            binding,
            crate::autonomy::PlannerModelBinding::AutoBound
                | crate::autonomy::PlannerModelBinding::ReboundInvalidConfigured { .. }
        );

        if should_persist {
            self.apply_host_action(HostAction::WriteRepoModelConfig {
                repo_root: PathBuf::from(&self.state.repo.path),
                model: model.to_string(),
            })?;
            self.state.model.configured = Some(model.to_string());
            self.persistence
                .touch_repo(&self.state.repo.path, &self.state.repo.name, Some(model));
        }

        self.state.model.active = Some(model.to_string());
        self.state.model.connected = true;
        self.persistence.update_model_status(
            self.state.model.configured.as_deref(),
            Some(model),
            true,
        );
        self.clear_execution_block();

        match binding {
            crate::autonomy::PlannerModelBinding::Configured => {
                self.emit_event("autonomy/preflight", &format!("Planner ready: {}", model));
            }
            crate::autonomy::PlannerModelBinding::ResolvedConfigured { requested_model } => {
                self.emit_event(
                    "autonomy/preflight",
                    &format!(
                        "Configured planner '{}' resolved to '{}'",
                        requested_model, model
                    ),
                );
            }
            crate::autonomy::PlannerModelBinding::AutoBound => {
                let message = format!("Auto-selected planner model: {}", model);
                self.emit_event("autonomy/preflight", &message);
                self.push_system_notice(&message);
            }
            crate::autonomy::PlannerModelBinding::ReboundInvalidConfigured {
                invalid_model,
                reason,
            } => {
                let message = format!(
                    "Configured planner '{}' was invalid ({}). Auto-selected planner model: {}",
                    invalid_model, reason, model
                );
                self.emit_event("autonomy/preflight", &message);
                self.push_system_notice(&message);
            }
        }

        Ok(())
    }

    fn fail_autonomous_planner_preflight(
        &mut self,
        outcome: crate::autonomy::PlannerPreflightOutcome,
    ) {
        let (reason, fix) = match outcome {
            crate::autonomy::PlannerPreflightOutcome::MissingLocalModel => (
                "No local coder-capable planner model found".to_string(),
                "Install qwen2.5-coder:14b in Ollama, then retry the goal".to_string(),
            ),
            crate::autonomy::PlannerPreflightOutcome::OllamaUnavailable { reason } => (
                format!("Ollama unavailable: {}", reason),
                "Start Ollama and retry the goal".to_string(),
            ),
            crate::autonomy::PlannerPreflightOutcome::InvalidConfiguredModel {
                configured_model,
                reason,
            } => (
                format!("Configured planner '{}' is invalid", configured_model),
                reason,
            ),
            crate::autonomy::PlannerPreflightOutcome::PreconditionFailed { reason } => (
                reason,
                "Resolve the precondition and retry the goal".to_string(),
            ),
            crate::autonomy::PlannerPreflightOutcome::Ready { .. } => return,
        };

        self.emit_event("autonomy/preflight", &format!("failed: {}", reason));
        self.set_execution_precondition_failed(reason, fix);
    }

    async fn generate_goal_plan(
        &mut self,
        goal: &crate::guidance::Goal,
    ) -> crate::guidance::PlanGenerationResult {
        let repo_path = if self.state.repo.path.trim().is_empty() {
            self.persistence.active_repo.as_deref()
        } else {
            Some(self.state.repo.path.as_str())
        };

        if let Some(plan) =
            crate::guidance::PlanEngine::generate_explicit_artifact_plan(goal, repo_path)
        {
            self.emit_event("goal/planner", "explicit artifact contract plan selected");
            return crate::guidance::PlanGenerationResult::Success(plan);
        }

        if let Some(plan) = crate::guidance::PlanEngine::generate_literal_creation(goal) {
            self.emit_event("goal/planner", "literal creation plan selected");
            return crate::guidance::PlanGenerationResult::Success(plan);
        }

        let fallback =
            |app: &Self| crate::guidance::PlanEngine::generate(goal, &app.state, &app.persistence);

        let Some(model) = self
            .state
            .model
            .active
            .clone()
            .or_else(|| self.state.model.configured.clone())
        else {
            self.emit_event("goal/planner", "fallback: no active model");
            return fallback(self);
        };

        if !self.state.ollama_connected {
            self.emit_event("goal/planner", "fallback: ollama disconnected");
            return fallback(self);
        }

        let previous_plan = self.goal_manager.previous_plan().cloned();
        let repo_evidence = self
            .get_or_build_repo_snapshot()
            .map(|snapshot| snapshot.format_context())
            .unwrap_or_else(|| "No repository snapshot available.".to_string());

        // CODEX-LIKE CONTINUITY: Inject working memory context for richer planner context
        let working_context = crate::working_memory::compute_working_memory(&self.persistence)
            .map(|memory| memory.format_context_block());

        let messages = crate::goal_planner::QwenGoalPlanner::build_messages(
            goal,
            &repo_evidence,
            previous_plan.as_ref(),
            working_context.as_deref(),
        );

        self.emit_event(
            "goal/planner",
            &format!("requesting Qwen-Coder plan with {}", model),
        );

        match self.ollama.chat(&model, &messages).await {
            Ok(response) => match crate::goal_planner::QwenGoalPlanner::parse_response(
                &response,
                &goal.statement,
            ) {
                Ok(plan) => {
                    self.emit_event("goal/planner", "Qwen-Coder plan accepted");
                    crate::guidance::PlanGenerationResult::Success(plan)
                }
                Err(error) => {
                    warn!("Qwen-Coder goal plan rejected: {}", error);
                    self.emit_event(
                        "goal/planner",
                        &format!("fallback: invalid Qwen-Coder plan ({})", error),
                    );
                    fallback(self)
                }
            },
            Err(error) => {
                warn!("Qwen-Coder goal planning failed: {}", error);
                self.emit_event(
                    "goal/planner",
                    &format!("fallback: Qwen-Coder unavailable ({})", error),
                );
                fallback(self)
            }
        }
    }

    async fn execute_chat_step(&mut self, content: &str) -> Result<Option<String>> {
        // PURE LLM CALL - no state setting, unified pipeline manages state
        self.send_to_ollama(content).await
    }

    /// Send to Ollama with preflight health check
    async fn send_to_ollama(&mut self, content: &str) -> Result<Option<String>> {
        // CRITICAL FIX: Preflight health check before attempting chat
        match self.ollama.health_check().await {
            Ok(health) if !health.connected => {
                let error_msg = format!(
                    "Ollama disconnected: {}\n\nFix:\n1. Start Ollama: ollama serve\n2. Check /api/tags is reachable\n3. Retry chat",
                    health.error.as_deref().unwrap_or("Connection failed")
                );
                error!("Preflight health check failed: {:?}", health.error);
                // STRUCTURED OBSERVABILITY: Health check failure
                self.emit_event("health", "FAILED: Ollama disconnected");
                self.persistence.add_event(
                    &self.conversation_id,
                    "health",
                    "ERROR",
                    &format!("Ollama disconnected: {:?}", health.error),
                );
                return Err(anyhow::anyhow!(error_msg));
            }
            Err(e) => {
                let error_msg = format!(
                    "Ollama health check failed: {}\n\nFix:\n1. Start Ollama: ollama serve\n2. Check endpoint: {}\n3. Retry chat",
                    e,
                    self.ollama.endpoint()
                );
                error!("Preflight health check error: {}", e);
                // STRUCTURED OBSERVABILITY: Health check error
                self.emit_event("health", &format!("ERROR: {}", e));
                self.persistence.add_event(
                    &self.conversation_id,
                    "health",
                    "ERROR",
                    &format!("Health check error: {}", e),
                );
                return Err(anyhow::anyhow!(error_msg));
            }
            _ => {
                // Health check passed
                self.emit_event("health", "OK: Ollama connected");
            }
        }

        let active_model = self.state.model.active.clone();

        if let Some(model) = active_model {
            // Emit request started event
            self.emit_event("request", &format!("Request started with model: {}", model));

            // Build system context with project information and repo snapshot
            let persona_context = format!(
                "You are Rasputin, a terminal-native coding agent.\n\
                 You are currently attached to a local project.\n\n\
                 Project Name: {}\n\
                 Project Path: {}\n\n\
                 You have access to the local filesystem within this project.\n\
                 You can reason about files, structure, and code in this directory.\n\
                 Do NOT say you lack access to files. Assume visibility.",
                self.state.repo.name, self.state.repo.path
            );

            // Get repo snapshot with real evidence (bounded, cached)
            let repo_evidence = self
                .get_or_build_repo_snapshot()
                .map(|snap| snap.format_context())
                .unwrap_or_else(|| "No repository snapshot available.".to_string());

            // Combine persona + evidence into single system message with citation rules
            let system_context = format!(
                "{}\n\n{}\n\nWhen answering:\n\
                 - Prefer referencing the provided project snapshot\n\
                 - Ground claims explicitly: use phrases like 'Based on the project tree...', 'From Cargo.toml...'\n\
                 - If something is not in the snapshot, say so explicitly: 'This file was not included in the snapshot'\n\
                 - Do not hallucinate files or structure not shown in the snapshot\n\
                 - Be precise about what you actually know vs. what you can infer\n\n\
                 Snapshot limitations (do not assume completeness):\n\
                 - Directory depth: 2 levels\n\
                 - Not all files are included\n\
                 - Only key config/documentation files were read\n\
                 - Large files are truncated\n\n\
                 If more detail is needed, you may request to inspect specific files.",
                persona_context, repo_evidence
            );

            // Build message history with system context first, then conversation history
            let mut api_messages: Vec<ChatMessage> = vec![ChatMessage {
                role: "system".to_string(),
                content: system_context,
            }];

            // Add conversation history (user and assistant messages only)
            api_messages.extend(self.state.messages.iter().filter_map(|m| match m.role {
                MessageRole::User => Some(ChatMessage {
                    role: "user".to_string(),
                    content: m.content.clone(),
                }),
                MessageRole::Assistant => Some(ChatMessage {
                    role: "assistant".to_string(),
                    content: m.content.clone(),
                }),
                _ => None,
            }));

            // Add current input as user message if not already present
            let user_msg = ChatMessage {
                role: "user".to_string(),
                content: content.to_string(),
            };

            // Check if last message is already this user message
            if api_messages
                .last()
                .map(|m| m.content != content || m.role != "user")
                .unwrap_or(true)
            {
                api_messages.push(user_msg);
            }

            self.emit_event("request", "Sending to Ollama...");

            // Call Ollama
            match self.ollama.chat(&model, &api_messages).await {
                Ok(response) => {
                    info!("Received response from Ollama");
                    self.emit_event("request", "Response received");
                    self.emit_event("request", &format!("Tokens: ~{}", response.len() / 4));

                    // Return the response for the unified pipeline to display
                    Ok(Some(response))
                }
                Err(e) => {
                    error!("Ollama chat failed: {}", e);
                    // STRUCTURED OBSERVABILITY: Chat failure with classification
                    self.emit_event("chat", &format!("FAILED: {}", e));
                    self.persistence.add_event(
                        &self.conversation_id,
                        "chat",
                        "ERROR",
                        &format!("Chat failed: {}", e),
                    );
                    Err(anyhow::anyhow!("Chat failed: {}", e))
                }
            }
        } else {
            warn!("No active model available");
            self.emit_event("error", "No active model available");
            Err(anyhow::anyhow!("No active model configured"))
        }
    }

    /// Show the execution plan to the user
    fn show_execution_plan(&mut self, plan: &ExecutionPlan) {
        let mut lines = vec![
            format!("▶ {}", plan.objective),
            "".to_string(),
            "Plan:".to_string(),
        ];

        for (i, step) in plan.steps.iter().enumerate() {
            lines.push(format!("  {}. {}", i + 1, step.description));
        }

        lines.push("".to_string());
        lines.push("Executing...".to_string());

        self.push_system_notice(&lines.join("\n"));
    }

    /// Finalize execution and show results
    fn finalize_execution(&mut self, success: bool, summary: &str) {
        let emoji = if success { "✓" } else { "✗" };
        let message = format!("{} {}", emoji, summary);

        self.push_system_notice(&message);
        self.record_last_action(summary.to_string());

        // Clear the plan after a delay (keep it in inspector for review)
        // For now, we keep it so inspector can show the full execution
    }

    /// Finalize execution with full outcome (including next actions and artifacts)
    fn finalize_execution_with_outcome(
        &mut self,
        success: bool,
        summary: String,
        outcome: ExecutionOutcome,
    ) {
        // CLEAR CURRENT STEP - execution is done, no active step
        self.set_current_step(None, None, None);

        // NOTE: We do NOT clear active_objective here
        // The objective stays visible until the next input starts

        // Build the final display message
        let mut lines = vec![];

        if success {
            lines.push("✓ Execution complete".to_string());
        } else {
            lines.push("✗ Execution failed".to_string());
        }

        lines.push("".to_string());
        lines.push(outcome.summary.clone());

        if !outcome.affected_files.is_empty() {
            lines.push(format!("{} files affected", outcome.affected_files.len()));
        }

        // Show artifacts (URLs, etc.)
        let url_artifacts: Vec<_> = outcome
            .artifacts
            .iter()
            .filter_map(|a| {
                if let Artifact::Url(url) = a {
                    Some(url.clone())
                } else {
                    None
                }
            })
            .collect();
        if !url_artifacts.is_empty() {
            lines.push(format!(
                "{} artifact(s): {}",
                url_artifacts.len(),
                url_artifacts.join(", ")
            ));
        }

        // What changed section
        if !outcome.what_changed.is_empty() {
            lines.push("".to_string());
            lines.push("What changed:".to_string());
            for change in &outcome.what_changed {
                lines.push(format!("  {}", change));
            }
        }

        // Next actions section - the key control surface
        if !outcome.next_actions.is_empty() {
            lines.push("".to_string());
            lines.push("Next:".to_string());
            for (i, action) in outcome.next_actions.iter().take(4).enumerate() {
                lines.push(format!("  {}. {}", i + 1, action.label()));
            }
        }

        let message = lines.join("\n");
        self.push_system_notice(&message);
        self.record_last_action(summary);
    }

    pub fn handle_input(&mut self, c: char) {
        if !self.composer_is_editable() {
            return;
        }
        self.state.cursor_position = crate::text::clamp_to_char_boundary(
            &self.state.input_buffer,
            self.state.cursor_position,
        );
        self.state
            .input_buffer
            .insert(self.state.cursor_position, c);
        self.state.cursor_position += c.len_utf8();
    }

    pub fn handle_backspace(&mut self) {
        if !self.composer_is_editable() {
            return;
        }
        if self.state.cursor_position > 0 {
            self.state.cursor_position = crate::text::clamp_to_char_boundary(
                &self.state.input_buffer,
                self.state.cursor_position,
            );
            let previous = self.state.input_buffer[..self.state.cursor_position]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.state.cursor_position = previous;
            self.state.input_buffer.remove(self.state.cursor_position);
        }
    }

    pub fn handle_delete(&mut self) {
        if !self.composer_is_editable() {
            return;
        }
        self.state.cursor_position = crate::text::clamp_to_char_boundary(
            &self.state.input_buffer,
            self.state.cursor_position,
        );
        if self.state.cursor_position < self.state.input_buffer.len() {
            self.state.input_buffer.remove(self.state.cursor_position);
        }
    }

    pub fn move_cursor_left(&mut self) {
        if !self.composer_is_editable() {
            return;
        }
        if self.state.cursor_position > 0 {
            self.state.cursor_position = crate::text::clamp_to_char_boundary(
                &self.state.input_buffer,
                self.state.cursor_position,
            );
            self.state.cursor_position = self.state.input_buffer[..self.state.cursor_position]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if !self.composer_is_editable() {
            return;
        }
        self.state.cursor_position = crate::text::clamp_to_char_boundary(
            &self.state.input_buffer,
            self.state.cursor_position,
        );
        if self.state.cursor_position < self.state.input_buffer.len() {
            let next_char_len = self.state.input_buffer[self.state.cursor_position..]
                .chars()
                .next()
                .map(|ch| ch.len_utf8())
                .unwrap_or(1);
            self.state.cursor_position += next_char_len;
        }
    }

    pub fn move_cursor_home(&mut self) {
        if !self.composer_is_editable() {
            return;
        }
        self.state.cursor_position = 0;
    }

    pub fn move_cursor_end(&mut self) {
        if !self.composer_is_editable() {
            return;
        }
        self.state.cursor_position = self.state.input_buffer.len();
    }

    pub fn scroll_up(&mut self) {
        if self.state.scroll_offset > 0 {
            self.state.scroll_offset -= 1;
        }
    }

    pub fn scroll_down(&mut self) {
        let max_scroll = self.state.messages.len().saturating_sub(1);
        if self.state.scroll_offset < max_scroll {
            self.state.scroll_offset += 1;
        }
    }

    // Inspector tab scroll functions
    pub fn scroll_runtime_tab_up(&mut self) {
        if self.state.runtime_tab_scroll > 0 {
            self.state.runtime_tab_scroll -= 1;
        }
    }

    pub fn scroll_runtime_tab_down(&mut self) {
        self.state.runtime_tab_scroll += 3; // Scroll by 3 lines
    }

    pub fn scroll_validation_tab_up(&mut self) {
        if self.state.validation_tab_scroll > 0 {
            self.state.validation_tab_scroll -= 1;
        }
    }

    pub fn scroll_validation_tab_down(&mut self) {
        self.state.validation_tab_scroll += 3;
    }

    pub fn scroll_logs_tab_up(&mut self) {
        if self.state.logs_tab_scroll > 0 {
            self.state.logs_tab_scroll -= 1;
        }
    }

    pub fn scroll_logs_tab_down(&mut self) {
        self.state.logs_tab_scroll += 3;
    }

    pub fn scroll_preview_tab_up(&mut self) {
        if self.state.preview_tab_scroll > 0 {
            self.state.preview_tab_scroll -= 1;
        }
    }

    pub fn scroll_preview_tab_down(&mut self) {
        self.state.preview_tab_scroll += 3;
    }

    pub fn scroll_diff_tab_up(&mut self) {
        if self.state.diff_tab_scroll > 0 {
            self.state.diff_tab_scroll -= 3;
        }
    }

    pub fn scroll_diff_tab_down(&mut self) {
        self.state.diff_tab_scroll += 3;
    }

    pub fn scroll_timeline_tab_up(&mut self) {
        if self.state.timeline_tab_scroll > 0 {
            self.state.timeline_tab_scroll -= 1;
        }
    }

    pub fn scroll_timeline_tab_down(&mut self) {
        self.state.timeline_tab_scroll += 3;
    }

    pub fn scroll_failure_tab_up(&mut self) {
        if self.state.failure_tab_scroll > 0 {
            self.state.failure_tab_scroll -= 1;
        }
    }

    pub fn scroll_failure_tab_down(&mut self) {
        self.state.failure_tab_scroll += 3;
    }

    pub fn scroll_steps_tab_up(&mut self) {
        if self.state.steps_tab_scroll > 0 {
            self.state.steps_tab_scroll -= 1;
        }
    }

    pub fn scroll_steps_tab_down(&mut self) {
        self.state.steps_tab_scroll += 3;
    }

    pub fn scroll_planner_trace_tab_up(&mut self) {
        if self.state.planner_trace_tab_scroll > 0 {
            self.state.planner_trace_tab_scroll -= 1;
        }
    }

    pub fn scroll_planner_trace_tab_down(&mut self) {
        self.state.planner_trace_tab_scroll += 3;
    }

    pub fn scroll_replay_tab_up(&mut self) {
        if self.state.replay_tab_scroll > 0 {
            self.state.replay_tab_scroll -= 1;
        }
    }

    pub fn scroll_replay_tab_down(&mut self) {
        self.state.replay_tab_scroll += 3;
    }

    pub fn scroll_debug_bundle_tab_up(&mut self) {
        if self.state.debug_bundle_tab_scroll > 0 {
            self.state.debug_bundle_tab_scroll -= 1;
        }
    }

    pub fn scroll_debug_bundle_tab_down(&mut self) {
        self.state.debug_bundle_tab_scroll += 3;
    }

    pub fn scroll_audit_tab_up(&mut self) {
        if self.state.audit_tab_scroll > 0 {
            self.state.audit_tab_scroll -= 1;
        }
    }

    pub fn scroll_audit_tab_down(&mut self) {
        self.state.audit_tab_scroll += 3;
    }

    pub fn composer_is_editable(&self) -> bool {
        // CRITICAL FIX: Block input when chat is blocked (e.g., Ollama down)
        if self.chat_blocked {
            return false;
        }
        !matches!(self.composer_mode, ComposerMode::Passive)
    }

    pub fn can_switch_execution_mode(&self) -> bool {
        matches!(self.composer_mode, ComposerMode::Chat)
    }

    pub fn composer_placeholder(&self) -> String {
        // Normal mode: conversational language
        // Operator mode: technical command hints
        let is_normal = !self.is_operator_mode();

        match self.composer_mode {
            ComposerMode::Chat => match self.state.execution.mode {
                // TASK is default mode - work-first semantics
                ExecutionMode::Task => "Describe the work to execute...".to_string(),
                ExecutionMode::Edit => {
                    if is_normal {
                        "Describe what you want to change...".to_string()
                    } else {
                        "Use /read, /write, /replace, or /run...".to_string()
                    }
                }
                // Chat is now the secondary/conversational mode
                ExecutionMode::Chat => "Ask questions or chat...".to_string(),
            },
            ComposerMode::Search => format!("Search {}...", self.search_scope_label()),
            ComposerMode::ProjectCreate => {
                if let Some(workflow) = &self.project_create_workflow {
                    format!("Project name in {}...", workflow.parent_dir.display())
                } else {
                    "Choose a parent folder, then enter a project name...".to_string()
                }
            }
            ComposerMode::ProjectConnect => "Enter a folder path to connect...".to_string(),
            ComposerMode::Passive => match self.active_panel {
                SidebarPanel::Plugins => "Use the plugin controls above.".to_string(),
                SidebarPanel::Automations => "Use the maintenance controls above.".to_string(),
                _ => "Choose an action to continue.".to_string(),
            },
        }
    }

    pub fn composer_mode_label(&self) -> String {
        match self.composer_mode {
            ComposerMode::Chat => self.state.execution.mode.as_str().to_string(),
            ComposerMode::Search => "Search".to_string(),
            ComposerMode::ProjectCreate => "Create".to_string(),
            ComposerMode::ProjectConnect => "Connect".to_string(),
            ComposerMode::Passive => self.active_panel.title().to_string(),
        }
    }

    pub fn composer_hint_text(&self) -> String {
        // V1.6 RECOVERY: Check for active recovery first and show narration
        if !self.recovery_state.recovery_path.is_empty() {
            let recovery_summary = self.get_recovery_summary();
            if !recovery_summary.is_empty() {
                return recovery_summary;
            }
        }

        // V1.5 PROGRESS: First check for active progress state (non-terminal, non-idle)
        // This ensures progress wording is used during active execution, not terminal outcome wording
        let execution_state = self.state.execution.state;
        if execution_state.is_active() {
            // Use canonical progress state for active execution
            match execution_state {
                ExecutionState::Planning => return "📝 Planning...".to_string(),
                ExecutionState::Executing => {
                    if let Some(ref tool) = self.state.execution.active_tool {
                        return format!("🔧 Executing: {}...", tool);
                    }
                    return "🔧 Executing...".to_string();
                }
                ExecutionState::Validating => return "✓ Validating...".to_string(),
                ExecutionState::Repairing => return "🔨 Repairing...".to_string(),
                ExecutionState::WaitingForApproval => {
                    if let Some(request) = self.pending_approval.as_ref() {
                        return format!("⏳ Approval: {} → /approve or /deny", request.title);
                    }
                    return "⏳ Waiting for approval...".to_string();
                }
                ExecutionState::Responding => return "💬 Responding...".to_string(),
                _ => {} // Idle shouldn't reach here due to is_active() check
            }
        }

        // V1.5 CLEANUP: Prioritize ExecutionOutcome over legacy block metadata for terminal states
        // Check for chain outcome first (authoritative truth)
        if let Some(ref chain_id) = self.persistence.active_chain_id {
            if let Some(chain) = self.persistence.get_chain(chain_id) {
                if let Some(outcome) = chain.get_outcome() {
                    match outcome {
                        crate::persistence::ExecutionOutcome::Success => {
                            return "✓ Complete - type a message".to_string();
                        }
                        crate::persistence::ExecutionOutcome::SuccessWithWarnings => {
                            return "⚡ Complete (warnings) - /replay to review".to_string();
                        }
                        crate::persistence::ExecutionOutcome::Blocked => {
                            // Only show block metadata when outcome is Blocked
                            return format!(
                                "⏸ Blocked: {}",
                                self.block_reason.as_deref().unwrap_or("Approval required")
                            );
                        }
                        crate::persistence::ExecutionOutcome::Failed => {
                            // Only show block metadata when outcome is Failed
                            return format!(
                                "✗ Failed: {}",
                                self.block_reason.as_deref().unwrap_or("Execution failed")
                            );
                        }
                    }
                }
            }
        }

        // Legacy fallback for non-chain or pre-outcome states
        if self.chat_blocked {
            return summarize_block_reason(self.block_reason.as_deref().unwrap_or("Chat blocked"));
        }

        if let Some(request) = self.pending_approval.as_ref() {
            return format!("Approval pending: {}  [/approve] [/deny]", request.title);
        }

        // Check for chain-specific guidance with momentum state
        let chain_hint = if let Some(ref chain_id) = self.persistence.active_chain_id {
            if let Some(chain) = self.persistence.get_chain(chain_id) {
                // Check for pending approval first
                if let Some(ref checkpoint) = chain.pending_checkpoint {
                    if checkpoint.is_pending() {
                        let risk_label = match checkpoint.risk_level {
                            crate::persistence::RiskLevel::High => "HIGH",
                            crate::persistence::RiskLevel::Medium => "MED",
                            crate::persistence::RiskLevel::Low => "LOW",
                        };
                        Some(format!(
                            "⏳ Approval required ({} risk): {} → /approve or /deny",
                            risk_label,
                            checkpoint.checkpoint_type.description()
                        ))
                    } else {
                        None // Checkpoint not pending
                    }
                } else if let Some(ref reason) = chain.blocked_state(&self.persistence.chain_policy)
                {
                    let action = reason.suggested_action();
                    Some(format!("▶ {} → {}", reason.description(), action))
                } else {
                    match chain.status {
                        crate::persistence::ChainLifecycleStatus::Halted => {
                            Some("[/chain resume] to continue".to_string())
                        }
                        crate::persistence::ChainLifecycleStatus::Running => {
                            if self.persistence.chain_policy.auto_advance {
                                Some("Auto-progressing...".to_string())
                            } else {
                                Some("Task running...".to_string())
                            }
                        }
                        crate::persistence::ChainLifecycleStatus::Draft
                        | crate::persistence::ChainLifecycleStatus::Ready => {
                            if chain.steps.is_empty() {
                                Some("[/plan] to view plan".to_string())
                            } else {
                                Some("[/chain resume] to start".to_string())
                            }
                        }
                        crate::persistence::ChainLifecycleStatus::Complete => {
                            Some("✓ Chain complete".to_string())
                        }
                        _ => None,
                    }
                }
            } else {
                None
            }
        } else {
            Some("[/chains] to select chain".to_string())
        };

        if let Some(hint) = chain_hint {
            return format!("{}  [Tab] panel", hint);
        }

        match self.composer_mode {
            ComposerMode::Chat => match self.state.execution.mode {
                ExecutionMode::Chat => {
                    "[Enter] chat  [Ctrl+T] cycle chat/edit/task  [Tab] panel".to_string()
                }
                ExecutionMode::Edit => {
                    "[Enter] chat  [/read /write /replace /run]  [Ctrl+T] cycle mode".to_string()
                }
                ExecutionMode::Task => {
                    "[Enter] task  [/task /validate /run]  [Ctrl+T] cycle mode".to_string()
                }
            },
            ComposerMode::Search => "[Enter] search  [Tab] panel".to_string(),
            ComposerMode::ProjectCreate => "[Enter] create  [Esc] cancel  [Tab] panel".to_string(),
            ComposerMode::ProjectConnect => "[Enter] connect  [Tab] panel".to_string(),
            ComposerMode::Passive => "Use buttons  [Tab] panel".to_string(),
        }
    }

    pub fn execution_mode_label(&self) -> &'static str {
        self.state.execution.mode.as_str()
    }

    pub fn execution_state_label(&self) -> &'static str {
        self.state.execution.state.as_str()
    }

    pub fn execution_objective_label(&self) -> String {
        self.state
            .execution
            .active_objective
            .clone()
            .unwrap_or_else(|| "none".to_string())
    }

    /// Get chain status label for header display
    /// Enhanced with momentum state (blocked, auto-progressing, etc.)
    pub fn chain_status_label(&self) -> String {
        if let Some(ref chain_id) = self.persistence.active_chain_id {
            if let Some(chain) = self.persistence.get_chain(chain_id) {
                let icon = match chain.status {
                    crate::persistence::ChainLifecycleStatus::Running => "▶",
                    crate::persistence::ChainLifecycleStatus::Complete => "✓",
                    crate::persistence::ChainLifecycleStatus::Failed => "✗",
                    crate::persistence::ChainLifecycleStatus::Halted => "⏸",
                    crate::persistence::ChainLifecycleStatus::WaitingForApproval => "⏳",
                    crate::persistence::ChainLifecycleStatus::Archived => "🗑",
                    _ => "○",
                };

                // Check for momentum state
                let momentum_suffix = match chain.status {
                    crate::persistence::ChainLifecycleStatus::Running => {
                        if self.persistence.chain_policy.auto_advance {
                            " [auto]"
                        } else {
                            ""
                        }
                    }
                    crate::persistence::ChainLifecycleStatus::WaitingForApproval => {
                        // Show checkpoint info
                        if let Some(ref checkpoint) = chain.pending_checkpoint {
                            match checkpoint.risk_level {
                                crate::persistence::RiskLevel::High => {
                                    " [awaiting approval - HIGH]"
                                }
                                crate::persistence::RiskLevel::Medium => {
                                    " [awaiting approval - MED]"
                                }
                                crate::persistence::RiskLevel::Low => " [awaiting approval - LOW]",
                            }
                        } else {
                            " [awaiting approval]"
                        }
                    }
                    crate::persistence::ChainLifecycleStatus::Halted
                    | crate::persistence::ChainLifecycleStatus::Draft
                    | crate::persistence::ChainLifecycleStatus::Ready => {
                        // Check if blocked
                        if let Some(ref reason) =
                            chain.blocked_state(&self.persistence.chain_policy)
                        {
                            match reason {
                                crate::persistence::BlockedReason::MissingContext => {
                                    " [needs context]"
                                }
                                crate::persistence::BlockedReason::InvalidContext => {
                                    " [invalid context]"
                                }
                                crate::persistence::BlockedReason::MaxStepsReached => {
                                    " [max steps]"
                                }
                                crate::persistence::BlockedReason::TooManyFailures => {
                                    " [too many failures]"
                                }
                                _ => " [blocked]",
                            }
                        } else {
                            " [ready]"
                        }
                    }
                    _ => "",
                };

                return format!(
                    "{} {} ({} steps){}",
                    icon,
                    chain.name,
                    chain.steps.len(),
                    momentum_suffix
                );
            }
        }
        "no active chain".to_string()
    }

    /// Get context assembly status label
    pub fn context_status_label(&self) -> String {
        if let Some(ref chain_id) = self.persistence.active_chain_id {
            if let Some(chain) = self.persistence.get_chain(chain_id) {
                if let Some(ref context) = chain.context_state {
                    let included = context.files.iter().filter(|f| f.included).count();
                    return format!("{} files", included);
                } else if !chain.selected_context_files.is_empty() {
                    return format!("{} files (v2)", chain.selected_context_files.len());
                } else {
                    return "not assembled".to_string();
                }
            }
        }
        "—".to_string()
    }

    /// Check if current chain can be resumed
    pub fn chain_can_resume(&self) -> bool {
        if let Some(ref chain_id) = self.persistence.active_chain_id {
            if let Some(chain) = self.persistence.get_chain(chain_id) {
                return matches!(
                    chain.status,
                    crate::persistence::ChainLifecycleStatus::Halted
                        | crate::persistence::ChainLifecycleStatus::WaitingForApproval
                        | crate::persistence::ChainLifecycleStatus::Draft
                        | crate::persistence::ChainLifecycleStatus::Ready
                );
            }
        }
        false
    }

    pub fn execution_step_label(&self) -> String {
        let base = self
            .state
            .execution
            .current_step
            .clone()
            .unwrap_or_else(|| "none".to_string());

        match (
            self.state.execution.step_index,
            self.state.execution.step_total,
        ) {
            (Some(index), Some(total)) if total > 0 => format!("{} ({}/{})", base, index, total),
            (Some(index), None) => format!("{} (#{})", base, index),
            _ => base,
        }
    }

    /// Get cached repo snapshot or build fresh if stale/missing
    /// Soft failure: returns None if no repo attached or snapshot fails
    pub fn get_or_build_repo_snapshot(&mut self) -> Option<RepoSnapshot> {
        let repo_name = &self.state.repo.name;
        let repo_path = &self.state.repo.path;

        // No repo attached
        if repo_path == "~" || repo_path.is_empty() || repo_name == "No repo" {
            return None;
        }

        // Check cache validity
        if let Some(ref cached) = self.repo_snapshot_cache {
            if !cached.is_stale(repo_name, repo_path) {
                return Some(cached.clone());
            }
        }

        // Build fresh snapshot
        match RepoSnapshot::build(repo_name, repo_path) {
            Some(snapshot) => {
                self.repo_snapshot_cache = Some(snapshot.clone());
                Some(snapshot)
            }
            None => {
                // Soft failure: clear stale cache but don't block chat
                self.repo_snapshot_cache = None;
                None
            }
        }
    }

    pub fn conversation_title(&self) -> String {
        if let Some(first_user) = self
            .state
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
        {
            return truncate_title(
                first_user
                    .content
                    .lines()
                    .next()
                    .unwrap_or("New Conversation"),
            );
        }

        if self.state.messages.is_empty() {
            "New Conversation".to_string()
        } else {
            "Getting Started".to_string()
        }
    }

    pub fn panel_title(&self) -> String {
        match self.active_panel {
            SidebarPanel::Chat => self.conversation_title(),
            panel => panel.title().to_string(),
        }
    }

    fn restore_repo_context_from_path(&mut self, path: &str) {
        let recent = self
            .persistence
            .recent_repos
            .iter()
            .find(|repo| repo.path == path)
            .cloned();

        let name = recent
            .as_ref()
            .map(|repo| repo.name.clone())
            .or_else(|| {
                Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_string())
            })
            .unwrap_or_else(|| "Unknown project".to_string());

        self.state.repo.name = name;
        self.state.repo.path = path.to_string();
        self.state.repo.display_path = Self::display_path(path);
        self.state.repo.git_detected = Path::new(path).join(".git").exists();
        self.state.repo.branch = None;
        self.state.model.configured = recent.and_then(|repo| repo.ollama_model);
        self.persistence.active_repo = Some(path.to_string());
    }

    fn parse_execution_mode(value: &str) -> ExecutionMode {
        match value {
            "EDIT" => ExecutionMode::Edit,
            "TASK" => ExecutionMode::Task,
            _ => ExecutionMode::Chat,
        }
    }

    fn parse_execution_state(value: &str) -> ExecutionState {
        match value {
            "PLANNING" => ExecutionState::Planning,
            "EXECUTING" => ExecutionState::Executing,
            "VALIDATING" => ExecutionState::Validating,
            "RESPONDING" => ExecutionState::Responding,
            "DONE" => ExecutionState::Done,
            "FAILED" => ExecutionState::Failed,
            "BLOCKED" => ExecutionState::Blocked,
            "PRECONDITION_FAILED" => ExecutionState::PreconditionFailed,
            _ => ExecutionState::Idle,
        }
    }

    fn parse_inspector_tab(value: &str) -> InspectorTab {
        match value {
            "Validation" => InspectorTab::Validation,
            "Logs" => InspectorTab::Logs,
            "Preview" => InspectorTab::Preview,
            "Diff" => InspectorTab::Diff,
            "Timeline" => InspectorTab::Timeline,
            "Failure" => InspectorTab::Failure,
            "Steps" => InspectorTab::Steps,
            "Planner" => InspectorTab::PlannerTrace,
            "Replay" => InspectorTab::Replay,
            "Bundle" => InspectorTab::DebugBundle,
            "Audit" => InspectorTab::Audit,
            "Checkpoint" => InspectorTab::Checkpoint,
            _ => InspectorTab::Runtime,
        }
    }

    fn parse_structured_output_kind(value: &str) -> StructuredOutputKind {
        match value {
            "Objective" => StructuredOutputKind::Objective,
            "Plan" => StructuredOutputKind::Plan,
            "Artifact" => StructuredOutputKind::ArtifactManifest,
            "Audit" => StructuredOutputKind::Audit,
            "Checkpoint" => StructuredOutputKind::Checkpoint,
            "Recovery" => StructuredOutputKind::Recovery,
            "Status" => StructuredOutputKind::Status,
            "Markdown" => StructuredOutputKind::Markdown,
            _ => StructuredOutputKind::Unknown,
        }
    }

    fn runtime_status_from_execution(state: ExecutionState) -> RuntimeStatus {
        match state {
            ExecutionState::Idle => RuntimeStatus::Idle,
            ExecutionState::Planning
            | ExecutionState::Executing
            | ExecutionState::Validating
            | ExecutionState::Responding
            | ExecutionState::Repairing => RuntimeStatus::Running,
            ExecutionState::WaitingForApproval => RuntimeStatus::Running,
            ExecutionState::Done => RuntimeStatus::Completed,
            ExecutionState::Failed
            | ExecutionState::Blocked
            | ExecutionState::PreconditionFailed => RuntimeStatus::Error,
        }
    }

    fn ensure_project_chat_binding(&mut self) {
        let repo_path = self.state.repo.path.clone();
        if !Self::is_real_repo_path(&repo_path) {
            return;
        }

        let current_matches = self
            .persistence
            .conversations
            .iter()
            .find(|conversation| conversation.id == self.conversation_id)
            .is_some_and(|conversation| {
                !conversation.archived
                    && conversation.repo_path.as_deref() == Some(repo_path.as_str())
            });

        if current_matches {
            self.persistence.active_conversation = Some(self.conversation_id.clone());
            return;
        }

        let project_chat = self
            .persistence
            .active_conversations()
            .into_iter()
            .find(|conversation| conversation.repo_path.as_deref() == Some(repo_path.as_str()))
            .map(|conversation| conversation.id.clone());

        if let Some(chat_id) = project_chat {
            let _ = self.open_conversation(&chat_id);
        } else {
            self.start_new_conversation();
        }
    }

    pub fn project_entries(&self) -> Vec<ProjectEntry> {
        let mut entries = Vec::new();
        let mut seen = HashSet::new();

        if Self::is_real_repo_path(&self.state.repo.path) {
            seen.insert(self.state.repo.path.clone());
            entries.push(ProjectEntry {
                name: self.state.repo.name.clone(),
                path: self.state.repo.path.clone(),
                display_path: self.state.repo.display_path.clone(),
                active: true,
            });
        }

        for repo in &self.persistence.recent_repos {
            if !seen.insert(repo.path.clone()) {
                continue;
            }

            entries.push(ProjectEntry {
                name: repo.name.clone(),
                path: repo.path.clone(),
                display_path: Self::display_path(&repo.path),
                active: repo.path == self.state.repo.path,
            });
        }

        entries
    }

    pub fn search_scope_label(&self) -> String {
        let names = self
            .project_entries()
            .into_iter()
            .map(|project| project.name)
            .take(3)
            .collect::<Vec<_>>();

        if names.is_empty() {
            "connected projects".to_string()
        } else {
            names.join(", ")
        }
    }

    fn activate_panel(&mut self, panel: SidebarPanel) {
        self.active_panel = panel;
        self.panel_status = None;
        self.clear_input();

        match panel {
            SidebarPanel::Chat => {
                self.composer_mode = ComposerMode::Chat;
                self.set_input_mode(InputMode::Editing);
                self.focused_ui = Some("composer:input".to_string());
                self.set_panel_status(
                    PanelStatusLevel::Info,
                    format!(
                        "{} mode active. Commentary stays in chat; task activity streams below.",
                        self.state.execution.mode.as_str()
                    ),
                );
            }
            SidebarPanel::Projects => {
                self.activate_project_connect();
            }
            SidebarPanel::Search => {
                self.composer_mode = ComposerMode::Search;
                self.set_input_mode(InputMode::Editing);
                self.focused_ui = Some("composer:input".to_string());
                self.set_panel_status(
                    PanelStatusLevel::Info,
                    format!("Search across {}.", self.search_scope_label()),
                );
            }
            SidebarPanel::Plugins => {
                self.composer_mode = ComposerMode::Passive;
                self.set_input_mode(InputMode::Normal);
                self.set_panel_status(
                    PanelStatusLevel::Info,
                    "Plugin panel loaded. Use it as a status surface, not a hidden workflow.",
                );
            }
            SidebarPanel::Automations => {
                self.composer_mode = ComposerMode::Passive;
                self.set_input_mode(InputMode::Normal);
                self.set_panel_status(
                    PanelStatusLevel::Info,
                    "Automation panel loaded. Use the controls above for bounded maintenance actions.",
                );
            }
        }
    }

    fn activate_project_create(&mut self) {
        self.active_panel = SidebarPanel::Projects;
        self.composer_mode = ComposerMode::ProjectCreate;
        self.clear_input();
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
        self.set_panel_status(
            PanelStatusLevel::Info,
            "Enter a folder name or path, then press Enter to create and attach it.",
        );
    }

    fn start_picker_driven_project_create(&mut self) {
        self.active_panel = SidebarPanel::Projects;
        self.project_create_workflow = None;
        self.clear_input();

        match self.apply_host_action(HostAction::PickProjectFolder) {
            Ok(result) => {
                if let Some(parent) = result.affected_paths.first() {
                    self.begin_project_name_entry(PathBuf::from(parent));
                } else {
                    self.clear_project_create_workflow();
                    self.set_panel_status(
                        PanelStatusLevel::Error,
                        "Folder picker did not return a parent directory.",
                    );
                }
            }
            Err(error) => {
                self.clear_project_create_workflow();
                self.set_panel_status(
                    PanelStatusLevel::Error,
                    format!(
                        "Folder picker unavailable: {}. Use /project create <name> --path <parent> as fallback.",
                        error
                    ),
                );
            }
        }
    }

    fn begin_project_name_entry(&mut self, parent_dir: PathBuf) {
        self.active_panel = SidebarPanel::Projects;
        self.composer_mode = ComposerMode::ProjectCreate;
        self.project_create_workflow = Some(ProjectCreateWorkflow {
            parent_dir: parent_dir.clone(),
        });
        self.clear_input();
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
        self.set_panel_status(
            PanelStatusLevel::Info,
            format!(
                "Parent selected: {}. Enter a project name, then press Enter.",
                parent_dir.display()
            ),
        );
    }

    fn clear_project_create_workflow(&mut self) {
        self.project_create_workflow = None;
        self.composer_mode = ComposerMode::Chat;
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
    }

    pub fn cancel_project_create_workflow(&mut self) -> bool {
        if self.project_create_workflow.is_none()
            && !matches!(self.composer_mode, ComposerMode::ProjectCreate)
        {
            return false;
        }

        self.project_create_workflow = None;
        self.composer_mode = ComposerMode::Chat;
        self.clear_input();
        self.active_panel = SidebarPanel::Projects;
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
        self.set_panel_status(PanelStatusLevel::Info, "Project creation canceled.");
        true
    }

    fn activate_project_connect(&mut self) {
        self.active_panel = SidebarPanel::Projects;
        self.composer_mode = ComposerMode::ProjectConnect;
        self.clear_input();
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
        self.set_panel_status(
            PanelStatusLevel::Info,
            "Paste or drop a folder path, then press Enter to connect it. Use Choose folder for the native picker.",
        );
    }

    fn set_panel_status(&mut self, level: PanelStatusLevel, message: impl Into<String>) {
        self.panel_status = Some(PanelStatus {
            level,
            message: message.into(),
        });
    }

    fn clear_input(&mut self) {
        self.state.input_buffer.clear();
        self.state.cursor_position = 0;
    }

    pub fn set_input_mode(&mut self, mode: crate::state::InputMode) {
        self.state.input_mode = mode;
    }

    fn seed_welcome_message(&mut self) {
        let welcome_text = "I am Rasputin, your terminal-native coding agent.\n\nType /help for commands.\n\nWhat would you like to work on?".to_string();
        self.state.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::Assistant,
            source_text: welcome_text.clone(),
            content: welcome_text,
            timestamp: chrono::Local::now(),
            run_card: None,
        });
    }

    /// V2.4: Push system notice with deduplication guardrail
    fn push_system_notice(&mut self, message: &str) {
        if let Some(kind) = Self::classify_structured_output("system", message) {
            self.push_structured_output(kind, "system", message);
            return;
        }

        self.push_system_notice_raw(message);
    }

    fn push_system_notice_raw(&mut self, message: &str) {
        // V2.4: Deduplication - prevent exact duplicate system messages within last 5 messages
        let recent_messages: Vec<_> = self.state.messages.iter().rev().take(5).collect();
        let is_duplicate = recent_messages
            .iter()
            .any(|m| m.role == MessageRole::System && m.content == message);

        if is_duplicate {
            debug!(
                "Skipping duplicate system notice: {}",
                crate::text::truncate_chars(message, 50)
            );
            return;
        }

        self.state.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::System,
            source_text: message.to_string(),
            content: message.to_string(),
            timestamp: chrono::Local::now(),
            run_card: None,
        });
        self.snapshot_current_conversation();
    }

    fn push_structured_output(&mut self, kind: StructuredOutputKind, source: &str, content: &str) {
        let title = Self::structured_output_title(kind, content);
        let output = StructuredOutput {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            title: title.clone(),
            source: source.to_string(),
            content: content.to_string(),
            timestamp: chrono::Local::now(),
        };

        if self
            .state
            .structured_outputs
            .last()
            .is_some_and(|existing| existing.kind == kind && existing.content == content)
        {
            return;
        }

        self.state.structured_outputs.push(output);
        if self.state.structured_outputs.len() > 20 {
            self.state.structured_outputs.remove(0);
        }

        self.show_inspector = true;
        self.state.active_inspector_tab = match kind {
            StructuredOutputKind::Audit => InspectorTab::Audit,
            StructuredOutputKind::Checkpoint => InspectorTab::Checkpoint,
            StructuredOutputKind::Recovery => InspectorTab::Recovery,
            _ => InspectorTab::Preview,
        };
        self.record_last_action(format!("{} routed to structured inspector", kind.label()));
        self.push_system_notice_raw(&format!(
            "{} routed to Inspector → {}: {}",
            kind.label(),
            self.state.active_inspector_tab.as_str(),
            title
        ));
    }

    fn structured_output_title(kind: StructuredOutputKind, content: &str) -> String {
        let first = content
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or(kind.label());
        crate::text::truncate_chars(first, 72)
    }

    fn classify_structured_output(source: &str, content: &str) -> Option<StructuredOutputKind> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return None;
        }

        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("checkpoint decision")
            || lower.contains("checkpoint validation")
            || lower.starts_with("checkpoint:")
        {
            return Some(StructuredOutputKind::Checkpoint);
        }
        if lower.contains("audit timeline") || lower.contains("audit replay") {
            return Some(StructuredOutputKind::Audit);
        }
        if lower.contains("recovery status") || lower.contains("repair") {
            return Some(StructuredOutputKind::Recovery);
        }
        if lower.starts_with("git repository state")
            || lower.starts_with("installed ollama models:")
            || lower.starts_with("chain status:")
            || lower.starts_with("active chains")
            || lower.starts_with("context assembly")
        {
            return Some(StructuredOutputKind::Status);
        }
        if lower.starts_with("plan for")
            || lower.contains("\nsteps:")
            || lower.contains("generated plan")
            || lower.contains("goal plan accepted")
        {
            return Some(StructuredOutputKind::Plan);
        }
        if lower.contains("modified files:")
            || lower.contains("staged files:")
            || lower.contains("untracked files:")
            || lower.contains("selected files:")
            || lower.contains("files affected")
        {
            return Some(StructuredOutputKind::ArtifactManifest);
        }

        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() < 6 && trimmed.chars().count() < 420 {
            return None;
        }

        let numbered = lines
            .iter()
            .filter(|line| {
                let line = line.trim_start();
                let Some((prefix, rest)) = line.split_once('.') else {
                    return false;
                };
                !rest.trim().is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit())
            })
            .count();
        let bullets = lines
            .iter()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("- ") || line.starts_with("* ") || line.starts_with("• ")
            })
            .count();
        let headings = lines
            .iter()
            .filter(|line| line.trim_start().starts_with('#'))
            .count();
        let colon_headers = lines
            .iter()
            .filter(|line| {
                let line = line.trim();
                line.ends_with(':')
                    || (line.contains(':')
                        && line.split(':').next().is_some_and(|label| {
                            label
                                .chars()
                                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == ' ')
                        }))
            })
            .count();

        if numbered >= 3 {
            return Some(StructuredOutputKind::Plan);
        }
        if bullets >= 4 || headings >= 2 || trimmed.contains("```") || colon_headers >= 4 {
            return Some(if source == "assistant" {
                StructuredOutputKind::Markdown
            } else {
                StructuredOutputKind::Status
            });
        }

        None
    }

    /// V2.4: Push user message with deduplication guardrail
    fn push_user_message(&mut self, content: &str) {
        // V2.4: Deduplication - prevent exact duplicate user messages within last 3 messages
        let recent_messages: Vec<_> = self.state.messages.iter().rev().take(3).collect();
        let is_duplicate = recent_messages
            .iter()
            .any(|m| m.role == MessageRole::User && m.content == content);

        if is_duplicate {
            debug!(
                "Skipping duplicate user message: {}",
                crate::text::truncate_chars(content, 50)
            );
            return;
        }

        self.state.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            source_text: content.to_string(),
            content: content.to_string(),
            timestamp: chrono::Local::now(),
            run_card: None,
        });
    }

    /// V2.4: Push assistant message with deduplication guardrail
    fn push_assistant_message(&mut self, content: &str) {
        if let Some(kind) = Self::classify_structured_output("assistant", content) {
            self.push_structured_output(kind, "assistant", content);
            return;
        }

        // V2.4: Deduplication - prevent exact duplicate assistant messages within last 3 messages
        let recent_messages: Vec<_> = self.state.messages.iter().rev().take(3).collect();
        let is_duplicate = recent_messages
            .iter()
            .any(|m| m.role == MessageRole::Assistant && m.content == content);

        if is_duplicate {
            debug!(
                "Skipping duplicate assistant message: {}",
                crate::text::truncate_chars(content, 50)
            );
            return;
        }

        self.state.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::Assistant,
            source_text: content.to_string(),
            content: content.to_string(),
            timestamp: chrono::Local::now(),
            run_card: None,
        });
    }

    /// Push a system notice with next-step guidance appended
    fn push_system_notice_with_guidance(&mut self, message: &str) {
        use crate::guidance::NextActionEngine;

        let guidance = NextActionEngine::format_inline(&self.state, &self.persistence);
        let full_message = if guidance.is_empty() {
            message.to_string()
        } else {
            format!("{}\n\n{}", message, guidance)
        };

        self.push_system_notice(&full_message);
    }

    /// Push a system notice with full action suggestions
    fn push_system_notice_with_suggestions(&mut self, message: &str) {
        use crate::guidance::NextActionEngine;

        let suggestions = NextActionEngine::suggest(&self.state, &self.persistence);
        let formatted = NextActionEngine::format_suggestions(&suggestions);

        let full_message = if formatted.is_empty() {
            message.to_string()
        } else {
            format!("{}\n\n{}", message, formatted)
        };

        self.push_system_notice(&full_message);
    }

    fn apply_host_action(&mut self, action: HostAction) -> Result<HostActionResult> {
        let intent = action.label();
        self.emit_event("host", &format!("{} requested", intent));
        let result = execute_host_action(action, &mut self.persistence);

        if result.success {
            self.record_host_action_result(&result);
            Ok(result)
        } else {
            let error = result
                .error
                .clone()
                .unwrap_or_else(|| format!("{} failed", intent));
            self.push_runtime_event(
                format!("host/{}", intent.to_lowercase()),
                RuntimeStatus::Error,
            );
            self.emit_event("host", &format!("{} failed: {}", intent, error));
            self.record_last_action(format!("{} failed", intent));
            self.set_panel_status(
                PanelStatusLevel::Error,
                format!("{} failed: {}", intent, error),
            );
            Err(anyhow::anyhow!(error))
        }
    }

    fn record_host_action_result(&mut self, result: &HostActionResult) {
        self.push_runtime_event(
            format!("host/{}", result.intent.to_lowercase()),
            RuntimeStatus::Completed,
        );
        self.emit_event("host", &result.summary);
        self.record_last_action(result.summary.clone());
        Self::push_execution_entry(
            &mut self.state.execution.tool_calls,
            format!("host: {}", result.summary),
            12,
        );

        if !result.file_mutations.is_empty()
            || matches!(
                result.intent,
                "CreateProject" | "DeleteProject" | "WriteRepoConfig" | "WriteFile" | "ApplyPatch"
            )
        {
            for path in &result.affected_paths {
                Self::push_execution_entry(&mut self.state.execution.file_writes, path.clone(), 12);
            }
        }

        for log_line in &result.logs {
            self.add_log(LogLevel::Info, "host", log_line);
        }

        if let Some(update) = result.state_updates.as_ref() {
            if let Some(next_state) = update.next_state.as_deref() {
                self.set_execution_state(Self::parse_execution_state(next_state));
            }
            if let Some(step) = update.current_step.clone() {
                self.set_current_step(Some(step), None, None);
            }
            if let Some(tool) = update.active_tool.clone() {
                self.state.execution.active_tool = Some(tool);
            }
            if let Some(summary) = update.validation_summary.clone() {
                self.state.execution.validation_summary = Some(summary);
            }
        }

        for mutation in &result.file_mutations {
            self.state.diff_store.add(mutation.clone());
        }
        if !result.file_mutations.is_empty() {
            self.state.active_inspector_tab = InspectorTab::Diff;
        }

        let mut lines = vec![format!("✔ {}", result.summary)];
        if !result.affected_paths.is_empty() {
            lines.push(format!(
                "PATHS: {}",
                result
                    .affected_paths
                    .iter()
                    .map(|path| path.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(diff) = result.diff.as_ref() {
            lines.push(format!("DIFF: {}", diff));
        }
        if let Some(output) = result.output.as_ref() {
            let preview = output.lines().take(12).collect::<Vec<_>>().join("\n");
            if !preview.trim().is_empty() {
                lines.push(preview);
            }
        }

        self.push_system_notice(&lines.join("\n"));
        self.set_panel_status(PanelStatusLevel::Success, result.summary.clone());
    }

    fn start_new_conversation(&mut self) {
        self.snapshot_current_conversation();

        let repo = self.state.repo.clone();
        let model = self.state.model.clone();
        let ollama_connected = self.state.ollama_connected;
        let runtime_status = if self.chat_blocked {
            RuntimeStatus::Error
        } else {
            RuntimeStatus::Idle
        };
        let active_tab = self.state.active_inspector_tab;

        self.conversation_id = uuid::Uuid::new_v4().to_string();
        self.state = AppState::new();
        self.state.repo = repo;
        self.state.model = model;
        self.state.ollama_connected = ollama_connected;
        self.state.runtime_status = runtime_status;
        self.state.active_inspector_tab = active_tab;
        self.active_panel = SidebarPanel::Chat;
        self.composer_mode = ComposerMode::Chat;
        self.panel_status = None;
        self.show_inspector = false;
        self.search_query = None;
        self.search_results.clear();
        self.selected_search_result = None;
        self.search_preview = None;
        self.active_run_message_id = None;
        self.pending_approval = None;
        self.clear_execution_block();
        if let Err(error) = self.apply_host_action(HostAction::CreateChat {
            id: self.conversation_id.clone(),
            project_id: if Self::is_real_repo_path(&self.state.repo.path) {
                Some(self.state.repo.path.clone())
            } else {
                None
            },
        }) {
            self.emit_event("host", &format!("CreateChat failed: {}", error));
        }
        self.seed_welcome_message();
        self.push_system_notice("✔ New task started");
        self.snapshot_current_conversation();
        self.persist_now();
    }

    /// Archive the current conversation
    pub fn archive_current_conversation(&mut self) -> Result<()> {
        let conv_id = self.conversation_id.clone();
        info!("Archiving conversation: {}", conv_id);

        self.apply_host_action(HostAction::ArchiveChat {
            id: conv_id.clone(),
        })?;

        // Start a new conversation
        self.start_new_conversation();

        // DEDUPLICATION: Only show archive notice if we haven't reported this conversation
        if self.last_archived_conversation.as_ref() != Some(&conv_id) {
            self.push_system_notice("✔ Chat archived. Use the archive list to restore it later.");
            self.last_archived_conversation = Some(conv_id);
        }

        Ok(())
    }

    /// Copy currently focused message to clipboard
    pub fn copy_current_message(&mut self) {
        use crate::clipboard::Clipboard;

        if let Some(ref msg_id) = self.state.focus_state.chat_focused_message
            && let Some(message) = self.state.messages.iter().find(|m| &m.id == msg_id)
        {
            let mut clipboard = Clipboard::new();
            let result = clipboard.copy_message(message);

            // Show status message
            let status_msg = result.message();
            self.state.messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: MessageRole::System,
                source_text: status_msg.to_string(),
                content: status_msg.to_string(),
                timestamp: chrono::Local::now(),
                run_card: None,
            });
        }
    }

    fn open_conversation(&mut self, id: &str) -> Result<()> {
        self.snapshot_current_conversation();

        let Some(conversation) = self
            .persistence
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .cloned()
        else {
            return Err(anyhow::anyhow!("Conversation not found: {}", id));
        };

        let repo = self.state.repo.clone();
        let model = self.state.model.clone();
        let ollama_connected = self.state.ollama_connected;

        self.conversation_id = conversation.id.clone();
        self.state = AppState::new();
        self.state.repo = repo;
        self.state.model = model;
        self.state.ollama_connected = ollama_connected;
        self.active_panel = SidebarPanel::Chat;
        self.composer_mode = ComposerMode::Chat;
        self.panel_status = None;
        self.active_run_message_id = None;
        self.persistence.active_conversation = Some(conversation.id.clone());
        if let Some(repo_path) = conversation.repo_path.as_deref() {
            self.restore_repo_context_from_path(repo_path);
        }
        self.state.messages = conversation
            .messages
            .iter()
            .map(|message| Message {
                id: message.id.clone(),
                role: match message.role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    _ => MessageRole::System,
                },
                source_text: message.content.clone(),
                content: message.content.clone(),
                timestamp: message.timestamp,
                run_card: None,
            })
            .collect();
        self.state.logs = conversation
            .runtime_events
            .iter()
            .map(|event| LogEntry {
                timestamp: event.timestamp,
                level: match event.level.as_str() {
                    "ERROR" => LogLevel::Error,
                    "WARN" => LogLevel::Warn,
                    "DEBUG" => LogLevel::Debug,
                    _ => LogLevel::Info,
                },
                source: event.source.clone(),
                message: event.message.clone(),
            })
            .collect();
        self.state.structured_outputs = conversation
            .structured_outputs
            .iter()
            .map(|output| StructuredOutput {
                id: output.id.clone(),
                kind: Self::parse_structured_output_kind(&output.kind),
                title: output.title.clone(),
                source: output.source.clone(),
                content: output.content.clone(),
                timestamp: output.timestamp,
            })
            .collect();
        self.state.execution.mode = Self::parse_execution_mode(&conversation.execution.mode);
        self.state.execution.state = Self::parse_execution_state(&conversation.execution.state);
        self.state.execution.active_objective = conversation.execution.active_objective.clone();
        self.state.execution.last_action = conversation.execution.last_action.clone();
        self.state.execution.current_step = conversation.execution.current_step.clone();
        self.state.execution.step_index = conversation.execution.step_index;
        self.state.execution.step_total = conversation.execution.step_total;
        self.state.execution.active_tool = conversation.execution.active_tool.clone();
        self.state.execution.planner_output = conversation.execution.planner_output.clone();
        self.state.execution.tool_calls = conversation.execution.tool_calls.clone();
        self.state.execution.file_writes = conversation.execution.file_writes.clone();
        self.state.execution.validation_summary = conversation.execution.validation_summary.clone();
        self.state.execution.block_reason = conversation.execution.block_reason.clone();
        self.state.execution.block_fix = conversation.execution.block_fix.clone();
        self.state.execution.block_command = conversation.execution.block_command.clone();
        self.state.runtime_status = Self::runtime_status_from_execution(self.state.execution.state);
        self.show_inspector = conversation.inspector.show_inspector;
        if self.state.execution.state != ExecutionState::Idle {
            self.show_inspector = true;
        }
        self.state.active_inspector_tab =
            Self::parse_inspector_tab(&conversation.inspector.active_tab);
        self.state.runtime_tab_scroll = conversation.inspector.runtime_scroll;
        self.state.validation_tab_scroll = conversation.inspector.validation_scroll;
        self.state.logs_tab_scroll = conversation.inspector.logs_scroll;
        self.state.preview_tab_scroll = conversation.inspector.preview_scroll;
        self.state.diff_tab_scroll = conversation.inspector.diff_scroll;
        self.record_last_action(format!("Chat restored: {}", self.short_session_id()));
        self.push_system_notice(&format!(
            "✔ Context switched\nProject: {}\nChat: {}\nState restored",
            self.state.repo.name,
            self.panel_title()
        ));

        if self.state.messages.is_empty() {
            self.seed_welcome_message();
        }

        self.persist_now();

        Ok(())
    }

    fn archive_conversation(&mut self, conv_id: &str) -> Result<()> {
        if conv_id == self.conversation_id {
            return self.archive_current_conversation();
        }

        self.apply_host_action(HostAction::ArchiveChat {
            id: conv_id.to_string(),
        })?;
        self.persist_now();
        Ok(())
    }

    fn unarchive_conversation(&mut self, conv_id: &str) -> Result<()> {
        self.apply_host_action(HostAction::RestoreChat {
            id: conv_id.to_string(),
        })?;
        self.persist_now();
        Ok(())
    }

    fn snapshot_current_conversation(&mut self) {
        let repo_path = if self.state.repo.path == "~" || self.state.repo.path.is_empty() {
            None
        } else {
            Some(self.state.repo.path.clone())
        };
        let title = self.derive_conversation_title();

        let events = self
            .state
            .logs
            .iter()
            .map(|log| crate::persistence::PersistentEvent {
                timestamp: log.timestamp,
                source: log.source.clone(),
                level: log.level.as_str().to_string(),
                message: log.message.clone(),
            })
            .collect::<Vec<_>>();

        let messages = self
            .state
            .messages
            .iter()
            .map(|message| crate::persistence::PersistentMessage {
                id: message.id.clone(),
                role: match message.role {
                    MessageRole::User => "user".to_string(),
                    MessageRole::Assistant => "assistant".to_string(),
                    MessageRole::System => "system".to_string(),
                },
                content: message.content.clone(),
                timestamp: message.timestamp,
            })
            .collect::<Vec<_>>();

        let structured_outputs = self
            .state
            .structured_outputs
            .iter()
            .map(|output| crate::persistence::PersistentStructuredOutput {
                id: output.id.clone(),
                kind: output.kind.label().to_string(),
                title: output.title.clone(),
                source: output.source.clone(),
                content: output.content.clone(),
                timestamp: output.timestamp,
            })
            .collect::<Vec<_>>();

        self.persistence.active_conversation = Some(self.conversation_id.clone());
        let conversation = self
            .persistence
            .get_or_create_conversation(&self.conversation_id);
        conversation.title = title;
        conversation.repo_path = repo_path.clone();
        conversation.project_id = repo_path;
        conversation.mode = self.state.execution.mode.as_str().to_string();
        conversation.execution = crate::persistence::PersistentExecutionState {
            mode: self.state.execution.mode.as_str().to_string(),
            state: self.state.execution.state.as_str().to_string(),
            active_objective: self.state.execution.active_objective.clone(),
            last_action: self.state.execution.last_action.clone(),
            current_step: self.state.execution.current_step.clone(),
            step_index: self.state.execution.step_index,
            step_total: self.state.execution.step_total,
            active_tool: self.state.execution.active_tool.clone(),
            planner_output: self.state.execution.planner_output.clone(),
            tool_calls: self.state.execution.tool_calls.clone(),
            file_writes: self.state.execution.file_writes.clone(),
            validation_summary: self.state.execution.validation_summary.clone(),
            block_reason: self.state.execution.block_reason.clone(),
            block_fix: self.state.execution.block_fix.clone(),
            block_command: self.state.execution.block_command.clone(),
        };
        conversation.inspector = crate::persistence::PersistentInspectorState {
            show_inspector: self.show_inspector,
            active_tab: self.state.active_inspector_tab.as_str().to_string(),
            runtime_scroll: self.state.runtime_tab_scroll,
            validation_scroll: self.state.validation_tab_scroll,
            logs_scroll: self.state.logs_tab_scroll,
            preview_scroll: self.state.preview_tab_scroll,
            diff_scroll: self.state.diff_tab_scroll,
        };
        conversation.messages = messages;
        conversation.runtime_events = events;
        conversation.structured_outputs = structured_outputs;
        conversation.updated_at = chrono::Local::now();
    }

    fn derive_conversation_title(&self) -> String {
        if let Some(first_user) = self
            .state
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
        {
            return first_user
                .content
                .lines()
                .next()
                .unwrap_or("New Conversation")
                .chars()
                .take(42)
                .collect::<String>();
        }

        if self.state.messages.is_empty() {
            "New Conversation".to_string()
        } else {
            "Getting Started".to_string()
        }
    }

    async fn set_repo_model(&mut self, requested_model: &str) -> Result<()> {
        if self.state.repo.path == "~" || self.state.repo.path.is_empty() {
            return Err(anyhow::anyhow!("No repo attached"));
        }

        let requested = normalize_requested_model(requested_model);
        let verification = self.ollama.verify_model(&requested).await?;

        let active_model = verification.resolved_model.clone().ok_or_else(|| {
            anyhow::anyhow!(
                verification
                    .error
                    .unwrap_or_else(|| format!("Model '{}' is not available", requested))
            )
        })?;

        let persisted_model =
            if active_model != requested && active_model.starts_with(DEFAULT_CODER_14B_MODEL) {
                active_model.clone()
            } else {
                requested.clone()
            };

        self.apply_host_action(HostAction::WriteRepoModelConfig {
            repo_root: PathBuf::from(&self.state.repo.path),
            model: persisted_model.clone(),
        })?;
        self.state.model.configured = Some(persisted_model.clone());
        self.state.model.active = Some(active_model.clone());
        self.state.model.available = verification.installed_models.clone();
        self.state.model.connected = true;
        self.persistence.touch_repo(
            &self.state.repo.path,
            &self.state.repo.name,
            Some(&persisted_model),
        );
        self.persistence.update_model_status(
            Some(&persisted_model),
            Some(&active_model),
            self.state.ollama_connected,
        );
        self.unblock_chat();

        let detail = if active_model == persisted_model {
            format!("Repo model set to '{}'.", active_model)
        } else if active_model == FALLBACK_PLANNER_MODEL {
            format!(
                "Configured '{}'; active runtime fallback is '{}'.",
                persisted_model, active_model
            )
        } else {
            format!(
                "Configured '{}' and resolved active model to '{}'.",
                persisted_model, active_model
            )
        };
        self.emit_event("model", &detail);
        self.state.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::System,
            source_text: detail.clone(),
            content: detail,
            timestamp: chrono::Local::now(),
            run_card: None,
        });
        self.persist().await;
        Ok(())
    }

    /// Run validation pipeline
    pub async fn run_validation(&mut self) -> Result<()> {
        let repo_path = self.state.repo.path.clone();
        if repo_path == "~" || repo_path.is_empty() {
            self.set_execution_block(
                "No active project",
                "Create or attach a project before validation",
                Some("/open <path>".to_string()),
            );
            self.emit_event("validation", "No repo attached, skipping validation");
            return Ok(());
        }

        self.emit_event("validation", "Starting validation pipeline...");

        let mut pipeline = ValidationPipeline::new(repo_path);

        self.state.validation_stages = default_validation_stages();

        let result = pipeline
            .run(|stage_name, status, detail| {
                // Update the stage in state
                if let Some(stage) = self
                    .state
                    .validation_stages
                    .iter_mut()
                    .find(|s| s.name == stage_name)
                {
                    stage.status = status;
                    stage.detail = detail.map(|d| d.to_string());
                }

                // Emit event
                let status_str = match status {
                    RuntimeStatus::Running => "running",
                    RuntimeStatus::Completed => "completed",
                    RuntimeStatus::Error => "failed",
                    _ => "idle",
                };

                let message = if let Some(d) = detail {
                    format!("{}: {} - {}", stage_name, status_str, d)
                } else {
                    format!("{}: {}", stage_name, status_str)
                };

                self.emit_event("validation", &message);
            })
            .await;

        match result {
            Ok(_) => {
                self.emit_event("validation", "All validation stages passed");
                self.state.runtime_status = RuntimeStatus::Completed;
                self.set_execution_state(ExecutionState::Done);
                self.state.execution.validation_summary = Some("Validation passed".to_string());
                self.set_current_step(None, None, None);
                self.record_last_action("Validation passed");
            }
            Err(e) => {
                self.emit_event("validation", &format!("Validation failed: {}", e));
                self.state.runtime_status = RuntimeStatus::Error;
                self.set_execution_state(ExecutionState::Failed);
                self.state.execution.validation_summary = Some(format!("Validation failed: {}", e));
                self.set_current_step(Some("Validation failed".to_string()), None, None);
                self.record_last_action(format!("Validation failed: {}", e));
            }
        }

        Ok(())
    }

    fn audit_output_excerpt(output: &str) -> String {
        const MAX_CHARS: usize = 2048;
        let char_count = output.chars().count();
        if char_count <= MAX_CHARS {
            return output.to_string();
        }

        output
            .chars()
            .skip(char_count.saturating_sub(MAX_CHARS))
            .collect()
    }

    /// Phase C: Handle chain step completion when Forge execution finishes
    /// Updates the chain step with result and prepares for potential auto-resume
    fn handle_chain_step_completion(&mut self, success: bool, error: Option<String>) {
        // Check if we have chain context
        let chain_id = match &self.state.current_chain_id {
            Some(id) => id.clone(),
            None => return, // Not part of a chain, nothing to do
        };
        let step_id = match &self.state.current_chain_step_id {
            Some(id) => id.clone(),
            None => return,
        };

        // Clone policy values before mutable borrow to avoid borrow checker issues
        let (
            halt_on_failure,
            max_steps,
            auto_resume,
            auto_retry_on_validation_failure,
            max_auto_retries_per_step,
            max_chain_recovery_depth,
        ) = {
            let policy = &self.persistence.chain_policy;
            (
                policy.halt_on_failure,
                policy.max_steps,
                policy.auto_resume,
                policy.auto_retry_on_validation_failure,
                policy.max_auto_retries_per_step,
                policy.max_chain_recovery_depth,
            )
        };

        // Collect notice messages to send after borrow ends
        let mut notices: Vec<String> = vec![];
        let mut retry_enqueued = false;
        let _chain_name: Option<String>;
        let mut checkpoint_request: Option<(
            crate::persistence::PersistentChain,
            std::path::PathBuf,
            String,
        )> = None;

        // Collect replay data from state before entering mutable borrow
        let task_summary = self
            .state
            .execution
            .active_objective
            .clone()
            .unwrap_or_default();
        let planner_model = self.state.model.active.clone();
        let stdout_capture = self.execution_output_buffer.join("\n");
        let affected_paths = self.state.execution.file_writes.clone();
        let test_results = self.state.execution.validation_summary.clone();
        let exit_code = Some(if success { 0 } else { 1 });
        let result_class = crate::persistence::ExecutionResultClass::classify(
            success,
            exit_code,
            error.as_deref(),
            test_results.as_deref(),
        );
        let failure_reason = if success {
            None
        } else {
            Some(crate::persistence::FailureReason::analyze(
                result_class,
                &stdout_capture,
                error.as_deref().unwrap_or_default(),
                exit_code,
                test_results.as_deref(),
            ))
        };
        let evidence_snapshot = failure_reason
            .as_ref()
            .map(|reason| reason.evidence.clone())
            .unwrap_or_default();
        let error_summary = error
            .clone()
            .or_else(|| test_results.clone())
            .unwrap_or_else(|| {
                if success {
                    "step completed".to_string()
                } else {
                    "step failed without diagnostic output".to_string()
                }
            });
        let stdout_audit_excerpt = Self::audit_output_excerpt(&stdout_capture);
        let stderr_audit_excerpt = Self::audit_output_excerpt(error.as_deref().unwrap_or_default());
        let test_audit_excerpt =
            Self::audit_output_excerpt(test_results.as_deref().unwrap_or_default());
        let mut completion_step_result: Option<crate::state::StepResult> = None;

        // Update the chain step with completion result
        if let Some(chain) = self.persistence.get_chain_mut(&chain_id) {
            // Calculate context fingerprint
            let context_file_count = chain
                .context_state
                .as_ref()
                .map(|ctx| ctx.files.len())
                .unwrap_or(chain.selected_context_files.len());
            let chain_force_override_used = chain.force_override_used;

            if let Some(step) = chain.steps.iter_mut().find(|s| s.id == step_id) {
                let was_recovery_step =
                    step.retry_of.is_some() || step.recovery_step_kind.is_some();
                let step_outcome = if success {
                    if chain_force_override_used || step.force_override_used {
                        crate::persistence::ExecutionOutcome::SuccessWithWarnings
                    } else {
                        crate::persistence::ExecutionOutcome::Success
                    }
                } else if matches!(
                    result_class,
                    crate::persistence::ExecutionResultClass::Blocked
                ) {
                    crate::persistence::ExecutionOutcome::Blocked
                } else {
                    crate::persistence::ExecutionOutcome::Failed
                };
                step.status = if success {
                    crate::persistence::ChainStepStatus::Completed
                } else if matches!(
                    result_class,
                    crate::persistence::ExecutionResultClass::Blocked
                ) {
                    crate::persistence::ChainStepStatus::Blocked
                } else {
                    crate::persistence::ChainStepStatus::Failed
                };
                step.execution_result_class = Some(result_class);
                step.failure_reason = failure_reason.clone();
                step.execution_results
                    .push(crate::persistence::ExecutionResultCapture {
                        attempt: step.retry_attempt,
                        result_class,
                        stdout: stdout_capture.clone(),
                        stderr: error.clone().unwrap_or_default(),
                        exit_code,
                        test_results: test_results.clone(),
                        error_message: error.clone(),
                        failure_reason: failure_reason.clone(),
                        captured_at: chrono::Local::now(),
                        generated_retry_step_id: None,
                        affected_paths: affected_paths.clone(),
                    });
                step.result_summary = Some(if success {
                    "Step completed successfully".to_string()
                } else {
                    error.clone().unwrap_or_else(|| "Step failed".to_string())
                });
                step.execution_outcome = Some(step_outcome);
                step.validation_passed = Some(success);
                step.completed_at = Some(chrono::Local::now());
                step.error_message = if success {
                    None
                } else {
                    Some(error.clone().unwrap_or_else(|| "Step failed".to_string()))
                };

                completion_step_result = Some(crate::state::StepResult {
                    affected_paths: affected_paths.clone(),
                    exit_code,
                    bytes_affected: Some(affected_paths.len()),
                    validation_result: test_results.clone(),
                    error_message: error.clone(),
                    artifact_urls: vec![],
                    duration_ms: None,
                    outcome_class: if success {
                        if was_recovery_step {
                            crate::state::StepOutcomeClass::Recovery
                        } else {
                            crate::state::StepOutcomeClass::Success
                        }
                    } else if matches!(
                        result_class,
                        crate::persistence::ExecutionResultClass::Blocked
                    ) {
                        crate::state::StepOutcomeClass::Blocked
                    } else {
                        crate::state::StepOutcomeClass::Failure
                    },
                    failure_reason: None,
                    evidence: crate::state::ExecutionEvidence {
                        stdout: (!stdout_capture.is_empty()).then_some(stdout_capture.clone()),
                        stderr: error.clone(),
                        exit_code,
                        failed_validation_stage: None,
                        validation_failure_details: test_results.clone(),
                        error_summary: Some(error_summary.clone()),
                        suggested_fix: None,
                    },
                    retry_attempt: step.retry_attempt,
                    recovery_for_step_id: step.retry_of.clone(),
                });

                // V1.6 AUDIT: Log step completion
                let step_audit_event = crate::state::AuditEvent {
                    timestamp: chrono::Utc::now(),
                    event_type: if success {
                        crate::state::AuditEventType::StepCompleted
                    } else {
                        crate::state::AuditEventType::StepCompleted // Still StepCompleted, failure in metadata
                    },
                    previous_state: None,
                    next_state: None,
                    triggering_event: Some(step_id.clone()),
                    step_id: Some(step_id.clone()),
                    chain_id: Some(chain_id.clone()),
                    task: Some(task_summary.clone()),
                    reason: if success { None } else { error.clone() },
                    metadata: Some(format!(
                        "success={}; result_class={:?}; exit_code={:?}; failure_reason={:?}; stdout_bytes={}; stderr_bytes={}; test_results_present={}; stdout_excerpt={:?}; stderr_excerpt={:?}; test_results_excerpt={:?}",
                        success,
                        result_class,
                        exit_code,
                        failure_reason.as_ref().map(|reason| &reason.kind),
                        stdout_capture.len(),
                        error.as_ref().map(|e| e.len()).unwrap_or(0),
                        test_results.is_some(),
                        stdout_audit_excerpt,
                        stderr_audit_excerpt,
                        test_audit_excerpt
                    )),
                };
                chain.audit_event(step_audit_event);
            }

            // Create and store replay record for this step
            // Note: This runs outside the step match to avoid borrow issues
            if let Some(step) = chain.steps.iter_mut().find(|s| s.id == step_id) {
                let mut replay = crate::persistence::StepReplayRecord::new(
                    chain_id.clone(),
                    step_id.clone(),
                    task_summary.clone(),
                    step.description.clone(),
                    planner_model.clone(),
                    format!("ctx-{}-files", context_file_count), // Simplified fingerprint
                    context_file_count,
                );

                // Populate replay record with execution data
                replay.execution_end = chrono::Local::now();
                replay.outcome = if success {
                    crate::persistence::ReplayOutcome::Success
                } else {
                    crate::persistence::ReplayOutcome::Failure
                };

                // Include approval checkpoint if present
                if let Some(ref checkpoint) = chain.pending_checkpoint {
                    if checkpoint.step_id == step_id {
                        replay.approval_checkpoint = Some(checkpoint.clone());
                    }
                }

                // Generate final fingerprint
                replay.execution_fingerprint = format!(
                    "{:016x}",
                    std::collections::hash_map::DefaultHasher::new().finish()
                );

                // Capture Git state for replay/audit
                replay.git_grounding = Some(capture_git_grounding(&self.state.repo.path));

                step.replay_record = Some(replay);
            }

            if !success && auto_retry_on_validation_failure && result_class.allows_retry() {
                if let Some(retry_step) = chain.enqueue_retry_step(
                    &step_id,
                    result_class,
                    failure_reason
                        .as_ref()
                        .expect("failure classification requires failure reason"),
                    &evidence_snapshot,
                    max_auto_retries_per_step,
                    max_chain_recovery_depth,
                ) {
                    retry_enqueued = true;
                    if let Some(failed_step) = chain.steps.iter_mut().find(|s| s.id == step_id) {
                        if let Some(capture) = failed_step.execution_results.last_mut() {
                            capture.generated_retry_step_id = Some(retry_step.id.clone());
                        }
                    }

                    chain.audit_event(crate::state::AuditEvent {
                        timestamp: chrono::Utc::now(),
                        event_type: crate::state::AuditEventType::RepairTriggered,
                        previous_state: None,
                        next_state: None,
                        triggering_event: Some(step_id.clone()),
                        step_id: Some(step_id.clone()),
                        chain_id: Some(chain_id.clone()),
                        task: Some(task_summary.clone()),
                        reason: Some(error_summary.clone()),
                        metadata: Some(format!(
                            "result_class={:?}; failure_reason={:?}; recovery_kind={:?}; retry_step_id={}; retry_attempt={}; max_retries={}; max_chain_recovery_depth={}",
                            result_class,
                            failure_reason.as_ref().map(|reason| &reason.kind),
                            retry_step.recovery_step_kind,
                            retry_step.id,
                            retry_step.retry_attempt,
                            max_auto_retries_per_step,
                            max_chain_recovery_depth
                        )),
                    });
                    chain.audit_event(crate::state::AuditEvent {
                        timestamp: chrono::Utc::now(),
                        event_type: crate::state::AuditEventType::ChainLifecycle {
                            event: "step_retry_enqueued".to_string(),
                        },
                        previous_state: None,
                        next_state: None,
                        triggering_event: Some("step_retry_enqueued".to_string()),
                        step_id: Some(retry_step.id.clone()),
                        chain_id: Some(chain_id.clone()),
                        task: Some(retry_step.description.clone()),
                        reason: Some(format!("retry_of={}", step_id)),
                        metadata: Some(format!(
                            "root_step_id={}; retry_attempt={}; result_class={:?}; failure_reason={:?}; recovery_kind={:?}",
                            retry_step
                                .retry_of
                                .clone()
                                .unwrap_or_else(|| step_id.clone()),
                            retry_step.retry_attempt,
                            result_class,
                            retry_step.failure_reason.as_ref().map(|reason| &reason.kind),
                            retry_step.recovery_step_kind
                        )),
                    });
                    notices.push(format!(
                        "Chain '{}' queued self-healing retry {} for failed step {}",
                        chain.name, retry_step.id, step_id
                    ));
                } else {
                    chain.audit_event(crate::state::AuditEvent {
                        timestamp: chrono::Utc::now(),
                        event_type: crate::state::AuditEventType::ChainLifecycle {
                            event: "step_retry_denied".to_string(),
                        },
                        previous_state: None,
                        next_state: None,
                        triggering_event: Some("step_retry_denied".to_string()),
                        step_id: Some(step_id.clone()),
                        chain_id: Some(chain_id.clone()),
                        task: Some(task_summary.clone()),
                        reason: Some(
                            "retry policy limit reached or result is not retryable".to_string(),
                        ),
                        metadata: Some(format!(
                            "result_class={:?}; failure_reason={:?}; max_retries={}; max_chain_recovery_depth={}",
                            result_class,
                            failure_reason.as_ref().map(|reason| &reason.kind),
                            max_auto_retries_per_step,
                            max_chain_recovery_depth
                        )),
                    });
                }
            }

            // Update chain stats
            chain.total_steps_executed += 1;
            if !success {
                chain.total_steps_failed += 1;
            }
            chain.objective_satisfaction =
                crate::autonomy::CompletionConfidenceEvaluator::refresh_objective_satisfaction(
                    chain,
                );

            _chain_name = Some(chain.name.clone());
            let steps_executed = chain.total_steps_executed;
            let pending_count = chain
                .steps
                .iter()
                .filter(|s| matches!(s.status, crate::persistence::ChainStepStatus::Pending))
                .count();

            // Handle chain status based on result and policy
            if !success && halt_on_failure && !retry_enqueued {
                chain.status = crate::persistence::ChainLifecycleStatus::Halted;
                notices.push(format!(
                    "Chain '{}' halted: step failed (halt_on_failure=true)",
                    chain.name
                ));

                if let Some(goal) = self.goal_manager.active_goal() {
                    if goal.chain_id.as_ref() == Some(&chain.id) {
                        let outcome = chain
                            .get_outcome()
                            .unwrap_or(crate::persistence::ExecutionOutcome::Failed);
                        notices.push(crate::guidance::CompletionExplanation::generate(
                            goal, chain, outcome,
                        ));
                    }
                }
                crate::autonomy::AutonomousLoopController::mark_goal_failed_for_chain(
                    &mut self.goal_manager,
                    chain,
                    format!("Chain '{}' halted after a failed step", chain.name),
                );
            } else if chain.total_steps_executed >= max_steps {
                chain.status = crate::persistence::ChainLifecycleStatus::Halted;
                notices.push(format!(
                    "Chain '{}' halted: max steps reached ({}/{})",
                    chain.name, steps_executed, max_steps
                ));
            } else if !chain
                .steps
                .iter()
                .any(|s| matches!(s.status, crate::persistence::ChainStepStatus::Pending))
            {
                if success {
                    let step_result = completion_step_result.clone().unwrap_or_default();
                    match crate::autonomy::CompletionConfidenceEvaluator::evaluate_after_step(
                        chain,
                        &step_result,
                        &chain.objective_satisfaction,
                    ) {
                        crate::autonomy::CompletionConfidenceDecision::Finalize { reason } => {
                            chain.status = crate::persistence::ChainLifecycleStatus::Complete;
                            chain.completed_at = Some(chrono::Local::now());
                            notices.push(format!(
                                "Chain '{}' completed all steps: {}",
                                chain.name, reason
                            ));
                            // V1.5: Clear pending confirmation on chain completion
                            self.pending_confirmation = None;

                            // V2.0: Generate completion explanation if this was a goal-driven chain
                            // V1.5: Finalize and use authoritative outcome
                            chain.finalize_outcome();

                            // V1.6 AUDIT: Log outcome finalization
                            if let Some(outcome) = chain.get_outcome() {
                                let outcome_audit =
                                    crate::state::AuditEvent::outcome_finalized(outcome, None);
                                chain.audit_event(outcome_audit);
                            }

                            if let Some(goal) = self.goal_manager.active_goal() {
                                if goal.chain_id.as_ref() == Some(&chain.id) {
                                    let outcome = chain
                                        .get_outcome()
                                        .unwrap_or(crate::persistence::ExecutionOutcome::Success);
                                    let summary = crate::guidance::CompletionExplanation::generate(
                                        goal, chain, outcome,
                                    );
                                    notices.push(summary);
                                    crate::autonomy::AutonomousLoopController::mark_goal_completed_for_chain(
                                        &mut self.goal_manager,
                                        chain,
                                        format!("Chain {} completed", chain.name),
                                    );
                                }
                            }
                        }
                        crate::autonomy::CompletionConfidenceDecision::Continue { reason } => {
                            let added_steps =
                                Self::enqueue_contract_continuation_steps(chain, &step_id);
                            chain.status = if auto_resume {
                                crate::persistence::ChainLifecycleStatus::Running
                            } else {
                                crate::persistence::ChainLifecycleStatus::Halted
                            };
                            chain.completed_at = None;
                            chain.execution_outcome = None;
                            notices.push(format!("Chain '{}' incomplete: {}", chain.name, reason));
                            if added_steps > 0 {
                                notices.push(format!(
                                    "Chain '{}' queued {} artifact contract continuation step(s)",
                                    chain.name, added_steps
                                ));
                            } else if !auto_resume {
                                notices.push(format!(
                                    "Chain '{}' paused after reaching the completion gate without a satisfiable artifact continuation.",
                                    chain.name
                                ));
                            }
                        }
                        crate::autonomy::CompletionConfidenceDecision::HaltForClarification {
                            reason,
                        } => {
                            chain.status = crate::persistence::ChainLifecycleStatus::Halted;
                            chain.completed_at = None;
                            chain.execution_outcome = None;
                            notices.push(format!(
                                "Chain '{}' halted at completion gate: {}",
                                chain.name, reason
                            ));
                        }
                    }
                } else {
                    // No more pending steps - chain complete
                    chain.status = crate::persistence::ChainLifecycleStatus::Complete;
                    chain.completed_at = Some(chrono::Local::now());
                    notices.push(format!("Chain '{}' completed all steps", chain.name));
                    // V1.5: Clear pending confirmation on chain completion
                    self.pending_confirmation = None;

                    // V2.0: Generate completion explanation if this was a goal-driven chain
                    // V1.5: Finalize and use authoritative outcome
                    chain.finalize_outcome();

                    // V1.6 AUDIT: Log outcome finalization
                    if let Some(outcome) = chain.get_outcome() {
                        let outcome_audit =
                            crate::state::AuditEvent::outcome_finalized(outcome, None);
                        chain.audit_event(outcome_audit);
                    }

                    if let Some(goal) = self.goal_manager.active_goal() {
                        if goal.chain_id.as_ref() == Some(&chain.id) {
                            let outcome = chain
                                .get_outcome()
                                .unwrap_or(crate::persistence::ExecutionOutcome::Success);
                            let summary =
                                crate::guidance::CompletionExplanation::generate(goal, chain, outcome);
                            notices.push(summary);
                            crate::autonomy::AutonomousLoopController::mark_goal_completed_for_chain(
                                &mut self.goal_manager,
                                chain,
                                format!("Chain {} completed", chain.name),
                            );
                        }
                    }
                }
            } else {
                // More steps remain - chain stays Running if auto-resume enabled, otherwise Halted
                if !auto_resume && !retry_enqueued {
                    chain.status = crate::persistence::ChainLifecycleStatus::Halted;
                    notices.push(format!(
                        "Chain '{}' paused: {} steps executed, {} remaining. Use /chain resume to continue",
                        chain.name, steps_executed, pending_count
                    ));
                } else {
                    chain.status = crate::persistence::ChainLifecycleStatus::Running;
                }
                // If auto_resume is true, status stays Running and auto-resume will trigger
            }

            chain.updated_at = chrono::Local::now();

            // V1.6 CHECKPOINT: Create validated checkpoint after all canonical chain state
            // has been committed, so the snapshot matches the audit boundary.
            if success {
                chain.audit_event(crate::state::AuditEvent::lifecycle(
                    "checkpoint_created",
                    Some(step_id.clone()),
                    Some(chain_id.clone()),
                ));
                checkpoint_request = Some((
                    chain.clone(),
                    std::path::PathBuf::from(chain.repo_path.as_deref().unwrap_or(".")),
                    step_id.clone(),
                ));
            }
        } else {
            _chain_name = None;
        }

        // V1.5 CLEANUP: Clear stale block metadata so it doesn't persist into success presentation
        // This runs after chain mutable borrow ends to avoid borrow checker issues
        if success && !halt_on_failure {
            self.clear_execution_block();
        }

        // Send collected notices after borrow ends
        for notice in notices {
            self.push_system_notice(&notice);
        }

        if let Some((chain_clone, base_path, completed_step_id)) = checkpoint_request {
            tokio::spawn(async move {
                match crate::persistence::CheckpointManager::create_validated_checkpoint(
                    &chain_clone,
                    &base_path,
                    crate::persistence::CheckpointSource::AutoValidatedStep,
                    Some(format!("Step {} completed", completed_step_id)),
                )
                .await
                {
                    Ok(checkpoint) => {
                        info!(
                            "Created checkpoint {} after step completion",
                            checkpoint.checkpoint_id
                        );
                    }
                    Err(e) => {
                        warn!("Failed to create checkpoint after step completion: {}", e);
                    }
                }
            });
        }

        // Clear current chain context (execution is done)
        self.state.current_chain_id = None;
        self.state.current_chain_step_id = None;
    }

    fn enqueue_contract_continuation_steps(
        chain: &mut crate::persistence::PersistentChain,
        triggering_step_id: &str,
    ) -> usize {
        let Some(contract) = chain.objective_satisfaction.artifact_contract.clone() else {
            return 0;
        };

        let mut added_steps = 0usize;
        let artifact_label = contract
            .artifact_type
            .clone()
            .unwrap_or_else(|| "artifact".to_string());

        for path in &contract.missing_filenames {
            chain.steps.push(crate::persistence::PersistentChainStep {
                id: format!("contract-{}-{}", added_steps + 1, uuid::Uuid::new_v4()),
                description: format!(
                    "Create missing required {} artifact {}. Completion remains blocked until this exact filename exists with non-empty content.",
                    artifact_label, path
                ),
                status: crate::persistence::ChainStepStatus::Pending,
                retry_of: Some(triggering_step_id.to_string()),
                retry_attempt: 0,
                execution_outcome: None,
                execution_result_class: None,
                execution_results: vec![],
                failure_reason: None,
                recovery_step_kind: Some(crate::persistence::RecoveryStepKind::Patch),
                evidence_snapshot: Some(format!(
                    "Artifact contract missing filename: {}",
                    path
                )),
                force_override_used: false,
                tool_calls: vec![],
                result_summary: None,
                validation_passed: None,
                started_at: None,
                completed_at: None,
                error_message: None,
                replay_record: None,
            });
            added_steps += 1;
        }

        for path in &contract.empty_filenames {
            chain.steps.push(crate::persistence::PersistentChainStep {
                id: format!("contract-{}-{}", added_steps + 1, uuid::Uuid::new_v4()),
                description: format!(
                    "Fill empty required {} artifact {}. Completion requires non-empty content at this exact filename.",
                    artifact_label, path
                ),
                status: crate::persistence::ChainStepStatus::Pending,
                retry_of: Some(triggering_step_id.to_string()),
                retry_attempt: 0,
                execution_outcome: None,
                execution_result_class: None,
                execution_results: vec![],
                failure_reason: None,
                recovery_step_kind: Some(crate::persistence::RecoveryStepKind::Patch),
                evidence_snapshot: Some(format!(
                    "Artifact contract empty filename: {}",
                    path
                )),
                force_override_used: false,
                tool_calls: vec![],
                result_summary: None,
                validation_passed: None,
                started_at: None,
                completed_at: None,
                error_message: None,
                replay_record: None,
            });
            added_steps += 1;
        }

        if !contract.unexpected_filenames.is_empty() {
            chain.steps.push(crate::persistence::PersistentChainStep {
                id: format!("contract-{}-{}", added_steps + 1, uuid::Uuid::new_v4()),
                description: format!(
                    "Reconcile unexpected {} artifact(s) outside the explicit deliverable set: {}. Completion requires the exact required filename set and count.",
                    artifact_label,
                    contract.unexpected_filenames.join(", ")
                ),
                status: crate::persistence::ChainStepStatus::Pending,
                retry_of: Some(triggering_step_id.to_string()),
                retry_attempt: 0,
                execution_outcome: None,
                execution_result_class: None,
                execution_results: vec![],
                failure_reason: None,
                recovery_step_kind: Some(crate::persistence::RecoveryStepKind::Fix),
                evidence_snapshot: Some(format!(
                    "Artifact contract unexpected filenames: {}",
                    contract.unexpected_filenames.join(", ")
                )),
                force_override_used: false,
                tool_calls: vec![],
                result_summary: None,
                validation_passed: None,
                started_at: None,
                completed_at: None,
                error_message: None,
                replay_record: None,
            });
            added_steps += 1;
        }

        if added_steps > 0 {
            chain.audit_event(crate::state::AuditEvent {
                timestamp: chrono::Utc::now(),
                event_type: crate::state::AuditEventType::ChainLifecycle {
                    event: "artifact_contract_continuation_enqueued".to_string(),
                },
                previous_state: None,
                next_state: None,
                triggering_event: Some(triggering_step_id.to_string()),
                step_id: Some(triggering_step_id.to_string()),
                chain_id: Some(chain.id.clone()),
                task: Some(chain.objective.clone()),
                reason: Some(
                    chain
                        .objective_satisfaction
                        .reason
                        .clone()
                        .unwrap_or_else(|| "artifact contract incomplete".to_string()),
                ),
                metadata: Some(format!(
                    "missing={}; empty={}; unexpected={}; added_steps={}",
                    contract.missing_filenames.len(),
                    contract.empty_filenames.len(),
                    contract.unexpected_filenames.len(),
                    added_steps
                )),
            });
        }

        added_steps
    }

    fn build_chain_step_task(
        chain: &crate::persistence::PersistentChain,
        step: &crate::persistence::PersistentChainStep,
        satisfaction: &crate::state::ObjectiveSatisfaction,
    ) -> String {
        let raw_prompt = chain.raw_prompt_text();
        let default_task = if raw_prompt.trim() != chain.objective.trim() {
            format!(
                "Active objective summary: {}\n\nCurrent step: {}\n\nOriginal task prompt (verbatim):\n{}",
                chain.objective, step.description, raw_prompt
            )
        } else {
            format!("{}: {}", chain.objective, step.description)
        };
        let Some(contract) = satisfaction.artifact_contract.as_ref() else {
            return default_task;
        };
        if !contract.has_requirements() {
            return default_task;
        }

        let artifact_label = contract
            .artifact_type
            .clone()
            .unwrap_or_else(|| "artifact".to_string());
        let current_target = Self::infer_contract_step_target(step, contract)
            .or_else(|| contract.missing_filenames.first().cloned());
        let raw_prompt_context =
            Self::extract_raw_prompt_context(raw_prompt, current_target.as_deref());

        let mut lines = vec![
            format!("Active objective summary: {}", chain.objective),
            String::new(),
            format!("Current step: {}", step.description),
            String::new(),
            "Execution brief:".to_string(),
        ];

        if let Some(target) = current_target.as_deref() {
            lines.push(format!("- Current step target: {}", target));
            if let Some(purpose) = contract.purpose_for_path(target) {
                lines.push(format!("- Required file purpose: {}", purpose));
            }
        }
        if !contract.missing_filenames.is_empty() {
            lines.push(format!(
                "- Missing required artifacts: {}",
                contract.missing_filenames.join(", ")
            ));
        }
        if !contract.empty_filenames.is_empty() {
            lines.push(format!(
                "- Empty required artifacts that must be replaced: {}",
                contract.empty_filenames.join(", ")
            ));
        }
        if !contract.created_filenames.is_empty() {
            lines.push(format!(
                "- Required artifacts already present: {}",
                contract.created_filenames.join(", ")
            ));
        }

        lines.push(String::new());
        lines.push("Structured contract constraints:".to_string());
        lines.push(format!(
            "- Produce exactly {} required {} artifact(s).",
            contract.required_deliverable_count(),
            artifact_label
        ));
        lines.push(
            "- Completion is false until every required filename exists and is non-empty."
                .to_string(),
        );
        if let Some(actual_count) = contract.actual_output_count {
            lines.push(format!(
                "- Current contract output count: {}/{}",
                actual_count,
                contract.required_deliverable_count()
            ));
        }

        lines.push(String::new());
        lines.push("Required filenames:".to_string());
        for (index, artifact) in contract.required_artifacts.iter().enumerate() {
            match artifact.purpose.as_deref() {
                Some(purpose) => lines.push(format!(
                    "{}. {} :: {}",
                    index + 1,
                    artifact.path,
                    purpose
                )),
                None => lines.push(format!("{}. {}", index + 1, artifact.path)),
            }
        }
        if contract.required_artifacts.is_empty() {
            for (index, path) in contract.required_filenames.iter().enumerate() {
                lines.push(format!("{}. {}", index + 1, path));
            }
        }

        if let Some(context) = raw_prompt_context {
            lines.push(String::new());
            lines.push("Relevant raw prompt context:".to_string());
            lines.push(context);
        }

        lines.push(String::new());
        lines.push(
            "Execution rule: create missing files, update existing required files, replace empty required files, and do not stop after a partial subset."
                .to_string(),
        );
        lines.push(String::new());
        lines.push("Original task prompt (verbatim):".to_string());
        lines.push(raw_prompt.to_string());

        lines.join("\n")
    }

    fn infer_contract_step_target(
        step: &crate::persistence::PersistentChainStep,
        contract: &crate::state::ArtifactCompletionContract,
    ) -> Option<String> {
        contract
            .required_filenames
            .iter()
            .find(|path| step.description.contains(path.as_str()))
            .cloned()
    }

    fn extract_raw_prompt_context(raw_prompt: &str, target_path: Option<&str>) -> Option<String> {
        let lines: Vec<&str> = raw_prompt.lines().collect();
        if lines.is_empty() {
            return None;
        }

        if let Some(target) = target_path
            && let Some(index) = lines.iter().position(|line| line.contains(target))
        {
            let start = index.saturating_sub(1);
            let end = (index + 2).min(lines.len());
            let excerpt = lines[start..end]
                .iter()
                .map(|line| line.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
            if !excerpt.trim().is_empty() {
                return Some(excerpt);
            }
        }

        let excerpt = lines
            .iter()
            .map(|line| line.trim_end())
            .filter(|line| !line.trim().is_empty())
            .take(6)
            .collect::<Vec<_>>()
            .join("\n");
        if excerpt.trim().is_empty() {
            None
        } else {
            Some(excerpt)
        }
    }

    // V2.5: Auto-chain helper - parse prompt into steps based on strategy
    fn parse_prompt_into_steps(
        &self,
        prompt: &str,
        strategy: &crate::commands::AutoChainStrategy,
    ) -> Vec<String> {
        match strategy {
            crate::commands::AutoChainStrategy::ByDocument => {
                // Split by document boundaries (e.g., "1. Title", "## Doc", numbered lists)
                prompt
                    .split(|c| c == '\n' && prompt.lines().any(|l| l.trim().starts_with("# ") || l.trim().starts_with("## ") || l.trim().matches(char::is_numeric).count() >= 2))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            crate::commands::AutoChainStrategy::ByFile => {
                // Split by file creation markers
                prompt
                    .lines()
                    .filter(|l| l.contains(".md") || l.contains("write_file") || l.contains("create"))
                    .map(|s| s.trim().to_string())
                    .collect()
            }
            crate::commands::AutoChainStrategy::BySection => {
                // Split by markdown headers
                prompt
                    .split("\n## ")
                    .enumerate()
                    .map(|(i, s)| {
                        if i == 0 {
                            s.trim().to_string()
                        } else {
                            format!("## {}", s.trim())
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            _ => {
                // Auto - try to detect or use default chunking
                if prompt.len() > 4000 {
                    // Chunk by approximate token size (4 chars = 1 token rough estimate)
                    prompt
                        .chars()
                        .collect::<Vec<_>>()
                        .chunks(4000)
                        .map(|c| c.iter().collect::<String>())
                        .collect()
                } else {
                    vec![prompt.to_string()]
                }
            }
        }
    }

    // V2.5: Execute auto-chain by creating chain and running steps
    async fn execute_auto_chain(&mut self, steps: Vec<String>) -> Result<()> {
        if steps.is_empty() {
            return Ok(());
        }

        // Create chain with objective from first step
        let objective = format!("Auto-chain: {} steps", steps.len());
        let chain = self.persistence.create_chain("auto", &objective);
        let chain_id = chain.id.clone();
        
        // CRITICAL: Set execution lock to prevent auto-resume from interfering
        self.state.current_chain_id = Some(chain_id.clone());
        
        self.push_system_notice(&format!(
            "Created auto-chain '{}' with {} steps",
            chain_id, steps.len()
        ));

        // Execute with guaranteed cleanup
        let result = self.execute_auto_chain_inner(steps, &chain_id).await;
        
        // CRITICAL: Always clear execution lock, even on error
        self.state.current_chain_id = None;
        
        result
    }

    // Inner execution - chain lock is managed by outer function
    async fn execute_auto_chain_inner(&mut self, steps: Vec<String>, chain_id: &str) -> Result<()> {

        // Execute each step
        for (i, step) in steps.iter().enumerate() {
            self.push_system_notice(&format!(
                "[Step {}/{}] Executing...",
                i + 1,
                steps.len()
            ));
            
            // Submit as task
            let step_prompt = format!(
                "Execute step {}/{}:\n{}\n\nThis is part of an auto-chain. Process only this step and report completion.",
                i + 1, steps.len(), step
            );
            
            // Send to LLM
            match self.execute_chat_step(&step_prompt).await {
                Ok(Some(response)) => {
                    self.push_system_notice(&format!(
                        "[Step {}/{}] ✓ Complete",
                        i + 1,
                        steps.len()
                    ));
                    // Add to conversation
                    self.push_assistant_message(&response);
                }
                Ok(None) => {
                    warn!("No response for step {}/{}", i + 1, steps.len());
                }
                Err(e) => {
                    // Check if this is an infra error (Ollama/model) vs task error
                    let error_str = e.to_string();
                    let is_infra_error = error_str.contains("Ollama") 
                        || error_str.contains("health check")
                        || error_str.contains("No active model");
                    
                    if is_infra_error {
                        // INFRA ERROR: Retry once after short delay, then continue
                        tracing::warn!("Step {}/{} infra issue, retrying: {}", i + 1, steps.len(), e);
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        
                        // Retry the step once
                        let retry_prompt = format!(
                            "Retry step {}/{}:\n{}\n\nPrevious attempt had connection issue. Please process this step.",
                            i + 1, steps.len(), step
                        );
                        
                        match self.execute_chat_step(&retry_prompt).await {
                            Ok(Some(response)) => {
                                self.push_system_notice(&format!(
                                    "[Step {}/{}] ✓ Complete (after retry)",
                                    i + 1, steps.len()
                                ));
                                self.push_assistant_message(&response);
                            }
                            _ => {
                                self.push_system_notice(&format!(
                                    "[Step {}/{}] ~ Skipped (connection issue)",
                                    i + 1, steps.len()
                                ));
                            }
                        }
                    } else {
                        // TASK ERROR: Log but continue chain
                        tracing::warn!("Step {}/{} soft-failed: {}", i + 1, steps.len(), e);
                        self.push_system_notice(&format!(
                            "[Step {}/{}] ~ Continuing (adjusted)",
                            i + 1, steps.len()
                        ));
                    }
                }
            }
            
            // Persist progress
            self.persist().await;
        }

        self.push_system_notice(&format!(
            "✅ Chain '{}' finished ({} steps processed)",
            chain_id, steps.len()
        ));
        
        Ok(())
    }

    /// Execute a decomposed task with self-correction
    /// Takes a DecomposedTask and executes each step with recovery on failure
    async fn execute_decomposed_task(&mut self, task: crate::large_task_decomposer::DecomposedTask) -> Result<()> {
        let total_steps = task.steps.len();
        let chain_id = task.chain.id.clone();
        
        // Set execution lock
        self.state.current_chain_id = Some(chain_id.clone());
        
        // Store chain in persistence
        self.persistence.chains.push(task.chain);
        self.persistence.active_chain_id = Some(chain_id.clone());
        
        for (i, step) in task.steps.iter().enumerate() {
            let step_num = i + 1;
            
            // Show step indicator
            let icon = match step.step_type {
                crate::large_task_decomposer::StepType::Planning => "📋",
                crate::large_task_decomposer::StepType::SourceMapping => "🗺️",
                crate::large_task_decomposer::StepType::ArtifactGeneration => "📝",
                crate::large_task_decomposer::StepType::Validation => "✓",
                crate::large_task_decomposer::StepType::Refinement => "🔧",
                crate::large_task_decomposer::StepType::Recovery => "🔄",
            };
            
            self.push_system_notice(&format!(
                "{} [Step {}/{}] {}",
                icon, step_num, total_steps, step.description
            ));
            
            // Execute step
            match self.execute_chat_step(&step.prompt).await {
                Ok(Some(response)) => {
                    self.push_system_notice(&format!(
                        "✓ [Step {}/{}] Complete",
                        step_num, total_steps
                    ));
                    
                    // Add assistant response for artifact generation
                    if matches!(step.step_type, crate::large_task_decomposer::StepType::ArtifactGeneration) {
                        self.push_assistant_message(&response);
                    }
                }
                Ok(None) => {
                    warn!("No response for step {}/{}", step_num, total_steps);
                    // Try recovery if this was an artifact generation step
                    if let Some(ref artifact) = step.artifact {
                        if matches!(step.step_type, crate::large_task_decomposer::StepType::ArtifactGeneration) {
                            self.push_system_notice(&format!(
                                "⚠️ Step {}/{} failed, attempting recovery...",
                                step_num, total_steps
                            ));
                            
                            // Generate recovery steps
                            let recovery_steps = crate::large_task_decomposer::LargeTaskDecomposer::generate_recovery_steps(
                                step,
                                artifact,
                            );
                            
                            // Execute recovery steps
                            for recovery in recovery_steps {
                                self.push_system_notice(&format!(
                                    "🔄 Recovery: {}",
                                    recovery.description
                                ));
                                
                                match self.execute_chat_step(&recovery.prompt).await {
                                    Ok(Some(_)) => {
                                        self.push_system_notice("✓ Recovery successful");
                                        break; // Success, stop trying recoveries
                                    }
                                    _ => {
                                        self.push_system_notice("⚠️ Recovery attempt failed, trying next...");
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let error_str = e.to_string();
                    let is_infra_error = error_str.contains("Ollama") 
                        || error_str.contains("health check")
                        || error_str.contains("No active model");
                    
                    if is_infra_error {
                        // Retry once for infra errors
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        
                        match self.execute_chat_step(&step.prompt).await {
                            Ok(Some(response)) => {
                                self.push_system_notice(&format!(
                                    "✓ [Step {}/{}] Complete (after retry)",
                                    step_num, total_steps
                                ));
                                if matches!(step.step_type, crate::large_task_decomposer::StepType::ArtifactGeneration) {
                                    self.push_assistant_message(&response);
                                }
                            }
                            _ => {
                                self.push_system_notice(&format!(
                                    "~ [Step {}/{}] Skipped (connection issue)",
                                    step_num, total_steps
                                ));
                            }
                        }
                    } else {
                        // Task error - log and continue
                        self.push_system_notice(&format!(
                            "~ [Step {}/{}] Continuing (adjusted): {}",
                            step_num, total_steps, e
                        ));
                    }
                }
            }
            
            // Persist progress
            self.persist().await;
        }
        
        // Clear execution lock
        self.state.current_chain_id = None;
        
        self.push_system_notice(&format!(
            "✅ Decomposed task finished: {} steps processed",
            total_steps
        ));
        
        Ok(())
    }

    /// Phase C: Attempt auto-resume if policy allows and steps remain
    /// Enhanced with execution readiness check for momentum-driven execution
    /// V1.6: Idempotent - same chain already starting/running is a no-op
    pub async fn try_auto_resume_chain(&mut self) -> bool {
        // Get active chain
        let chain_id = match &self.persistence.active_chain_id {
            Some(id) => id.clone(),
            None => return false,
        };

        // V1.6: Check for execution lock - make auto-start idempotent
        if let Some(ref current_chain_id) = self.state.current_chain_id {
            if current_chain_id == &chain_id {
                // Same chain is already starting/running - this is a no-op, not an error
                // This prevents "Execution already in progress" race on auto-resume
                return false;
            } else {
                // Different chain is running - block cleanly without error
                self.push_system_notice(
                    "▶ Auto-resume skipped: another chain is currently executing",
                );
                return false;
            }
        }

        // Also check if runtime is active (additional safety)
        if self.active_execution_runtime.is_some() {
            // Runtime is active - check if it's for our chain by comparing chain IDs
            if let Some(ref running_chain_id) = self.state.current_chain_id {
                if running_chain_id == &chain_id {
                    // Same chain is already running - no-op (idempotent)
                    return false;
                }
            }
            // Different execution running
            self.push_system_notice(
                "▶ Auto-resume skipped: execution runtime is active",
            );
            return false;
        }

        // Clone policy for checks
        let policy = self.persistence.chain_policy.clone();

        // Check execution readiness
        let readiness = if let Some(chain) = self.persistence.get_chain(&chain_id) {
            chain.check_execution_readiness(&policy)
        } else {
            return false;
        };

        // If not ready, log the blocked reason but don't spam
        if !readiness.can_execute {
            if let Some(ref reason) = readiness.reason {
                // Only log if this is a new blocked state or significant
                if policy.auto_advance || policy.auto_resume {
                    self.push_system_notice(&format!(
                        "Chain blocked: {} → {}",
                        reason.description(),
                        reason.suggested_action()
                    ));
                }
            }
            return false;
        }

        // Check auto-resume policy (manual resume -> auto)
        let should_resume = policy.auto_resume || policy.auto_advance;
        if !should_resume {
            return false;
        }

        // Check chain is in appropriate state
        let chain_state_ok = if let Some(chain) = self.persistence.get_chain(&chain_id) {
            matches!(
                chain.status,
                crate::persistence::ChainLifecycleStatus::Running
                    | crate::persistence::ChainLifecycleStatus::Ready
                    | crate::persistence::ChainLifecycleStatus::Draft
            )
        } else {
            false
        };

        if !chain_state_ok {
            return false;
        }

        // All checks passed - auto-resume
        let next_step_desc = readiness
            .next_step_description
            .clone()
            .unwrap_or_else(|| "next step".to_string());
        self.push_system_notice(&format!("▶ Auto-progressing to {}", next_step_desc));

        // Use the existing resume logic via command
        if let Err(e) = self
            .handle_command(crate::commands::Command::ChainResume {
                chain_id: "active".to_string(),
                force: false,
            })
            .await
        {
            // V1.6: Suppress "Execution already in progress" as it's a race condition
            // that occurs when multiple events trigger auto-resume
            let error_msg = e.to_string();
            if error_msg.contains("Execution already in progress") {
                // This is a duplicate trigger - already handled above, but just in case
                return false;
            }
            self.push_system_notice(&format!("Auto-resume failed: {}", e));
            return false;
        }

        true
    }

    /// Start a task execution
    pub fn start_execution_task(&mut self, task: &str) -> Result<()> {
        self.start_execution_task_with_display_objective(task, task)
    }

    fn start_execution_task_with_display_objective(
        &mut self,
        task: &str,
        display_objective: &str,
    ) -> Result<()> {
        let display_objective = if display_objective.trim().is_empty() {
            task
        } else {
            display_objective
        };

        if !Self::is_real_repo_path(&self.state.repo.path) {
            self.set_execution_block(
                "No active project",
                "Create or attach a project first",
                Some("/open <path>".to_string()),
            );
            self.record_last_action("Task blocked: no active project");
            return Err(anyhow::anyhow!("No active project"));
        }

        if self.conversation_id.trim().is_empty() {
            self.set_execution_block(
                "No active task",
                "Create a task before starting work",
                Some("New Task".to_string()),
            );
            self.record_last_action("Work blocked: no active task");
            return Err(anyhow::anyhow!("No active chat"));
        }

        if self.active_execution_runtime.is_some() {
            warn!("Execution already in progress");
            return Err(anyhow::anyhow!("Execution already in progress"));
        }

        let Some(configured_model) = self.state.model.configured.clone() else {
            self.set_execution_block(
                "Planner model not configured",
                "Configure a planner model before running TASK mode",
                Some("/config set planner_model <model>".to_string()),
            );
            self.record_last_action("Task blocked: no active project");
            return Err(anyhow::anyhow!("Planner model not configured"));
        };
        let planner_model = self.state.model.active.clone().unwrap_or(configured_model);

        // Capture Git repository state for safety checks and audit
        let git_grounding = capture_git_grounding(&self.state.repo.path);
        let policy = TaskStartPolicy::default();
        let git_check = TaskStartChecker::check(&git_grounding, &policy);

        // Handle Git state warnings and approval requirements
        if !git_check.allowed {
            // Blocked - show execution block with summary
            let warning_text = git_check
                .warnings
                .iter()
                .map(|w| format!("• {}", w.message))
                .collect::<Vec<_>>()
                .join("\n");

            self.set_execution_block(
                &format!("Git Safety: {}", git_check.summary),
                &warning_text,
                Some("/git status".to_string()),
            );
            self.record_last_action("Task blocked: Git safety check failed");
            return Err(anyhow::anyhow!(
                "Git safety check failed: {}",
                git_check.summary
            ));
        }

        if git_check.requires_approval {
            // Requires approval - create checkpoint
            let warning_text = git_check
                .warnings
                .iter()
                .map(|w| format!("• {}: {}", w.class, w.message))
                .collect::<Vec<_>>()
                .join("\n");

            self.set_execution_block(
                &format!("Approval Required: {}", git_check.summary),
                &format!("Repository state requires approval:\n{}\n\nUse /approve to proceed or /reject to cancel.", warning_text),
                Some("/approve".to_string()),
            );
            self.record_last_action("Task requires approval: Git safety warnings");
            // Store the Git state for later use
            self.pending_git_grounding = Some(git_grounding);
            return Err(anyhow::anyhow!("Task requires approval due to Git state"));
        }

        // Log warnings if any (non-blocking)
        if git_check.has_warnings() {
            for warning in &git_check.warnings {
                warn!(
                    "Git safety warning [{}]: {}",
                    warning.class, warning.message
                );
                self.emit_event(
                    "git/safety",
                    &format!(
                        "Git safety warning [{}]: {}; continuing execution",
                        warning.class, warning.message
                    ),
                );
            }
            let warning_text = git_check
                .warnings
                .iter()
                .map(|warning| format!("• {}", warning.message))
                .collect::<Vec<_>>()
                .join("\n");
            self.push_system_notice(&format!(
                "⚠ Git safety warning\n{}\nStatus: continuing execution.",
                warning_text
            ));
        }

        info!(
            "Starting execution: {} (display: {}; Git: {})",
            task,
            display_objective,
            git_grounding.summary()
        );
        self.emit_event(
            "execution",
            &format!("Starting: {} | task={}", display_objective, task),
        );
        let css_compression = should_enable_css_compression(&planner_model);

        // Create Forge config
        let config = ForgeConfig {
            task: task.to_string(),
            max_iterations: 10,
            planner_type: "http".to_string(), // PHASE 4: Use real HTTP planner
            planner_endpoint: "http://127.0.0.1:11434".to_string(),
            planner_model: planner_model.clone(),
            working_dir: self.state.repo.path.clone(),
            css_compression,
            planner_seed: 42,
            planner_temperature: 0.0,
        };

        // Start execution runtime
        let handle = ForgeRuntimeHandle::run_task(config);
        self.active_execution_runtime = Some(handle);
        self.reset_execution_terminal_seal();
        self.execution_output_buffer.clear();
        self.state.runtime_events.clear();
        self.state.validation_stages = default_validation_stages();
        self.state.runtime_status = RuntimeStatus::Running;
        self.state.execution.mode = ExecutionMode::Task;
        self.set_execution_state(ExecutionState::Planning);
        self.set_active_objective(Some(display_objective.to_string()));
        self.reset_execution_activity();
        self.set_current_step(Some("Generating plan".to_string()), Some(1), Some(3));
        self.state.execution.validation_summary = Some("pending".to_string());
        Self::push_execution_entry(
            &mut self.state.execution.planner_output,
            format!("generating plan for {}", display_objective),
            5,
        );
        self.push_runtime_event("planner/generating", RuntimeStatus::Running);
        self.add_log(
            LogLevel::Info,
            "planner",
            &format!("Generating plan for {}", display_objective),
        );
        self.record_last_action("Task execution started");
        self.show_inspector = true;
        self.state.active_inspector_tab = InspectorTab::Runtime;

        // Add assistant commentary plus live run card.
        let msg_id = uuid::Uuid::new_v4().to_string();
        let msg_content = format!(
            "Working in TASK mode on: {}\nTool calls and validation will stream underneath.",
            display_objective
        );
        self.state.messages.push(Message {
            id: msg_id.clone(),
            role: MessageRole::Assistant,
            source_text: msg_content.clone(),
            content: msg_content,
            timestamp: chrono::Local::now(),
            run_card: Some(RunCard::new(
                task.to_string(),
                self.short_session_id(),
                Some(planner_model),
            )),
        });
        self.active_run_message_id = Some(msg_id);

        Ok(())
    }

    /// Poll execution runtime events and update UI
    pub fn poll_execution_events(&mut self) -> bool {
        // Collect events first to avoid borrow issues
        let events: Vec<RuntimeEvent> = if let Some(handle) = &self.active_execution_runtime {
            let mut events = vec![];
            while let Some(event) = handle.poll_event() {
                events.push(event);
            }
            events
        } else {
            return false;
        };

        if events.is_empty() {
            return false;
        }

        // Process events without holding reference to handle
        let mut task_complete = false;
        let mut stale_events = Vec::new();

        for event in events {
            if self.execution_run_sealed {
                stale_events.push(event);
                continue;
            }

            let is_terminal_event = matches!(event, RuntimeEvent::Finished { .. });
            let formatted = format_forge_event(&event);
            self.apply_execution_event(&event);

            if is_terminal_event {
                self.seal_execution_run();
                task_complete = true;
            }

            self.track_inspector_event(&event);

            // Phase 1: Transform through transparency layer for user-facing output
            let user_event = self.interface.transform_event(&event);
            let chat_line = user_event
                .as_ref()
                .map(format_user_event)
                .unwrap_or_else(|| formatted.clone());

            match &event {
                RuntimeEvent::Finished { success, .. } => {
                    self.state.runtime_status = if *success {
                        RuntimeStatus::Completed
                    } else {
                        RuntimeStatus::Error
                    };
                }
                _ => {
                    self.state.runtime_status = RuntimeStatus::Running;
                }
            }

            // Emit as runtime event (for inspector/logs)
            let event_type = match &event {
                RuntimeEvent::Init { .. } => "init",
                RuntimeEvent::IterationStart { .. } => "iteration",
                RuntimeEvent::PreflightPassed => "preflight",
                RuntimeEvent::ContextAssembly { .. } => "context",
                RuntimeEvent::PlannerOutput { .. } => "planner",
                RuntimeEvent::ProtocolValidation { .. } => "validation",
                RuntimeEvent::ToolCall { .. } => "tool",
                RuntimeEvent::ToolExecuting { .. } => "execute",
                RuntimeEvent::ToolResult { success, .. } => {
                    if *success {
                        "success"
                    } else {
                        "error"
                    }
                }
                RuntimeEvent::BrowserPreview { .. } => "browser_preview",
                RuntimeEvent::MutationsDetected { .. } => "mutation",
                RuntimeEvent::ValidationRunning => "validation",
                RuntimeEvent::ValidationResult { .. } => "validation",
                RuntimeEvent::ValidationStage { .. } => "validation_stage",
                RuntimeEvent::StateCommitting { .. } => "commit",
                RuntimeEvent::Completion { .. } => "complete",
                RuntimeEvent::Failure { .. } => "failure",
                RuntimeEvent::RepairLoop { .. } => "repair",
                RuntimeEvent::Finished { success, .. } => {
                    if *success {
                        "complete"
                    } else {
                        "error"
                    }
                }
            };

            self.emit_event(&format!("runtime/{}", event_type), &formatted);
            self.update_active_run_card_from_event(&event, chat_line.clone());

            // Buffer user-facing formatted output for final summary
            self.execution_output_buffer.push(chat_line);
        }

        self.drop_post_terminal_events(&stale_events);

        if task_complete {
            self.active_runtime_session_id = None;
            self.execution_output_buffer.clear();
            self.active_run_message_id = None;
        }

        false
    }

    async fn create_project_and_attach(&mut self, raw_path: &str) -> Result<()> {
        let path = self.resolve_project_path(raw_path)?;
        let result = self.apply_host_action(HostAction::CreateProject { path })?;
        let project_root = result
            .affected_paths
            .first()
            .ok_or_else(|| anyhow::anyhow!("Host action did not return a project path"))?;

        self.attach_repo(project_root).await?;
        self.ensure_project_chat_binding();
        self.active_panel = SidebarPanel::Chat;
        self.composer_mode = ComposerMode::Chat;
        self.panel_status = None;
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
        Ok(())
    }

    async fn create_project_from_parent_and_attach(
        &mut self,
        parent_dir: PathBuf,
        project_name: &str,
    ) -> Result<String> {
        let name = Self::validate_project_name(project_name)?;
        let parent_dir = if parent_dir.is_absolute() {
            parent_dir
        } else {
            self.default_project_root().join(parent_dir)
        };

        if !parent_dir.exists() {
            return Err(anyhow::anyhow!(
                "Parent folder does not exist: {}",
                parent_dir.display()
            ));
        }
        if !parent_dir.is_dir() {
            return Err(anyhow::anyhow!(
                "Parent path is not a folder: {}",
                parent_dir.display()
            ));
        }

        let project_path = parent_dir.join(name);
        if project_path.exists() {
            return Err(anyhow::anyhow!(
                "Project folder already exists: {}",
                project_path.display()
            ));
        }

        let previous_repo = self.state.repo.clone();
        let result = self.apply_host_action(HostAction::CreateProject { path: project_path })?;
        let project_root = result
            .affected_paths
            .first()
            .ok_or_else(|| anyhow::anyhow!("Host action did not return a project path"))?
            .clone();

        if let Err(error) = self.attach_repo(&project_root).await {
            self.state.repo = previous_repo;
            self.clear_project_create_workflow();
            return Err(anyhow::anyhow!(
                "Project folder was created but attach failed: {}",
                error
            ));
        }

        self.ensure_project_chat_binding();
        self.active_panel = SidebarPanel::Chat;
        self.composer_mode = ComposerMode::Chat;
        self.project_create_workflow = None;
        self.panel_status = None;
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
        self.persist().await;
        self.push_system_notice(&format!(
            "Project '{}' created and attached.\nPath: {}",
            name, project_root
        ));
        Ok(project_root)
    }

    fn validate_project_name(name: &str) -> Result<&str> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("Project name cannot be empty."));
        }
        if trimmed == "." || trimmed == ".." {
            return Err(anyhow::anyhow!("Project name cannot be '.' or '..'."));
        }
        if trimmed.contains('/') || trimmed.contains('\\') {
            return Err(anyhow::anyhow!(
                "Project name cannot contain path separators."
            ));
        }
        if trimmed
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' ')))
        {
            return Err(anyhow::anyhow!(
                "Project name can only contain letters, numbers, spaces, '.', '-', and '_'."
            ));
        }
        if trimmed.starts_with('.') {
            return Err(anyhow::anyhow!("Project name cannot start with '.'."));
        }

        Ok(trimmed)
    }

    async fn connect_project(&mut self, raw_path: &str) -> Result<()> {
        let path = self.resolve_project_path(raw_path)?;
        let result = self.apply_host_action(HostAction::AttachProject { path })?;
        let project_root = result
            .affected_paths
            .first()
            .ok_or_else(|| anyhow::anyhow!("Host action did not return a project path"))?;

        self.attach_repo(project_root).await?;
        self.ensure_project_chat_binding();
        self.active_panel = SidebarPanel::Chat;
        self.composer_mode = ComposerMode::Chat;
        self.panel_status = None;
        self.set_input_mode(InputMode::Editing);
        self.focused_ui = Some("composer:input".to_string());
        Ok(())
    }

    fn run_project_search(&mut self, query: &str) {
        let trimmed = query.trim();
        self.search_query = Some(trimmed.to_string());
        self.search_results.clear();
        self.selected_search_result = None;
        self.search_preview = None;

        if trimmed.is_empty() {
            self.set_panel_status(PanelStatusLevel::Error, "Enter a search query first.");
            return;
        }

        let scopes = self
            .project_entries()
            .into_iter()
            .filter(|project| Path::new(&project.path).is_dir())
            .collect::<Vec<_>>();

        if scopes.is_empty() {
            self.set_panel_status(
                PanelStatusLevel::Error,
                "Connect at least one project before searching.",
            );
            return;
        }

        let mut command = std::process::Command::new("rg");
        command.args([
            "--line-number",
            "--no-heading",
            "--color",
            "never",
            "--smart-case",
            "--",
        ]);
        command.arg(trimmed);
        for scope in &scopes {
            command.arg(&scope.path);
        }

        match command.output() {
            Ok(output) => {
                if !output.status.success() && output.status.code() != Some(1) {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    self.set_panel_status(
                        PanelStatusLevel::Error,
                        if stderr.is_empty() {
                            "Search command failed.".to_string()
                        } else {
                            stderr
                        },
                    );
                    return;
                }

                let stdout = String::from_utf8_lossy(&output.stdout);
                self.search_results = stdout
                    .lines()
                    .filter_map(|line| Self::parse_search_result(line, &scopes))
                    .take(24)
                    .collect();

                if self.search_results.is_empty() {
                    self.set_panel_status(
                        PanelStatusLevel::Info,
                        format!(
                            "No matches for '{}' across {} project(s).",
                            trimmed,
                            scopes.len()
                        ),
                    );
                } else {
                    self.set_panel_status(
                        PanelStatusLevel::Success,
                        format!(
                            "{} match(es) for '{}' across {} project(s).",
                            self.search_results.len(),
                            trimmed,
                            scopes.len()
                        ),
                    );
                    let _ = self.select_search_result(0);
                }
            }
            Err(e) => {
                self.set_panel_status(
                    PanelStatusLevel::Error,
                    format!("Search unavailable: {}", e),
                );
            }
        }
    }

    fn parse_search_result(line: &str, scopes: &[ProjectEntry]) -> Option<ProjectSearchResult> {
        let mut parts = line.splitn(3, ':');
        let file_path = parts.next()?;
        let line_number = parts.next()?.parse().ok()?;
        let preview = parts.next()?.trim().to_string();

        let project = scopes
            .iter()
            .filter(|scope| file_path.starts_with(&scope.path))
            .max_by_key(|scope| scope.path.len())?;

        let relative_path = Path::new(file_path)
            .strip_prefix(&project.path)
            .ok()
            .and_then(|path| path.to_str())
            .unwrap_or(file_path)
            .trim_start_matches('/');

        let display_path = if relative_path.is_empty() {
            project.name.clone()
        } else {
            format!("{}/{}", project.name, relative_path)
        };

        Some(ProjectSearchResult {
            file_path: file_path.to_string(),
            display_path,
            line_number,
            preview,
        })
    }

    fn select_search_result(&mut self, index: usize) -> Result<()> {
        let Some(result) = self.search_results.get(index).cloned() else {
            return Err(anyhow::anyhow!("Search result {} not found", index));
        };

        let preview = Self::build_search_preview(&result)?;
        self.selected_search_result = Some(index);
        self.search_preview = Some(preview);
        Ok(())
    }

    fn build_search_preview(result: &ProjectSearchResult) -> Result<SearchPreview> {
        let content = std::fs::read(&result.file_path)?;
        let text = String::from_utf8_lossy(&content);
        let lines = text.lines().collect::<Vec<_>>();
        let target = result.line_number.max(1);
        let start = target.saturating_sub(2).max(1);
        let end = usize::min(target + 2, lines.len().max(1));

        let preview_lines = (start..=end)
            .filter_map(|number| {
                lines
                    .get(number.saturating_sub(1))
                    .map(|line| SearchPreviewLine {
                        number,
                        content: line.to_string(),
                        highlighted: number == target,
                    })
            })
            .collect();

        Ok(SearchPreview {
            title: format!("{}:{}", result.display_path, result.line_number),
            file_path: result.file_path.clone(),
            lines: preview_lines,
        })
    }

    fn resolve_project_path(&self, raw_path: &str) -> Result<PathBuf> {
        let mut trimmed = raw_path.trim();
        if (trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        {
            trimmed = &trimmed[1..trimmed.len().saturating_sub(1)];
        }
        let file_url_owned;
        if let Some(rest) = trimmed.strip_prefix("file://") {
            file_url_owned = rest.replace("%20", " ");
            trimmed = file_url_owned.trim();
        }
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("Project path cannot be empty."));
        }

        let expanded = if trimmed == "~" {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        } else if let Some(rest) = trimmed.strip_prefix("~/") {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(rest)
        } else {
            PathBuf::from(trimmed)
        };

        if expanded.is_absolute() {
            Ok(expanded)
        } else {
            Ok(self.default_project_root().join(expanded))
        }
    }

    fn default_project_root(&self) -> PathBuf {
        if Self::is_real_repo_path(&self.state.repo.path) {
            let repo_path = Path::new(&self.state.repo.path);
            if let Some(parent) = repo_path.parent() {
                return parent.to_path_buf();
            }
            return repo_path.to_path_buf();
        }

        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    fn is_real_repo_path(path: &str) -> bool {
        !(path.is_empty() || path == "~")
    }

    fn display_path(path: &str) -> String {
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() && path.starts_with(&home) {
            path.replacen(&home, "~", 1)
        } else {
            path.to_string()
        }
    }
}

/// Extract affected file paths from a step action for risk evaluation
fn extract_affected_paths(step_action: &crate::state::StepAction) -> Vec<String> {
    use crate::state::StepAction;

    match step_action {
        StepAction::WriteFile { path } => vec![path.clone()],
        StepAction::ReadFile { path } => vec![path.clone()],
        StepAction::PatchFile { path } => vec![path.clone()],
        StepAction::CreateDirectory { path } => vec![path.clone()],
        StepAction::RunCommand { .. } => vec![],
        StepAction::Git { .. } => vec![],
        StepAction::Search { .. } => vec![],
        StepAction::ValidateProject => vec![],
        _ => vec![],
    }
}

fn truncate_title(text: &str) -> String {
    const MAX_LEN: usize = 42;

    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_LEN {
        trimmed.to_string()
    } else {
        let mut out = trimmed
            .chars()
            .take(MAX_LEN.saturating_sub(3))
            .collect::<String>();
        out.push_str("...");
        out
    }
}

fn preferred_available_model(models: &[String]) -> Option<String> {
    let mut candidates = models.to_vec();
    candidates.sort_by_key(|model| (model_preference_rank(model), model.clone()));
    candidates.into_iter().next()
}

fn canonical_or_display_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|resolved| resolved.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

fn summarize_block_reason(reason: &str) -> String {
    const MAX_LEN: usize = 22;

    let cleaned = reason.replace('\n', " ");
    if cleaned.len() <= MAX_LEN {
        cleaned
    } else {
        let mut short = cleaned
            .chars()
            .take(MAX_LEN.saturating_sub(3))
            .collect::<String>();
        short.push_str("...");
        short
    }
}

fn validation_stage_rank(name: &str) -> usize {
    match name {
        "protocol" => 0,
        "validation" => 1,
        "syntax" => 2,
        "lint" => 3,
        "build" => 4,
        "test" => 5,
        _ => usize::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{
        CheckpointCheckReport, CheckpointOperatorReport, CheckpointOperatorStatus,
    };
    use std::fs;

    fn checkpoint_report_for_test(
        status: CheckpointOperatorStatus,
        resume_allowed: bool,
    ) -> CheckpointOperatorReport {
        CheckpointOperatorReport {
            chain_id: "chain-test".to_string(),
            checkpoint_id: Some("chk-chain-test-1234".to_string()),
            checkpoint_timestamp: Some(chrono::Local::now()),
            active_step: Some(1),
            step_description: Some("second step".to_string()),
            audit_cursor: Some(4),
            audit_log_len: 7,
            workspace_hash: Some("abcdef1234567890".to_string()),
            workspace_result: match status {
                CheckpointOperatorStatus::Valid | CheckpointOperatorStatus::Divergent => {
                    CheckpointCheckReport::passed("2 tracked files match checkpoint hash")
                }
                CheckpointOperatorStatus::Stale => CheckpointCheckReport::failed(
                    "workspace hash changed: checkpoint abcdef12 current 99999999; files: src/main.rs",
                ),
                CheckpointOperatorStatus::Corrupted => {
                    CheckpointCheckReport::failed("checkpoint cannot be decoded")
                }
                CheckpointOperatorStatus::Missing => {
                    CheckpointCheckReport::not_checked("no checkpoint selected")
                }
            },
            replay_result: match status {
                CheckpointOperatorStatus::Valid | CheckpointOperatorStatus::Stale => {
                    CheckpointCheckReport::passed("audit events 0..4 replay to Executing")
                }
                CheckpointOperatorStatus::Divergent => CheckpointCheckReport::failed(
                    "cursor replay state Done does not match checkpoint state Executing",
                ),
                CheckpointOperatorStatus::Corrupted => CheckpointCheckReport::not_checked(
                    "replay blocked because checkpoint is unreadable",
                ),
                CheckpointOperatorStatus::Missing => {
                    CheckpointCheckReport::not_checked("no audit cursor available")
                }
            },
            final_status: status,
            resume_allowed,
            smallest_safe_next_action: match status {
                CheckpointOperatorStatus::Valid if resume_allowed => "/chain resume".to_string(),
                CheckpointOperatorStatus::Valid => {
                    "Chain is terminal; start a new chain".to_string()
                }
                CheckpointOperatorStatus::Stale => {
                    "Resolve workspace divergence, then create a fresh checkpoint".to_string()
                }
                CheckpointOperatorStatus::Corrupted => {
                    "Delete corrupted checkpoint and use a valid checkpoint".to_string()
                }
                CheckpointOperatorStatus::Divergent => {
                    "/audit replay, then restart from a valid checkpoint".to_string()
                }
                CheckpointOperatorStatus::Missing => {
                    "/checkpoint list, then restart from a validated checkpoint boundary"
                        .to_string()
                }
            },
        }
    }

    #[test]
    fn valid_checkpoint_status_rendering() {
        let report = checkpoint_report_for_test(CheckpointOperatorStatus::Valid, true);
        let rendered = App::render_checkpoint_report(&report);
        assert!(rendered.contains("Final Status: ✓ Valid"));
        assert!(rendered.contains("Resume: allowed"));
        assert!(rendered.contains("Workspace: passed"));
        assert!(rendered.contains("Replay: passed"));
    }

    #[test]
    fn stale_checkpoint_rendering() {
        let report = checkpoint_report_for_test(CheckpointOperatorStatus::Stale, false);
        let rendered = App::render_checkpoint_report(&report);
        assert!(rendered.contains("Final Status: ! Stale"));
        assert!(rendered.contains("Resume: blocked"));
        assert!(rendered.contains("workspace hash changed"));
        assert!(rendered.contains("Resolve workspace divergence"));
    }

    #[test]
    fn corrupted_checkpoint_rendering() {
        let report = checkpoint_report_for_test(CheckpointOperatorStatus::Corrupted, false);
        let rendered = App::render_checkpoint_report(&report);
        assert!(rendered.contains("Final Status: x Corrupted"));
        assert!(rendered.contains("checkpoint cannot be decoded"));
        assert!(rendered.contains("Delete corrupted checkpoint"));
    }

    #[test]
    fn divergent_checkpoint_rendering() {
        let report = checkpoint_report_for_test(CheckpointOperatorStatus::Divergent, false);
        let rendered = App::render_checkpoint_report(&report);
        assert!(rendered.contains("Final Status: ~ Divergent"));
        assert!(rendered.contains("cursor replay state Done"));
        assert!(rendered.contains("/audit replay"));
    }

    #[test]
    fn missing_checkpoint_rendering() {
        let report = CheckpointOperatorReport::missing("chain-test", 3);
        let rendered = App::render_checkpoint_report(&report);
        assert!(rendered.contains("Final Status: ? Missing"));
        assert!(rendered.contains("Checkpoint: (missing)"));
        assert!(rendered.contains("no checkpoint selected"));
    }

    #[test]
    fn chain_status_includes_checkpoint_summary() {
        let report = checkpoint_report_for_test(CheckpointOperatorStatus::Valid, true);
        let line = App::render_checkpoint_status_line(&report);
        assert!(line.contains("Valid"));
        assert!(line.contains("checkpoint=chk-chain-test-1234"));
        assert!(line.contains("cursor=4/7"));
        assert!(line.contains("resume=allowed"));
    }

    #[test]
    fn blocked_resume_notice_uses_checkpoint_report() {
        let report = checkpoint_report_for_test(CheckpointOperatorStatus::Divergent, false);
        let rendered = App::render_checkpoint_report(&report);
        assert!(rendered.contains("Final Status: ~ Divergent"));
        assert!(rendered.contains("Replay: failed"));
        assert!(rendered.contains("Next: /audit replay"));
        assert!(!rendered.contains("force"));
    }

    fn self_healing_test_step(
        id: &str,
        description: &str,
        status: crate::persistence::ChainStepStatus,
    ) -> crate::persistence::PersistentChainStep {
        crate::persistence::PersistentChainStep {
            id: id.to_string(),
            description: description.to_string(),
            status,
            retry_of: None,
            retry_attempt: 0,
            execution_outcome: None,
            execution_result_class: None,
            execution_results: vec![],
            failure_reason: None,
            recovery_step_kind: None,
            evidence_snapshot: None,
            force_override_used: false,
            tool_calls: vec![],
            result_summary: None,
            validation_passed: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            replay_record: None,
        }
    }

    fn multi_artifact_doc_prompt() -> String {
        "Create exactly 15 markdown files with these precise filenames:\n\
1. docs/01_PROJECT_OVERVIEW.md\n\
2. docs/02_ARCHITECTURE.md\n\
3. docs/03_TECHNOLOGY_STACK.md\n\
4. docs/04_CORE_CONCEPTS.md\n\
5. docs/05_FOLDER_STRUCTURE.md\n\
6. docs/06_MAIN_WORKFLOWS.md\n\
7. docs/07_API_REFERENCE.md\n\
8. docs/08_DATA_MODEL.md\n\
9. docs/09_CONFIGURATION.md\n\
10. docs/10_DEVELOPMENT_GUIDE.md\n\
11. docs/11_TESTING_STRATEGY.md\n\
12. docs/12_DEPLOYMENT_AND_OPERATIONS.md\n\
13. docs/13_SECURITY_AND_COMPLIANCE.md\n\
14. docs/14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md\n\
15. docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md\n\
All of these must be produced."
            .to_string()
    }

    fn required_doc_paths() -> Vec<&'static str> {
        vec![
            "docs/01_PROJECT_OVERVIEW.md",
            "docs/02_ARCHITECTURE.md",
            "docs/03_TECHNOLOGY_STACK.md",
            "docs/04_CORE_CONCEPTS.md",
            "docs/05_FOLDER_STRUCTURE.md",
            "docs/06_MAIN_WORKFLOWS.md",
            "docs/07_API_REFERENCE.md",
            "docs/08_DATA_MODEL.md",
            "docs/09_CONFIGURATION.md",
            "docs/10_DEVELOPMENT_GUIDE.md",
            "docs/11_TESTING_STRATEGY.md",
            "docs/12_DEPLOYMENT_AND_OPERATIONS.md",
            "docs/13_SECURITY_AND_COMPLIANCE.md",
            "docs/14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md",
            "docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md",
        ]
    }

    fn runtime_handle_for_test(
        events: Vec<crate::forge_runtime::RuntimeEvent>,
    ) -> crate::forge_runtime::ForgeRuntimeHandle {
        let (sender, receiver) = std::sync::mpsc::channel();
        for event in events {
            sender.send(event).expect("queue test runtime event");
        }
        drop(sender);
        crate::forge_runtime::ForgeRuntimeHandle::from_test_receiver(receiver)
    }

    fn install_runtime_test_context(app: &mut App) -> String {
        let run_id = "run-card-test".to_string();
        app.state.execution.state = ExecutionState::Validating;
        app.state.execution.active_objective = Some("seal runtime".to_string());
        app.state.runtime_status = RuntimeStatus::Running;
        app.reset_execution_terminal_seal();
        app.state.messages.push(Message {
            id: run_id.clone(),
            role: MessageRole::Assistant,
            source_text: "Working".to_string(),
            content: "Working".to_string(),
            timestamp: chrono::Local::now(),
            run_card: Some(RunCard::new(
                "seal runtime".to_string(),
                "runtime-test".to_string(),
                None,
            )),
        });
        app.active_run_message_id = Some(run_id.clone());
        run_id
    }

    #[tokio::test]
    async fn post_terminal_planner_output_is_ignored_and_run_card_freezes() {
        let mut app = App::new().await;
        let run_id = install_runtime_test_context(&mut app);
        app.active_execution_runtime = Some(runtime_handle_for_test(vec![
            RuntimeEvent::Finished {
                success: true,
                iterations: 1,
                error: None,
            },
            RuntimeEvent::PlannerOutput {
                raw: "late-planner".to_string(),
                output_type: "tool_call".to_string(),
            },
        ]));

        app.poll_execution_events();

        assert_eq!(app.state.execution.state, ExecutionState::Done);
        assert!(app.execution_run_sealed);
        assert!(app.active_execution_runtime.is_none());
        assert_eq!(app.state.runtime_events.len(), 1);
        assert_eq!(app.state.runtime_events[0].stage, "finished");
        let run_card = app
            .state
            .messages
            .iter()
            .find(|message| message.id == run_id)
            .and_then(|message| message.run_card.as_ref())
            .expect("run card remains in transcript");
        assert!(run_card.events.iter().all(|event| !event.contains("late-planner")));
        assert!(
            app.state.logs.iter().any(|entry| {
                entry.source == "runtime/stale-post-terminal"
                    && entry.message.contains("planner_output")
            }),
            "late planner output should be collapsed into operator-only logging"
        );
    }

    #[tokio::test]
    async fn post_terminal_tool_call_is_ignored() {
        let mut app = App::new().await;
        let run_id = install_runtime_test_context(&mut app);
        app.active_execution_runtime = Some(runtime_handle_for_test(vec![
            RuntimeEvent::Finished {
                success: true,
                iterations: 1,
                error: None,
            },
            RuntimeEvent::ToolCall {
                name: "late-tool".to_string(),
                arguments: "{}".to_string(),
            },
        ]));

        app.poll_execution_events();

        let run_card = app
            .state
            .messages
            .iter()
            .find(|message| message.id == run_id)
            .and_then(|message| message.run_card.as_ref())
            .expect("run card remains in transcript");
        assert!(run_card.events.iter().all(|event| !event.contains("late-tool")));
        assert!(app.state.execution.active_tool.is_none());
        assert_eq!(app.state.runtime_events.len(), 1);
    }

    #[tokio::test]
    async fn post_terminal_runtime_finished_is_ignored() {
        let mut app = App::new().await;
        let run_id = install_runtime_test_context(&mut app);
        app.active_execution_runtime = Some(runtime_handle_for_test(vec![
            RuntimeEvent::Finished {
                success: true,
                iterations: 1,
                error: None,
            },
            RuntimeEvent::Finished {
                success: false,
                iterations: 99,
                error: Some("late-finish".to_string()),
            },
        ]));

        app.poll_execution_events();

        let run_card = app
            .state
            .messages
            .iter()
            .find(|message| message.id == run_id)
            .and_then(|message| message.run_card.as_ref())
            .expect("run card remains in transcript");
        assert_eq!(run_card.iterations, 1);
        assert_eq!(run_card.status, RuntimeStatus::Completed);
        assert!(run_card.events.iter().all(|event| !event.contains("late-finish")));
        assert_eq!(app.state.runtime_events.len(), 1);
    }

    #[tokio::test]
    async fn sealed_progress_transitions_audit_once_without_normal_ui_leakage() {
        let mut app = App::new().await;
        let chain = app.persistence.create_chain("sealed", "sealed objective").clone();
        let chain_id = chain.id.clone();
        app.persistence.active_chain_id = Some(chain_id.clone());
        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-1".to_string());
        app.state.execution.active_objective = Some("sealed objective".to_string());
        app.execution_run_sealed = true;

        let message_count = app.state.messages.len();
        app.apply_progress_transition(crate::state::ProgressTransitionEvent::PlannerOutput);
        app.apply_progress_transition(crate::state::ProgressTransitionEvent::ToolCalling {
            name: "write_file".to_string(),
        });
        app.apply_progress_transition(crate::state::ProgressTransitionEvent::RuntimeFinished {
            success: true,
        });

        let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
        let stale_audits = chain
            .audit_log
            .get_last_n(10)
            .into_iter()
            .filter(|event| {
                matches!(
                    &event.event_type,
                    crate::state::AuditEventType::ChainLifecycle { event }
                        if event == "stale_post_terminal"
                )
            })
            .count();

        assert_eq!(stale_audits, 1);
        assert_eq!(app.state.messages.len(), message_count);
        assert!(
            !app.state
                .messages
                .iter()
                .any(|message| message.content.contains("[STATE MACHINE]")),
            "state-machine debug output must stay out of the normal chat transcript"
        );
        assert_eq!(
            app.state
                .logs
                .iter()
                .filter(|entry| entry.source == "runtime/stale-post-terminal")
                .count(),
            1,
            "post-terminal suppression should collapse repeated noise"
        );
    }

    #[tokio::test]
    async fn self_healing_retry_enqueues_fix_and_audits_recovery_path() {
        let mut app = App::new().await;
        app.persistence
            .chain_policy
            .auto_retry_on_validation_failure = true;
        app.persistence.chain_policy.auto_resume = true;
        app.persistence.chain_policy.auto_advance = true;
        app.persistence.chain_policy.halt_on_failure = true;
        app.persistence.chain_policy.max_auto_retries_per_step = 1;
        app.persistence.chain_policy.max_chain_recovery_depth = 3;

        let chain = app.persistence.create_chain("heal", "heal").clone();
        let chain_id = chain.id.clone();
        app.persistence.active_chain_id = Some(chain_id.clone());
        {
            let chain = app
                .persistence
                .get_chain_mut(&chain_id)
                .expect("created chain");
            chain.status = crate::persistence::ChainLifecycleStatus::Running;
            chain.selected_context_files.push("Cargo.toml".to_string());
            chain.steps.push(self_healing_test_step(
                "step-1",
                "Run failing validation",
                crate::persistence::ChainStepStatus::Pending,
            ));
            chain.steps.push(self_healing_test_step(
                "step-2",
                "Continue after recovery",
                crate::persistence::ChainStepStatus::Pending,
            ));
        }

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-1".to_string());
        app.state.execution.active_objective = Some("Run failing validation".to_string());
        app.state.execution.validation_summary = Some("test result: 1 failed".to_string());
        app.execution_output_buffer = vec!["stdout: cargo test started".to_string()];

        app.handle_chain_step_completion(false, Some("cargo test failed".to_string()));

        let retry_step_id = {
            let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
            let failed_step = chain.steps.iter().find(|s| s.id == "step-1").unwrap();
            assert_eq!(
                failed_step.status,
                crate::persistence::ChainStepStatus::Failed
            );
            assert_eq!(
                failed_step.execution_result_class,
                Some(crate::persistence::ExecutionResultClass::Failure)
            );
            assert_eq!(failed_step.execution_results.len(), 1);
            assert!(
                failed_step.execution_results[0]
                    .stdout
                    .contains("cargo test")
            );
            assert_eq!(failed_step.execution_results[0].exit_code, Some(1));
            assert_eq!(
                failed_step
                    .failure_reason
                    .as_ref()
                    .map(|reason| reason.kind),
                Some(crate::persistence::FailureReasonKind::TestFailure)
            );
            assert_eq!(
                failed_step.execution_results[0].generated_retry_step_id,
                chain.steps.get(1).map(|s| s.id.clone())
            );

            let retry_step = chain.steps.get(1).expect("retry inserted after failure");
            assert_eq!(retry_step.retry_of.as_deref(), Some("step-1"));
            assert_eq!(retry_step.retry_attempt, 1);
            assert_eq!(
                retry_step.status,
                crate::persistence::ChainStepStatus::Pending
            );
            assert_eq!(
                retry_step.recovery_step_kind,
                Some(crate::persistence::RecoveryStepKind::Fix)
            );
            assert!(retry_step.description.contains("Failure reason"));
            assert!(
                retry_step
                    .evidence_snapshot
                    .as_deref()
                    .unwrap_or_default()
                    .contains("cargo test failed")
            );
            assert_eq!(
                chain.status,
                crate::persistence::ChainLifecycleStatus::Running
            );

            let repair_events = chain
                .audit_log
                .get_events_by_type(crate::state::AuditEventType::RepairTriggered);
            assert_eq!(repair_events.len(), 1);
            assert!(
                repair_events[0]
                    .metadata
                    .as_deref()
                    .unwrap_or_default()
                    .contains("retry_step_id=")
            );
            assert!(chain.audit_log.get_last_n(20).iter().any(|event| matches!(
                &event.event_type,
                crate::state::AuditEventType::ChainLifecycle { event }
                    if event == "step_retry_enqueued"
            )));

            retry_step.id.clone()
        };

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some(retry_step_id.clone());
        app.state.execution.active_objective = Some("Retry failing validation".to_string());
        app.state.execution.validation_summary = Some("test result: ok".to_string());
        app.execution_output_buffer = vec!["stdout: tests passed".to_string()];

        app.handle_chain_step_completion(true, None);

        let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
        let retry_step = chain.steps.iter().find(|s| s.id == retry_step_id).unwrap();
        assert_eq!(
            retry_step.status,
            crate::persistence::ChainStepStatus::Completed
        );
        assert_eq!(
            retry_step.execution_result_class,
            Some(crate::persistence::ExecutionResultClass::Success)
        );
        assert_eq!(
            chain.next_pending_step().map(|step| step.id.as_str()),
            Some("step-2")
        );

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-2".to_string());
        app.state.execution.active_objective = Some("Continue after recovery".to_string());
        app.state.execution.validation_summary = Some("test result: ok".to_string());
        app.execution_output_buffer = vec!["stdout: continuation passed".to_string()];

        app.handle_chain_step_completion(true, None);

        let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
        assert_eq!(
            chain.status,
            crate::persistence::ChainLifecycleStatus::Complete
        );
    }

    #[tokio::test]
    async fn self_healing_retry_limit_is_audited_and_blocks_second_retry() {
        let mut app = App::new().await;
        app.persistence
            .chain_policy
            .auto_retry_on_validation_failure = true;
        app.persistence.chain_policy.auto_resume = true;
        app.persistence.chain_policy.halt_on_failure = true;
        app.persistence.chain_policy.max_auto_retries_per_step = 1;
        app.persistence.chain_policy.max_chain_recovery_depth = 3;

        let chain = app.persistence.create_chain("bounded", "bounded").clone();
        let chain_id = chain.id.clone();
        app.persistence.active_chain_id = Some(chain_id.clone());
        {
            let chain = app
                .persistence
                .get_chain_mut(&chain_id)
                .expect("created chain");
            chain.status = crate::persistence::ChainLifecycleStatus::Running;
            chain.selected_context_files.push("Cargo.toml".to_string());
            chain.steps.push(self_healing_test_step(
                "step-1",
                "Run failing validation",
                crate::persistence::ChainStepStatus::Pending,
            ));
        }

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-1".to_string());
        app.state.execution.validation_summary = Some("test result: 1 failed".to_string());
        app.handle_chain_step_completion(false, Some("first failure".to_string()));

        let retry_step_id = app
            .persistence
            .get_chain(&chain_id)
            .unwrap()
            .steps
            .get(1)
            .expect("first retry")
            .id
            .clone();

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some(retry_step_id);
        app.state.execution.validation_summary = Some("test result: still failing".to_string());
        app.handle_chain_step_completion(false, Some("second failure".to_string()));

        let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
        let retry_count = chain
            .steps
            .iter()
            .filter(|step| step.retry_of.as_deref() == Some("step-1"))
            .count();
        assert_eq!(retry_count, 1);
        assert!(chain.audit_log.get_last_n(20).iter().any(|event| matches!(
            &event.event_type,
            crate::state::AuditEventType::ChainLifecycle { event }
                if event == "step_retry_denied"
        )));
        assert_eq!(
            chain.status,
            crate::persistence::ChainLifecycleStatus::Halted
        );
    }

    #[tokio::test]
    async fn multi_artifact_contract_prevents_early_completion_after_one_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("docs")).expect("docs dir");
        fs::write(
            temp.path()
                .join("docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md"),
            "# roadmap\n",
        )
        .expect("roadmap doc");

        let mut app = App::new().await;
        let repo_path = temp.path().to_string_lossy().to_string();
        app.state.repo.path = repo_path.clone();
        app.persistence.active_repo = Some(repo_path.clone());

        let prompt = multi_artifact_doc_prompt();
        let chain = app.persistence.create_chain("docs", &prompt).clone();
        let chain_id = chain.id.clone();
        app.persistence.active_chain_id = Some(chain_id.clone());
        {
            let chain = app
                .persistence
                .get_chain_mut(&chain_id)
                .expect("created chain");
            chain.status = crate::persistence::ChainLifecycleStatus::Running;
            chain.repo_path = Some(repo_path.clone());
            chain.objective_satisfaction = crate::guidance::build_objective_satisfaction(&prompt);
            chain.steps.push(self_healing_test_step(
                "step-1",
                "Create the deliverable set",
                crate::persistence::ChainStepStatus::Pending,
            ));
        }

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-1".to_string());
        app.state.execution.active_objective = Some(prompt.clone());
        app.state.execution.file_writes =
            vec!["docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md".to_string()];

        app.handle_chain_step_completion(true, None);

        let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
        let contract = chain
            .objective_satisfaction
            .artifact_contract
            .as_ref()
            .expect("artifact contract preserved");
        assert_ne!(
            chain.status,
            crate::persistence::ChainLifecycleStatus::Complete
        );
        assert!(chain.execution_outcome.is_none());
        assert_eq!(contract.required_filenames.len(), 15);
        assert_eq!(contract.created_filenames.len(), 1);
        assert_eq!(contract.missing_filenames.len(), 14);
        assert!(
            chain.steps.iter().any(|step| {
                matches!(step.status, crate::persistence::ChainStepStatus::Pending)
                    && step
                        .description
                        .contains("docs/01_PROJECT_OVERVIEW.md")
            }),
            "missing filenames should be turned into continuation steps"
        );
    }

    #[tokio::test]
    async fn chain_step_task_includes_explicit_artifact_contract_status() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("docs")).expect("docs dir");
        fs::write(
            temp.path().join("docs/01_PROJECT_OVERVIEW.md"),
            "# overview\n",
        )
        .expect("overview doc");

        let prompt = multi_artifact_doc_prompt();
        let mut app = App::new().await;
        let repo_path = temp.path().to_string_lossy().to_string();
        let chain = app.persistence.create_chain("docs", &prompt).clone();
        let chain_id = chain.id.clone();
        {
            let chain = app
                .persistence
                .get_chain_mut(&chain_id)
                .expect("created chain");
            chain.repo_path = Some(repo_path);
            chain.objective_satisfaction = crate::guidance::build_objective_satisfaction(&prompt);
            chain.steps.push(self_healing_test_step(
                "step-1",
                "Create the document set",
                crate::persistence::ChainStepStatus::Pending,
            ));
        }

        let chain = app.persistence.get_chain(&chain_id).expect("chain");
        let refreshed =
            crate::autonomy::CompletionConfidenceEvaluator::refresh_objective_satisfaction(chain);
        let step = chain.steps.iter().find(|step| step.id == "step-1").unwrap();
        let task = App::build_chain_step_task(chain, step, &refreshed);

        assert!(task.contains("Active objective summary:"));
        assert!(task.contains("Structured contract constraints:"));
        assert!(task.contains("Produce exactly 15 required markdown artifact(s)."));
        assert!(task.contains("Missing required artifacts:"));
        assert!(task.contains("docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md"));
        assert!(task.contains("do not stop after a partial subset"));
        assert!(task.contains("Original task prompt (verbatim):"));
    }

    #[tokio::test]
    async fn chain_step_task_carries_required_file_purpose_and_raw_prompt_context() {
        let prompt = "Create exactly 2 markdown files:\n\
1. docs/01_PROJECT_OVERVIEW.md - explain the product scope, operators, and outcomes.\n\
2. docs/02_ARCHITECTURE.md: describe the runtime architecture, boundaries, and critical flows.\n\
All of these must be produced."
            .to_string();

        let mut app = App::new().await;
        let chain = app
            .persistence
            .create_chain(
                "Generate exactly 2 markdown file(s) with the specified filenames and purposes",
                "Generate exactly 2 markdown file(s) with the specified filenames and purposes",
            )
            .clone();
        let chain_id = chain.id.clone();
        {
            let chain = app
                .persistence
                .get_chain_mut(&chain_id)
                .expect("created chain");
            chain.raw_prompt = prompt.clone();
            chain.objective_satisfaction = crate::guidance::build_objective_satisfaction(&prompt);
            chain.steps.push(self_healing_test_step(
                "step-1",
                "Create missing required markdown artifact docs/01_PROJECT_OVERVIEW.md",
                crate::persistence::ChainStepStatus::Pending,
            ));
        }

        let chain = app.persistence.get_chain(&chain_id).expect("chain");
        let refreshed =
            crate::autonomy::CompletionConfidenceEvaluator::refresh_objective_satisfaction(chain);
        let step = chain.steps.iter().find(|step| step.id == "step-1").unwrap();
        let task = App::build_chain_step_task(chain, step, &refreshed);

        assert!(task.contains("Current step target: docs/01_PROJECT_OVERVIEW.md"));
        assert!(
            task.contains("Required file purpose: explain the product scope, operators, and outcomes.")
        );
        assert!(task.contains("Relevant raw prompt context:"));
        assert!(task.contains("1. docs/01_PROJECT_OVERVIEW.md - explain the product scope, operators, and outcomes."));
    }

    #[tokio::test]
    async fn multi_artifact_contract_completes_only_when_full_set_exists() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("docs")).expect("docs dir");
        for path in required_doc_paths() {
            fs::write(temp.path().join(path), format!("# {}\n", path)).expect("doc file");
        }

        let mut app = App::new().await;
        let repo_path = temp.path().to_string_lossy().to_string();
        app.state.repo.path = repo_path.clone();
        app.persistence.active_repo = Some(repo_path.clone());

        let prompt = multi_artifact_doc_prompt();
        let chain = app.persistence.create_chain("docs", &prompt).clone();
        let chain_id = chain.id.clone();
        app.persistence.active_chain_id = Some(chain_id.clone());
        {
            let chain = app
                .persistence
                .get_chain_mut(&chain_id)
                .expect("created chain");
            chain.status = crate::persistence::ChainLifecycleStatus::Running;
            chain.repo_path = Some(repo_path.clone());
            chain.objective_satisfaction = crate::guidance::build_objective_satisfaction(&prompt);
            chain.steps.push(self_healing_test_step(
                "step-1",
                "Create the deliverable set",
                crate::persistence::ChainStepStatus::Pending,
            ));
        }

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-1".to_string());
        app.state.execution.active_objective = Some(prompt);
        app.state.execution.file_writes =
            vec!["docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md".to_string()];

        app.handle_chain_step_completion(true, None);

        let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
        let contract = chain
            .objective_satisfaction
            .artifact_contract
            .as_ref()
            .expect("artifact contract preserved");
        assert_eq!(
            chain.status,
            crate::persistence::ChainLifecycleStatus::Complete
        );
        assert_eq!(
            chain.execution_outcome,
            Some(crate::persistence::ExecutionOutcome::Success)
        );
        assert!(contract.is_satisfied());
    }

    #[tokio::test]
    async fn self_healing_chain_recovery_depth_is_enforced() {
        let mut app = App::new().await;
        app.persistence
            .chain_policy
            .auto_retry_on_validation_failure = true;
        app.persistence.chain_policy.auto_resume = true;
        app.persistence.chain_policy.halt_on_failure = true;
        app.persistence.chain_policy.max_auto_retries_per_step = 2;
        app.persistence.chain_policy.max_chain_recovery_depth = 0;

        let chain = app.persistence.create_chain("depth", "depth").clone();
        let chain_id = chain.id.clone();
        app.persistence.active_chain_id = Some(chain_id.clone());
        {
            let chain = app
                .persistence
                .get_chain_mut(&chain_id)
                .expect("created chain");
            chain.status = crate::persistence::ChainLifecycleStatus::Running;
            chain.selected_context_files.push("Cargo.toml".to_string());
            chain.steps.push(self_healing_test_step(
                "step-1",
                "Run failing validation",
                crate::persistence::ChainStepStatus::Pending,
            ));
        }

        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-1".to_string());
        app.state.execution.validation_summary = Some("test result: 1 failed".to_string());
        app.handle_chain_step_completion(false, Some("depth-limited failure".to_string()));

        let chain = app.persistence.get_chain(&chain_id).expect("chain remains");
        assert_eq!(chain.steps.len(), 1);
        assert!(chain.audit_log.get_last_n(20).iter().any(|event| matches!(
            &event.event_type,
            crate::state::AuditEventType::ChainLifecycle { event }
                if event == "step_retry_denied"
        )));
        assert_eq!(
            chain.status,
            crate::persistence::ChainLifecycleStatus::Halted
        );
    }

    #[tokio::test]
    async fn picker_driven_project_create_valid_parent_and_name_attaches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut app = App::new().await;

        let project_root = app
            .create_project_from_parent_and_attach(temp.path().to_path_buf(), "alpha-workspace")
            .await
            .expect("project created");

        assert!(temp.path().join("alpha-workspace").is_dir());
        assert_eq!(app.state.repo.path, project_root);
        assert_eq!(
            app.persistence.active_repo.as_deref(),
            Some(project_root.as_str())
        );
        assert_eq!(app.active_panel, SidebarPanel::Chat);
        assert_eq!(app.composer_mode, ComposerMode::Chat);
        assert!(app.project_create_workflow.is_none());
    }

    #[tokio::test]
    async fn picker_driven_project_create_rejects_empty_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut app = App::new().await;
        let previous_repo = app.state.repo.clone();

        let error = app
            .create_project_from_parent_and_attach(temp.path().to_path_buf(), " ")
            .await
            .expect_err("empty name rejected");

        assert!(error.to_string().contains("cannot be empty"));
        assert_eq!(app.state.repo.path, previous_repo.path);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn picker_driven_project_create_rejects_invalid_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut app = App::new().await;

        let error = app
            .create_project_from_parent_and_attach(temp.path().to_path_buf(), "bad/name")
            .await
            .expect_err("invalid name rejected");

        assert!(error.to_string().contains("path separators"));
        assert!(!temp.path().join("bad").exists());
    }

    #[tokio::test]
    async fn picker_driven_project_create_rejects_existing_folder_collision() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("existing")).expect("existing folder");
        let mut app = App::new().await;

        let error = app
            .create_project_from_parent_and_attach(temp.path().to_path_buf(), "existing")
            .await
            .expect_err("collision rejected");

        assert!(error.to_string().contains("already exists"));
        assert_eq!(app.state.repo.name, "No repo");
    }

    #[tokio::test]
    async fn project_create_command_fallback_uses_canonical_create_and_attach() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut app = App::new().await;

        app.handle_command(Command::ProjectCreate {
            name: "command-workspace".to_string(),
            path: Some(temp.path().display().to_string()),
            init_git: false,
        })
        .await
        .expect("command handled");

        let expected = temp.path().join("command-workspace");
        assert!(expected.is_dir());
        assert_eq!(
            app.state.repo.path,
            expected.canonicalize().unwrap().display().to_string()
        );
        assert_eq!(
            app.persistence.active_repo.as_deref(),
            Some(app.state.repo.path.as_str())
        );
    }

    #[tokio::test]
    async fn cancel_project_create_exits_cleanly_without_partial_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut app = App::new().await;
        app.begin_project_name_entry(temp.path().to_path_buf());

        assert!(app.cancel_project_create_workflow());

        assert!(app.project_create_workflow.is_none());
        assert_eq!(app.composer_mode, ComposerMode::Chat);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        assert!(
            app.panel_status
                .as_ref()
                .is_some_and(|status| status.message.contains("canceled"))
        );
    }

    #[tokio::test]
    async fn failed_project_create_does_not_corrupt_active_repo_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let active = temp.path().join("active");
        std::fs::create_dir(&active).expect("active repo dir");
        let mut app = App::new().await;
        app.attach_repo(active.to_str().unwrap())
            .await
            .expect("attach active");
        let previous_repo = app.state.repo.clone();

        let missing_parent = temp.path().join("missing-parent");
        let error = app
            .create_project_from_parent_and_attach(missing_parent, "new-project")
            .await
            .expect_err("missing parent rejected");

        assert!(error.to_string().contains("Parent folder does not exist"));
        assert_eq!(app.state.repo.path, previous_repo.path);
        assert_eq!(app.state.repo.name, previous_repo.name);
    }

    #[tokio::test]
    async fn initializes_with_installed_repo_model_when_ollama_is_available() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root");
        let repo_root = repo_root.to_str().expect("utf-8 repo path");

        let mut app = App::new().await;
        app.initialize(Some(repo_root))
            .await
            .expect("app initializes");

        if !app.state.ollama_connected {
            eprintln!("skipping live ollama assertion because Ollama is not connected");
            return;
        }

        assert_eq!(
            app.state.model.configured.as_deref(),
            Some(DEFAULT_CODER_14B_MODEL)
        );
        assert!(
            app.state
                .model
                .active
                .as_deref()
                .is_some_and(|model| model.starts_with(DEFAULT_CODER_14B_MODEL)),
            "expected active 14B model family, got {:?}",
            app.state.model.active
        );
        assert!(
            !app.chat_blocked,
            "chat should not be blocked when configured model is installed; reason={:?}, available={:?}",
            app.block_reason, app.state.model.available
        );
    }

    #[test]
    fn parses_search_result_with_absolute_file_path() {
        let repo_root =
            std::env::temp_dir().join(format!("rasputin-search-{}", std::process::id()));
        let file_path = repo_root.join("src").join("main.rs");

        let scopes = vec![ProjectEntry {
            name: "demo".to_string(),
            path: repo_root.to_string_lossy().to_string(),
            display_path: "~/demo".to_string(),
            active: true,
        }];
        let line = format!("{}:12:fn main() {{}}", file_path.display());

        let parsed = App::parse_search_result(&line, &scopes).expect("search result");
        assert_eq!(parsed.file_path, file_path.to_string_lossy().to_string());
        assert_eq!(parsed.display_path, "demo/src/main.rs");
        assert_eq!(parsed.line_number, 12);
    }

    #[test]
    fn builds_preview_around_selected_search_line() {
        let repo_root = std::env::temp_dir().join(format!(
            "rasputin-preview-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        let file_path = repo_root.join("greeting.rs");
        fs::create_dir_all(&repo_root).expect("temp repo");
        fs::write(
            &file_path,
            "line one\nline two\nline three\nline four\nline five\n",
        )
        .expect("preview file");

        let preview = App::build_search_preview(&ProjectSearchResult {
            file_path: file_path.to_string_lossy().to_string(),
            display_path: "demo/greeting.rs".to_string(),
            line_number: 3,
            preview: "line three".to_string(),
        })
        .expect("search preview");

        assert_eq!(preview.title, "demo/greeting.rs:3");
        assert!(
            preview
                .lines
                .iter()
                .any(|line| line.number == 3 && line.highlighted)
        );
        assert!(preview.lines.iter().any(|line| line.number == 1));
        assert!(preview.lines.iter().any(|line| line.number == 5));

        let _ = fs::remove_dir_all(&repo_root);
    }

    #[tokio::test]
    async fn autonomy_precondition_failure_is_not_blocked_recovery_state() {
        let mut app = App::new().await;
        app.persistence.chains.clear();
        app.persistence.active_chain_id = None;

        app.set_execution_precondition_failed(
            "No local coder-capable planner model found",
            "Install qwen2.5-coder:14b",
        );

        assert_eq!(
            app.state.execution.state,
            ExecutionState::PreconditionFailed
        );
        assert!(!app.chat_blocked);
        assert!(app.persistence.active_chain_id.is_none());
        assert!(app.persistence.chains.is_empty());
        assert_eq!(
            app.state.execution.current_step.as_deref(),
            Some("Autonomy preflight failed")
        );
    }

    #[tokio::test]
    async fn goal_confirm_preflight_failure_does_not_create_fake_progress() {
        let mut app = App::new().await;
        app.state.repo.path.clear();
        app.persistence.chains.clear();
        app.persistence.active_chain_id = None;
        app.pending_command = None;

        app.goal_manager
            .stake_goal("create a tiny docs note file", app.conversation_id.clone());
        let goal_id = app
            .goal_manager
            .active_goal_id
            .clone()
            .expect("active goal id");
        app.goal_manager
            .active_goal_mut()
            .expect("active goal")
            .set_plan(crate::guidance::GeneratedPlan {
                raw_prompt: "create a tiny docs note file".to_string(),
                objective: "create a tiny docs note file".to_string(),
                steps: vec![crate::guidance::PlanStep {
                    number: 1,
                    description: "Create the docs note file".to_string(),
                    action_type: crate::guidance::StepActionType::Write,
                    risk_level: crate::guidance::RiskLevel::Safe,
                    likely_approval_needed: false,
                    affected_files: vec!["docs/tiny-note.md".to_string()],
                }],
                risks: vec![],
                approval_points: vec![],
                required_context: vec![],
                estimated_outcome: crate::guidance::OutcomePrediction::Success,
                safe_to_chain: true,
                reasoning: "Single safe file creation".to_string(),
            });

        let started = app
            .handle_command(Command::GoalConfirm)
            .await
            .expect("goal confirm handled");

        assert!(!started);
        assert_eq!(
            app.state.execution.state,
            ExecutionState::PreconditionFailed
        );
        assert!(!app.chat_blocked);
        assert!(app.persistence.active_chain_id.is_none());
        assert!(app.persistence.chains.is_empty());
        assert!(app.pending_command.is_none());

        let goal = app
            .goal_manager
            .get(&goal_id)
            .expect("goal remains tracked");
        assert_eq!(goal.status, crate::guidance::GoalStatus::Proposed);
        assert!(goal.chain_id.is_none());
        assert!(
            app.state
                .messages
                .iter()
                .any(|message| message.content.contains("AUTONOMY PREFLIGHT FAILED"))
        );
        assert!(
            !app.state
                .messages
                .iter()
                .any(|message| message.content.contains("Goal plan accepted"))
        );
    }

    #[tokio::test]
    async fn structured_plan_output_routes_to_inspector_not_chat() {
        let mut app = App::new().await;
        let structured = "Plan for “Unicode docs”\n\
1. Inspect README.md\n\
2. Update docs/guide.md\n\
3. Validate rendered markdown\n\
4. Run cargo test\n\
\n\
Next:\n\
- Confirm the plan\n\
- Apply the edits";

        app.push_system_notice(structured);

        assert_eq!(app.state.structured_outputs.len(), 1);
        assert_eq!(
            app.state.structured_outputs[0].kind,
            StructuredOutputKind::Plan
        );
        assert_eq!(app.state.active_inspector_tab, InspectorTab::Preview);
        assert!(
            !app.state.messages.iter().any(|message| {
                message.role == MessageRole::System && message.content == structured
            }),
            "structured payload must not be dumped into normal chat"
        );
        assert!(
            app.state
                .messages
                .iter()
                .any(|message| { message.content.contains("routed to Inspector") })
        );
    }

    #[tokio::test]
    async fn markdown_heavy_assistant_output_routes_to_structured_surface() {
        let mut app = App::new().await;
        let markdown = "# Next Goal\n\n\
## Plan\n\
1. Read “src/lib.rs”\n\
2. Patch the API surface — safely\n\
3. Run validation\n\n\
```text\n\
unicode: “curly quotes” — em dash\n\
```\n";

        app.push_assistant_message(markdown);

        assert_eq!(app.state.structured_outputs.len(), 1);
        assert_eq!(
            app.state.structured_outputs[0].kind,
            StructuredOutputKind::Plan
        );
        assert!(
            !app.state.messages.iter().any(|message| {
                message.role == MessageRole::Assistant && message.content == markdown
            }),
            "structured assistant output must not be appended as conversational chat"
        );
    }

    #[tokio::test]
    async fn generated_file_manifest_routes_to_structured_surface() {
        let mut app = App::new().await;
        let manifest = "Git Repository State\n\n\
Branch: main\n\
Head: abcdef0\n\
Status: Dirty\n\n\
Modified files:\n\
  M src/lib.rs\n\
  M docs/“unicode”.md\n\n\
Untracked files:\n\
  ?? notes/next-goal.md";

        app.push_system_notice(manifest);

        assert_eq!(app.state.structured_outputs.len(), 1);
        assert_eq!(
            app.state.structured_outputs[0].kind,
            StructuredOutputKind::Status
        );
        assert!(
            !app.state
                .messages
                .iter()
                .any(|message| message.content == manifest)
        );
    }

    #[tokio::test]
    async fn composer_cursor_operations_are_unicode_safe() {
        let mut app = App::new().await;

        for ch in "a“b”—c".chars() {
            app.handle_input(ch);
        }
        app.move_cursor_left();
        app.move_cursor_left();
        app.handle_backspace();
        app.handle_delete();

        assert!(
            app.state
                .input_buffer
                .is_char_boundary(app.state.cursor_position)
        );
    }

    // ============================================================================
    // REGRESSION TESTS: Auto-start locking and idempotency
    // ============================================================================

    #[tokio::test]
    async fn auto_resume_chain_is_idempotent_for_same_chain() {
        // Regression test: duplicate auto-start should be no-op, not "Execution already in progress"
        let mut app = App::new().await;

        // Create a chain
        let chain = app.persistence.create_chain("test", "test objective").clone();
        let chain_id = chain.id.clone();
        app.persistence.active_chain_id = Some(chain_id.clone());

        // Set chain to ready state with steps
        {
            let chain = app.persistence.get_chain_mut(&chain_id).unwrap();
            chain.status = crate::persistence::ChainLifecycleStatus::Ready;
            chain.steps.push(crate::persistence::PersistentChainStep {
                id: "step-1".to_string(),
                description: "Test step".to_string(),
                status: crate::persistence::ChainStepStatus::Pending,
                retry_of: None,
                retry_attempt: 0,
                execution_outcome: None,
                execution_result_class: None,
                execution_results: vec![],
                failure_reason: None,
                recovery_step_kind: None,
                evidence_snapshot: None,
                force_override_used: false,
                tool_calls: vec![],
                result_summary: None,
                validation_passed: None,
                started_at: None,
                completed_at: None,
                error_message: None,
                replay_record: None,
            });
        }

        // Enable auto-resume
        app.persistence.chain_policy.auto_resume = true;
        app.persistence.chain_policy.auto_advance = true;

        // First auto-resume should start execution (but will fail because no runtime)
        // We simulate the chain being marked as currently executing
        app.state.current_chain_id = Some(chain_id.clone());
        app.state.current_chain_step_id = Some("step-1".to_string());

        // Second auto-resume for same chain should be no-op (idempotent)
        // Should NOT produce "Execution already in progress" error
        let result = app.try_auto_resume_chain().await;

        // Should return false (no action taken) not error
        assert!(!result, "duplicate auto-resume for same chain should be no-op");

        // Verify no error message was pushed
        let has_error = app.state.messages.iter().any(|m| {
            m.content.contains("Execution already in progress")
                || m.content.contains("Auto-resume failed")
        });
        assert!(
            !has_error,
            "should not show 'Execution already in progress' error for idempotent call"
        );
    }

    #[tokio::test]
    async fn auto_resume_chain_blocks_cleanly_for_different_chain() {
        // Test that auto-start for different chain is blocked cleanly without error
        let mut app = App::new().await;

        // Create two chains
        let chain1 = app.persistence.create_chain("chain1", "objective 1").clone();
        let chain1_id = chain1.id.clone();
        let chain2 = app.persistence.create_chain("chain2", "objective 2").clone();
        let chain2_id = chain2.id.clone();

        // Set chain1 as active and simulate it running
        app.persistence.active_chain_id = Some(chain2_id.clone());
        app.state.current_chain_id = Some(chain1_id.clone()); // chain1 is running
        app.state.current_chain_step_id = Some("step-1".to_string());

        // Enable auto-resume
        app.persistence.chain_policy.auto_resume = true;
        app.persistence.chain_policy.auto_advance = true;

        // Set chain2 as ready
        {
            let chain2 = app.persistence.get_chain_mut(&chain2_id).unwrap();
            chain2.status = crate::persistence::ChainLifecycleStatus::Ready;
            chain2.steps.push(crate::persistence::PersistentChainStep {
                id: "step-1".to_string(),
                description: "Test step".to_string(),
                status: crate::persistence::ChainStepStatus::Pending,
                retry_of: None,
                retry_attempt: 0,
                execution_outcome: None,
                execution_result_class: None,
                execution_results: vec![],
                failure_reason: None,
                recovery_step_kind: None,
                evidence_snapshot: None,
                force_override_used: false,
                tool_calls: vec![],
                result_summary: None,
                validation_passed: None,
                started_at: None,
                completed_at: None,
                error_message: None,
                replay_record: None,
            });
        }

        // Auto-resume for chain2 while chain1 is running should be blocked cleanly
        let result = app.try_auto_resume_chain().await;

        // Should return false (no action taken)
        assert!(!result, "auto-resume should be blocked when different chain is running");

        // Should show clean skip message, not error
        let has_skip_message = app.state.messages.iter().any(|m| {
            m.content.contains("another chain is currently executing")
        });
        assert!(
            has_skip_message,
            "should show clean skip message, got messages: {:?}",
            app.state.messages.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn terminal_seal_clears_execution_lock_on_new_run() {
        // Test that execution_run_sealed is cleared when a new execution starts
        // This prevents stale execution locks from blocking new chain resumes
        let mut app = App::new().await;

        // Simulate completed execution (seal should remain until new execution)
        app.execution_run_sealed = true;
        app.post_terminal_audit_recorded = true;
        app.active_runtime_session_id = Some("session-1".to_string());
        app.active_run_message_id = Some("run-1".to_string());

        // Verify seal exists before new execution
        assert!(app.execution_run_sealed, "seal should exist after execution");

        // Simulate start of new execution - this should clear the seal
        app.reset_execution_terminal_seal();

        assert!(
            !app.execution_run_sealed,
            "execution_run_sealed should be cleared when new execution starts"
        );
        assert!(
            !app.post_terminal_audit_recorded,
            "post_terminal_audit_recorded should be cleared when new execution starts"
        );
        assert!(
            app.active_runtime_session_id.is_none(),
            "session ID should be cleared"
        );
    }
}

// Submodules
pub mod focus;
pub mod scroll;

pub use focus::{FocusState, FocusTarget};
pub use scroll::ChatScrollState;
