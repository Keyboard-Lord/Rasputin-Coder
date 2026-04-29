//! Guidance System - Next Action Engine
//!
//! Transforms Rasputin from command-driven to guided operator experience.
//! Every state produces context-aware recommendations.

use crate::persistence::{ChainLifecycleStatus, ExecutionOutcome, PersistentState};
use crate::state::{
    AppState, ArtifactCompletionContract, ArtifactCrudOperation, ArtifactRequirement,
    ObjectiveSatisfaction, RequiredSurface,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Priority level for recommended actions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionPriority {
    /// Primary action - the most likely next step
    Primary,
    /// Secondary action - valid alternative
    Secondary,
    /// Optional action - situational enhancement
    Optional,
}

impl ActionPriority {
    /// Get display label for the priority
    pub fn label(&self) -> &'static str {
        match self {
            ActionPriority::Primary => "PRIMARY",
            ActionPriority::Secondary => "SECONDARY",
            ActionPriority::Optional => "OPTIONAL",
        }
    }

    /// Get color for UI rendering
    pub fn color(&self) -> ratatui::style::Color {
        match self {
            ActionPriority::Primary => ratatui::style::Color::Green,
            ActionPriority::Secondary => ratatui::style::Color::Yellow,
            ActionPriority::Optional => ratatui::style::Color::Gray,
        }
    }
}

/// A recommended next action
#[derive(Debug, Clone)]
pub struct NextAction {
    /// Command to execute
    pub command: String,
    /// Human-readable description
    pub description: String,
    /// Priority level
    pub priority: ActionPriority,
}

impl NextAction {
    /// Create a new next action
    pub fn new(
        command: impl Into<String>,
        description: impl Into<String>,
        priority: ActionPriority,
    ) -> Self {
        Self {
            command: command.into(),
            description: description.into(),
            priority,
        }
    }

    /// Create a primary action
    pub fn primary(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(command, description, ActionPriority::Primary)
    }

    /// Create a secondary action
    pub fn secondary(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(command, description, ActionPriority::Secondary)
    }

    /// Create an optional action
    pub fn optional(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(command, description, ActionPriority::Optional)
    }
}

/// System-wide severity classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Informational - no action needed
    Info,
    /// Warning - attention recommended
    Warning,
    /// Critical - immediate action required
    Critical,
}

impl Severity {
    /// Get display indicator for the severity
    pub fn indicator(&self) -> &'static str {
        match self {
            Severity::Info => "ℹ",
            Severity::Warning => "⚠",
            Severity::Critical => "❌",
        }
    }

    /// Get label for the severity
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Info => "Info",
            Severity::Warning => "Warning",
            Severity::Critical => "Critical",
        }
    }

    /// Get color for UI rendering
    pub fn color(&self) -> ratatui::style::Color {
        match self {
            Severity::Info => ratatui::style::Color::Blue,
            Severity::Warning => ratatui::style::Color::Yellow,
            Severity::Critical => ratatui::style::Color::Red,
        }
    }
}

/// State classification for guidance
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemState {
    /// No active chain or plan
    Idle,
    /// Chain exists but not executing
    Planned,
    /// Chain is actively running
    Running,
    /// Execution blocked (approval needed)
    Blocked,
    /// Execution failed
    Failed,
    /// Execution completed successfully
    Completed,
}

/// The Next Action Engine - produces context-aware recommendations
pub struct NextActionEngine;

impl NextActionEngine {
    /// Analyze current state and suggest next actions
    pub fn suggest(state: &AppState, persistence: &PersistentState) -> Vec<NextAction> {
        let system_state = Self::classify_state(state, persistence);

        match system_state {
            SystemState::Idle => Self::suggestions_for_idle(state, persistence),
            SystemState::Planned => Self::suggestions_for_planned(state, persistence),
            SystemState::Running => Self::suggestions_for_running(state, persistence),
            SystemState::Blocked => Self::suggestions_for_blocked(state, persistence),
            SystemState::Failed => Self::suggestions_for_failed(state, persistence),
            SystemState::Completed => Self::suggestions_for_completed(state, persistence),
        }
    }

    /// Classify current system state
    fn classify_state(_state: &AppState, persistence: &PersistentState) -> SystemState {
        // Check if there's an active chain
        let has_active_chain = persistence.active_chain_id.is_some();
        let has_pending_approval = persistence.has_active_checkpoint();

        if has_pending_approval {
            return SystemState::Blocked;
        }

        if let Some(ref chain_id) = persistence.active_chain_id {
            if let Some(chain) = persistence.get_chain(chain_id) {
                // V1.5 UNIFICATION: Use authoritative ExecutionOutcome when set
                if let Some(outcome) = chain.get_outcome() {
                    match outcome {
                        ExecutionOutcome::Success => return SystemState::Completed,
                        ExecutionOutcome::SuccessWithWarnings => return SystemState::Completed,
                        ExecutionOutcome::Blocked => return SystemState::Blocked,
                        ExecutionOutcome::Failed => return SystemState::Failed,
                    }
                }
                // Fallback to ChainLifecycleStatus for transitional states
                match chain.status {
                    ChainLifecycleStatus::Running => return SystemState::Running,
                    ChainLifecycleStatus::Failed => return SystemState::Failed,
                    ChainLifecycleStatus::Complete => return SystemState::Completed,
                    ChainLifecycleStatus::Halted => return SystemState::Blocked,
                    ChainLifecycleStatus::Draft | ChainLifecycleStatus::Ready => {
                        if !chain.steps.is_empty() {
                            return SystemState::Planned;
                        }
                    }
                    _ => {}
                }
            }
        }

        if has_active_chain {
            SystemState::Planned
        } else {
            SystemState::Idle
        }
    }

    /// Suggestions when no active chain exists
    fn suggestions_for_idle(_state: &AppState, _persistence: &PersistentState) -> Vec<NextAction> {
        vec![
            NextAction::primary("/plan", "Create a task plan to get started"),
            NextAction::secondary("/status", "Check current system status"),
            NextAction::optional("/help", "See available commands"),
        ]
    }

    /// Suggestions when chain is planned but not executing
    fn suggestions_for_planned(
        _state: &AppState,
        persistence: &PersistentState,
    ) -> Vec<NextAction> {
        let mut suggestions = vec![];

        // Check if we have steps to execute
        if let Some(ref chain_id) = persistence.active_chain_id {
            if let Some(chain) = persistence.get_chain(chain_id) {
                if !chain.steps.is_empty() {
                    suggestions.push(NextAction::primary(
                        "/chain resume",
                        "Start executing the planned chain",
                    ));
                } else {
                    suggestions.push(NextAction::primary("/plan", "Add steps to the chain"));
                }
            }
        }

        suggestions.push(NextAction::secondary(
            "/plan context",
            "Add context files for better results",
        ));
        suggestions.push(NextAction::optional(
            "/git status",
            "Inspect repository state",
        ));

        suggestions
    }

    /// Suggestions when chain is actively running
    fn suggestions_for_running(
        _state: &AppState,
        _persistence: &PersistentState,
    ) -> Vec<NextAction> {
        vec![
            NextAction::primary("/chain status", "Monitor execution progress"),
            NextAction::secondary("/replay", "View replay of current execution"),
            NextAction::optional("/status", "Check detailed system status"),
        ]
    }

    /// Suggestions when execution is blocked
    fn suggestions_for_blocked(
        _state: &AppState,
        persistence: &PersistentState,
    ) -> Vec<NextAction> {
        let mut suggestions = vec![];

        // Check if there's a checkpoint requiring approval
        if persistence.has_active_checkpoint() {
            suggestions.push(NextAction::primary(
                "/approve",
                "Approve the pending checkpoint and continue",
            ));
            suggestions.push(NextAction::secondary(
                "/deny",
                "Deny the pending checkpoint",
            ));
            suggestions.push(NextAction::optional(
                "/chain status",
                "Review checkpoint details",
            ));
        } else {
            suggestions.push(NextAction::primary(
                "/chain status",
                "Check why execution is blocked",
            ));
            suggestions.push(NextAction::secondary("/status", "Review system state"));
        }

        suggestions
    }

    /// Suggestions when execution has failed
    fn suggestions_for_failed(_state: &AppState, persistence: &PersistentState) -> Vec<NextAction> {
        let mut suggestions = vec![
            NextAction::primary("/replay", "Inspect what went wrong"),
            NextAction::secondary("/chain status", "Review failure details"),
        ];

        // Add step-specific suggestions if we have an active chain
        if let Some(ref chain_id) = persistence.active_chain_id {
            if let Some(chain) = persistence.get_chain(chain_id) {
                if let Some(active_step) = chain.active_step {
                    suggestions.push(NextAction::optional(
                        &format!("/replay diff {}", active_step),
                        &format!("Compare step {} for divergence", active_step),
                    ));
                }
            }
        }

        suggestions
    }

    /// Suggestions when execution has completed
    fn suggestions_for_completed(
        _state: &AppState,
        _persistence: &PersistentState,
    ) -> Vec<NextAction> {
        vec![
            NextAction::primary("/replay", "Verify execution determinism"),
            NextAction::secondary("/plan", "Start a new task"),
            NextAction::optional("/chain archive", "Archive completed chain"),
        ]
    }

    /// Get a single primary suggestion for inline display
    pub fn primary_suggestion(
        state: &AppState,
        persistence: &PersistentState,
    ) -> Option<NextAction> {
        Self::suggest(state, persistence)
            .into_iter()
            .find(|a| a.priority == ActionPriority::Primary)
    }

    /// Format suggestions for display
    pub fn format_suggestions(actions: &[NextAction]) -> String {
        if actions.is_empty() {
            return String::new();
        }

        let mut lines = vec!["→ Recommended Next Actions:".to_string()];

        for action in actions {
            lines.push(format!(
                "  [{}] {:12} {}",
                action.priority.label(),
                action.command,
                action.description
            ));
        }

        lines.join("\n")
    }

    /// Format a single inline suggestion
    pub fn format_inline(state: &AppState, persistence: &PersistentState) -> String {
        if let Some(action) = Self::primary_suggestion(state, persistence) {
            format!("→ Next: {} {}", action.command, action.description)
        } else {
            String::new()
        }
    }
}

/// System narrative generator - converts raw outputs into explanations
pub struct SystemNarrative;

impl SystemNarrative {
    /// Generate narrative for blocked execution
    pub fn blocked(reason: &str, impact: &str, actions: &[(&str, &str)]) -> String {
        let mut narrative = format!("Execution paused.\n\n");
        narrative.push_str(&format!("Reason: {}\n", reason));
        narrative.push_str(&format!("Impact: {}\n\n", impact));
        narrative.push_str("→ Next:\n");

        for (command, description) in actions {
            narrative.push_str(&format!("  {:12} {}\n", command, description));
        }

        narrative
    }

    /// Generate narrative for failed execution
    pub fn failed(reason: &str, impact: &str, actions: &[(&str, &str)]) -> String {
        let mut narrative = format!("Execution failed.\n\n");
        narrative.push_str(&format!("Reason: {}\n", reason));
        narrative.push_str(&format!("Impact: {}\n\n", impact));
        narrative.push_str("→ Next:\n");

        for (command, description) in actions {
            narrative.push_str(&format!("  {:12} {}\n", command, description));
        }

        narrative
    }

    /// Generate narrative for completed execution
    pub fn completed(summary: &str, actions: &[(&str, &str)]) -> String {
        let mut narrative = format!("{}\n\n", summary);
        narrative.push_str("→ Next:\n");

        for (command, description) in actions {
            narrative.push_str(&format!("  {:12} {}\n", command, description));
        }

        narrative
    }

    /// Generate idle state guidance
    pub fn idle() -> String {
        "No active chain.\n\n".to_string()
            + "→ Start with:\n"
            + "  /plan       Create a task plan\n"
            + "  /help       See all commands"
    }
}

/// Replay confidence classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayConfidence {
    /// Full match
    Exact,
    /// Timestamp or ordering only
    TrivialDeviation,
    /// Small tool difference
    MinorDeviation,
    /// Missing mutation
    MajorDeviation,
    /// Outcome mismatch
    CriticalFailure,
}

impl ReplayConfidence {
    /// Get display label
    pub fn label(&self) -> &'static str {
        match self {
            ReplayConfidence::Exact => "Exact",
            ReplayConfidence::TrivialDeviation => "Trivial Deviation",
            ReplayConfidence::MinorDeviation => "Minor Deviation",
            ReplayConfidence::MajorDeviation => "Major Deviation",
            ReplayConfidence::CriticalFailure => "Critical Failure",
        }
    }

    /// Get severity level
    pub fn severity(&self) -> Severity {
        match self {
            ReplayConfidence::Exact => Severity::Info,
            ReplayConfidence::TrivialDeviation => Severity::Info,
            ReplayConfidence::MinorDeviation => Severity::Warning,
            ReplayConfidence::MajorDeviation => Severity::Warning,
            ReplayConfidence::CriticalFailure => Severity::Critical,
        }
    }

    /// Get indicator with severity
    pub fn indicator_with_severity(&self) -> String {
        format!("{} {}", self.severity().indicator(), self.label())
    }
}

/// Compute replay confidence based on comparison results
pub fn compute_replay_confidence(
    fingerprint_match: bool,
    tool_calls_match: bool,
    mutations_match: bool,
    outcome_match: bool,
) -> ReplayConfidence {
    if fingerprint_match && tool_calls_match && mutations_match && outcome_match {
        ReplayConfidence::Exact
    } else if outcome_match && mutations_match && !tool_calls_match {
        // Small tool differences (maybe ordering)
        ReplayConfidence::TrivialDeviation
    } else if outcome_match && !mutations_match {
        // Missing some mutations but outcome OK
        ReplayConfidence::MinorDeviation
    } else if !outcome_match && mutations_match {
        // Different outcome but mutations match
        ReplayConfidence::MajorDeviation
    } else {
        // Complete mismatch
        ReplayConfidence::CriticalFailure
    }
}

// ============================================================================
// V1.3: Assisted Execution + Intent Awareness
// ============================================================================

/// Confidence level for automated action execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionConfidence {
    /// High confidence - safe to suggest auto-execution
    High,
    /// Medium confidence - ask for confirmation
    Medium,
    /// Low confidence - require explicit command
    Low,
}

impl ActionConfidence {
    /// Get display label
    pub fn label(&self) -> &'static str {
        match self {
            ActionConfidence::High => "High",
            ActionConfidence::Medium => "Medium",
            ActionConfidence::Low => "Low",
        }
    }

    /// Determine if this confidence level permits auto-suggestion
    pub fn permits_auto(&self) -> bool {
        matches!(self, ActionConfidence::High)
    }

    /// Determine if this confidence level requires confirmation
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, ActionConfidence::Medium)
    }
}

/// Intent memory - tracks operator goals for contextual guidance
#[derive(Debug, Clone, Default)]
pub struct OperatorIntent {
    /// Last stated goal/objective
    pub last_goal: Option<String>,
    /// Last action taken
    pub last_action: Option<String>,
    /// Current workflow stage
    pub workflow_stage: WorkflowStage,
    /// When the intent was recorded
    pub recorded_at: Option<chrono::DateTime<chrono::Local>>,
}

/// Workflow stage for intent tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkflowStage {
    /// No active workflow
    #[default]
    Idle,
    /// Planning a task
    Planning,
    /// Executing a plan
    Executing,
    /// Reviewing results
    Reviewing,
    /// Debugging failures
    Debugging,
}

impl OperatorIntent {
    /// Create new intent memory
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a goal
    pub fn record_goal(&mut self, goal: impl Into<String>) {
        self.last_goal = Some(goal.into());
        self.recorded_at = Some(chrono::Local::now());
    }

    /// Record an action
    pub fn record_action(&mut self, action: impl Into<String>) {
        self.last_action = Some(action.into());
        self.recorded_at = Some(chrono::Local::now());
    }

    /// Set workflow stage
    pub fn set_stage(&mut self, stage: WorkflowStage) {
        self.workflow_stage = stage;
    }

    /// Check if we're in a specific workflow
    pub fn is_planning(&self) -> bool {
        self.workflow_stage == WorkflowStage::Planning
    }

    /// Check if we're in execution phase
    pub fn is_executing(&self) -> bool {
        self.workflow_stage == WorkflowStage::Executing
    }

    /// Get contextual suggestion bias based on intent
    pub fn suggestion_bias(&self) -> SuggestionBias {
        match self.workflow_stage {
            WorkflowStage::Planning => SuggestionBias::TowardExecution,
            WorkflowStage::Executing => SuggestionBias::TowardMonitoring,
            WorkflowStage::Debugging => SuggestionBias::TowardInspection,
            _ => SuggestionBias::Neutral,
        }
    }
}

/// Suggestion bias for contextual guidance
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionBias {
    /// No specific bias
    Neutral,
    /// Bias toward execution commands
    TowardExecution,
    /// Bias toward monitoring/status commands
    TowardMonitoring,
    /// Bias toward inspection/debug commands
    TowardInspection,
}

/// Confirmation UX - presents prepared actions for operator confirmation
#[derive(Debug, Clone)]
pub struct ConfirmationRequest {
    /// The prepared command
    pub command: String,
    /// Why this action is suggested
    pub reason: String,
    /// What will happen
    pub impact: String,
    /// Confidence level
    pub confidence: ActionConfidence,
    /// Alternative actions
    pub alternatives: Vec<NextAction>,
}

impl ConfirmationRequest {
    /// Create a new confirmation request
    pub fn new(
        command: impl Into<String>,
        reason: impl Into<String>,
        impact: impl Into<String>,
        confidence: ActionConfidence,
    ) -> Self {
        Self {
            command: command.into(),
            reason: reason.into(),
            impact: impact.into(),
            confidence,
            alternatives: vec![],
        }
    }

    /// Format for display
    pub fn format(&self) -> String {
        let mut text = format!(
            "→ About to run: {}\nReason: {}\nImpact: {}",
            self.command, self.reason, self.impact
        );

        if self.confidence.requires_confirmation() {
            text.push_str("\n\nPress Enter to confirm, or type a command to cancel");
        }

        if !self.alternatives.is_empty() {
            text.push_str("\n\nAlternatives:");
            for alt in &self.alternatives {
                text.push_str(&format!("\n  {} - {}", alt.command, alt.description));
            }
        }

        text
    }
}

/// Flow mode - aggressive suggestion mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowMode {
    /// Standard guidance mode
    Standard,
    /// Aggressive suggestion mode
    Active,
}

impl FlowMode {
    /// Check if flow mode is active
    pub fn is_active(&self) -> bool {
        matches!(self, FlowMode::Active)
    }
}

/// Confidence-based behavior decision matrix
pub struct BehaviorMatrix;

impl BehaviorMatrix {
    /// Determine behavior based on confidence, severity, and state
    pub fn decide(
        confidence: ActionConfidence,
        severity: Severity,
        has_explicit_command: bool,
    ) -> BehaviorDecision {
        // Critical severity always requires explicit command
        if severity == Severity::Critical {
            return BehaviorDecision::RequireExplicit;
        }

        // If user typed something explicit, respect it
        if has_explicit_command {
            return BehaviorDecision::ExecuteExplicit;
        }

        match confidence {
            ActionConfidence::High => BehaviorDecision::AutoSuggest,
            ActionConfidence::Medium => BehaviorDecision::Confirm,
            ActionConfidence::Low => BehaviorDecision::RequireExplicit,
        }
    }
}

/// Behavior decision outcomes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviorDecision {
    /// Auto-suggest the action (high confidence, low risk)
    AutoSuggest,
    /// Show confirmation prompt (medium confidence)
    Confirm,
    /// Require explicit command entry (low confidence or critical)
    RequireExplicit,
    /// Execute what user explicitly typed
    ExecuteExplicit,
}

/// Assisted Execution Engine - V1.3 core system
pub struct AssistedExecution;

impl AssistedExecution {
    /// Prepare next action with confidence assessment
    pub fn prepare_next_action(
        state: &AppState,
        persistence: &PersistentState,
        intent: &OperatorIntent,
        flow_mode: FlowMode,
    ) -> Option<ConfirmationRequest> {
        let actions = NextActionEngine::suggest(state, persistence);
        let primary = actions.first()?.clone();

        // Assess confidence based on state and intent
        let confidence = Self::assess_confidence(state, persistence, intent, &primary.command);

        // In flow mode, bias toward auto-suggestion
        let confidence = if flow_mode.is_active() && confidence == ActionConfidence::Medium {
            ActionConfidence::High // Upgrade medium to high in flow mode
        } else {
            confidence
        };

        // Build alternatives from remaining suggestions
        let alternatives: Vec<NextAction> = actions.into_iter().skip(1).take(2).collect();

        // Determine reason and impact based on state
        let (reason, impact) = Self::describe_action(&primary.command, state, persistence);

        Some(ConfirmationRequest {
            command: primary.command,
            reason,
            impact,
            confidence,
            alternatives,
        })
    }

    /// Assess confidence for a given action
    fn assess_confidence(
        state: &AppState,
        persistence: &PersistentState,
        intent: &OperatorIntent,
        command: &str,
    ) -> ActionConfidence {
        // Check for blocking conditions - always low confidence
        if persistence.has_active_checkpoint()
            || state.execution.state == crate::state::ExecutionState::Blocked
        {
            return ActionConfidence::Low; // Requires explicit approval
        }

        // Check chain status
        if let Some(ref chain_id) = persistence.active_chain_id {
            if let Some(chain) = persistence.get_chain(chain_id) {
                match chain.status {
                    ChainLifecycleStatus::Failed => return ActionConfidence::Low, // Needs investigation
                    ChainLifecycleStatus::Halted => return ActionConfidence::Low, // Needs decision
                    ChainLifecycleStatus::Complete => {
                        // After completion, /replay is high confidence
                        if command.starts_with("/replay") {
                            return ActionConfidence::High;
                        }
                        return ActionConfidence::Medium;
                    }
                    ChainLifecycleStatus::Draft | ChainLifecycleStatus::Ready => {
                        if command.starts_with("/chain resume") {
                            return ActionConfidence::High;
                        }
                        return ActionConfidence::Medium;
                    }
                    _ => {}
                }
            }
        }

        // Intent-based confidence boost
        if intent.is_planning() && command.starts_with("/plan") {
            return ActionConfidence::High;
        }
        if intent.is_executing() && command.starts_with("/chain resume") {
            return ActionConfidence::High;
        }

        // Default: medium confidence for primary actions
        ActionConfidence::Medium
    }

    /// Describe what an action will do
    fn describe_action(
        command: &str,
        _state: &AppState,
        persistence: &PersistentState,
    ) -> (String, String) {
        if command.starts_with("/chain resume") {
            let step_count = persistence
                .active_chain_id
                .as_ref()
                .and_then(|id| persistence.get_chain(id))
                .map(|c| c.steps.len())
                .unwrap_or(0);
            (
                "Plan is ready to execute".to_string(),
                format!("Will run {} steps", step_count),
            )
        } else if command.starts_with("/approve") {
            (
                "Approval required to continue".to_string(),
                "Will execute the pending checkpoint".to_string(),
            )
        } else if command.starts_with("/replay") {
            (
                "Verify execution determinism".to_string(),
                "Will compare recorded vs actual execution".to_string(),
            )
        } else if command.starts_with("/plan") {
            (
                "Create a task plan".to_string(),
                "Will generate steps to achieve objective".to_string(),
            )
        } else {
            (
                "Recommended next step".to_string(),
                "Will advance the current workflow".to_string(),
            )
        }
    }

    /// Format flow mode status line
    pub fn flow_mode_indicator(flow_mode: FlowMode) -> Option<String> {
        if flow_mode.is_active() {
            Some("[FLOW MODE] ".to_string())
        } else {
            None
        }
    }
}

// ============================================================================
// V1.4: Multi-Step Lookahead + Risk Forecasting + Session Memory
// ============================================================================

/// Preview of upcoming execution steps
#[derive(Debug, Clone)]
pub struct ExecutionPreview {
    /// Steps that will execute
    pub upcoming_steps: Vec<StepPreview>,
    /// Detected risks
    pub risks: Vec<Risk>,
    /// Predicted outcome
    pub estimated_outcome: OutcomePrediction,
    /// Whether auto-chaining is safe
    pub safe_to_chain: bool,
    /// Number of approvals that will be required
    pub approvals_needed: usize,
}

/// Preview of a single upcoming step
#[derive(Debug, Clone)]
pub struct StepPreview {
    /// Step number (1-indexed)
    pub step_number: usize,
    /// Description of what will happen
    pub description: String,
    /// Action type
    pub action_type: StepActionType,
    /// Risk level for this step
    pub risk_level: RiskLevel,
    /// Whether this step requires approval
    pub requires_approval: bool,
}

/// Categorized action types for preview
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepActionType {
    Read,
    Write,
    Execute,
    Validate,
    Commit,
    External,
}

impl StepActionType {
    /// Get icon for the action type
    pub fn icon(&self) -> &'static str {
        match self {
            StepActionType::Read => "📄",
            StepActionType::Write => "📝",
            StepActionType::Execute => "⚡",
            StepActionType::Validate => "✓",
            StepActionType::Commit => "⎇",
            StepActionType::External => "🔌",
        }
    }

    /// Get description
    pub fn description(&self) -> &'static str {
        match self {
            StepActionType::Read => "Read file",
            StepActionType::Write => "Modify file",
            StepActionType::Execute => "Execute command",
            StepActionType::Validate => "Validate result",
            StepActionType::Commit => "Commit changes",
            StepActionType::External => "External action",
        }
    }
}

/// Risk level classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    /// No risk detected
    Safe,
    /// Minor concern, proceed with awareness
    Caution,
    /// Significant risk, requires attention
    Warning,
    /// High risk, explicit approval recommended
    Critical,
}

impl RiskLevel {
    /// Get indicator icon
    pub fn icon(&self) -> &'static str {
        match self {
            RiskLevel::Safe => "✓",
            RiskLevel::Caution => "⚡",
            RiskLevel::Warning => "⚠",
            RiskLevel::Critical => "❌",
        }
    }

    /// Get severity mapping
    pub fn to_severity(&self) -> Severity {
        match self {
            RiskLevel::Safe => Severity::Info,
            RiskLevel::Caution => Severity::Info,
            RiskLevel::Warning => Severity::Warning,
            RiskLevel::Critical => Severity::Critical,
        }
    }
}

/// Detected risk with context
#[derive(Debug, Clone)]
pub struct Risk {
    /// Type of risk
    pub risk_type: RiskType,
    /// Description of the risk
    pub description: String,
    /// Affected files/resources
    pub affected: Vec<String>,
    /// Suggested mitigation
    pub mitigation: String,
    /// Risk level
    pub level: RiskLevel,
}

/// Types of risks that can be forecast
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskType {
    /// Git conflicts (uncommitted changes)
    GitConflict,
    /// Validation might fail
    ValidationFailure,
    /// Missing context information
    MissingContext,
    /// Will require approval
    ApprovalRequired,
    /// File modification without backup
    UnprotectedWrite,
    /// External dependency issue
    ExternalDependency,
    /// Execution mode limitation
    ModeLimitation,
}

impl RiskType {
    /// Get display name
    pub fn name(&self) -> &'static str {
        match self {
            RiskType::GitConflict => "Git Conflict",
            RiskType::ValidationFailure => "Validation Risk",
            RiskType::MissingContext => "Missing Context",
            RiskType::ApprovalRequired => "Approval Required",
            RiskType::UnprotectedWrite => "Unprotected Write",
            RiskType::ExternalDependency => "External Dependency",
            RiskType::ModeLimitation => "Mode Limitation",
        }
    }
}

/// Predicted execution outcome
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomePrediction {
    /// Expected to succeed
    Success,
    /// Expected to succeed with warnings
    SuccessWithWarnings,
    /// May fail
    Uncertain,
    /// Expected to fail
    FailureLikely,
}

impl OutcomePrediction {
    /// Get display description
    pub fn description(&self) -> &'static str {
        match self {
            OutcomePrediction::Success => "Success expected",
            OutcomePrediction::SuccessWithWarnings => "Success (with warnings)",
            OutcomePrediction::Uncertain => "Outcome uncertain",
            OutcomePrediction::FailureLikely => "Failure likely",
        }
    }

    /// Get indicator
    pub fn indicator(&self) -> &'static str {
        match self {
            OutcomePrediction::Success => "✓",
            OutcomePrediction::SuccessWithWarnings => "⚡",
            OutcomePrediction::Uncertain => "?",
            OutcomePrediction::FailureLikely => "✗",
        }
    }
}

/// Multi-step lookahead engine
pub struct LookaheadEngine;

impl LookaheadEngine {
    /// Generate preview of upcoming execution
    pub fn preview_execution(
        persistence: &PersistentState,
        state: &AppState,
    ) -> Option<ExecutionPreview> {
        let chain = persistence.get_active_chain()?;
        let remaining_steps: Vec<_> = chain
            .steps
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.status, crate::persistence::ChainStepStatus::Pending))
            .map(|(i, s)| (i, s))
            .collect();

        if remaining_steps.is_empty() {
            return None;
        }

        let mut upcoming = Vec::new();
        let mut risks = Vec::new();
        let mut approvals_needed = 0;

        // Analyze each upcoming step
        for (idx, step) in remaining_steps.iter().take(5) {
            let action_type = Self::classify_action(step);
            let risk_level = Self::assess_step_risk(step, persistence, state);
            let requires_approval =
                risk_level == RiskLevel::Warning || risk_level == RiskLevel::Critical;

            if requires_approval {
                approvals_needed += 1;
            }

            upcoming.push(StepPreview {
                step_number: idx + 1,
                description: step.description.clone(),
                action_type,
                risk_level,
                requires_approval,
            });
        }

        // Detect workflow-level risks
        risks.extend(Self::detect_workflow_risks(persistence, state, &upcoming));

        // Calculate outcome prediction
        let estimated_outcome = Self::predict_outcome(&upcoming, &risks);

        // Determine if safe to auto-chain
        let safe_to_chain = risks.is_empty()
            && approvals_needed == 0
            && matches!(
                estimated_outcome,
                OutcomePrediction::Success | OutcomePrediction::SuccessWithWarnings
            );

        Some(ExecutionPreview {
            upcoming_steps: upcoming,
            risks,
            estimated_outcome,
            safe_to_chain,
            approvals_needed,
        })
    }

    /// Classify step action type from description and tool calls
    fn classify_action(step: &crate::persistence::PersistentChainStep) -> StepActionType {
        let desc_lower = step.description.to_lowercase();

        // Infer from description keywords
        if desc_lower.contains("read") || desc_lower.contains("view") {
            StepActionType::Read
        } else if desc_lower.contains("write")
            || desc_lower.contains("edit")
            || desc_lower.contains("modify")
        {
            StepActionType::Write
        } else if desc_lower.contains("run")
            || desc_lower.contains("execute")
            || desc_lower.contains("command")
        {
            StepActionType::Execute
        } else if desc_lower.contains("validate") || desc_lower.contains("test") {
            StepActionType::Validate
        } else if desc_lower.contains("commit") || desc_lower.contains("git") {
            StepActionType::Commit
        } else {
            StepActionType::External
        }
    }

    /// Assess risk for a single step
    fn assess_step_risk(
        step: &crate::persistence::PersistentChainStep,
        _persistence: &PersistentState,
        state: &AppState,
    ) -> RiskLevel {
        let action_type = Self::classify_action(step);
        let desc_lower = step.description.to_lowercase();

        match action_type {
            StepActionType::Write => {
                // Check for uncommitted changes
                if has_uncommitted_changes(state) {
                    RiskLevel::Warning
                } else {
                    RiskLevel::Caution
                }
            }
            StepActionType::Execute => {
                // Commands always carry some risk
                // Higher risk for destructive commands
                if desc_lower.contains("delete")
                    || desc_lower.contains("remove")
                    || desc_lower.contains("drop")
                {
                    RiskLevel::Warning
                } else {
                    RiskLevel::Caution
                }
            }
            StepActionType::Commit => {
                // Commit/push requires explicit approval
                RiskLevel::Critical
            }
            _ => RiskLevel::Safe,
        }
    }

    /// Detect workflow-level risks
    fn detect_workflow_risks(
        persistence: &PersistentState,
        state: &AppState,
        upcoming: &[StepPreview],
    ) -> Vec<Risk> {
        let mut risks = Vec::new();

        // Check for git conflicts
        if has_uncommitted_changes(state) {
            let write_steps: Vec<_> = upcoming
                .iter()
                .filter(|s| s.action_type == StepActionType::Write)
                .map(|s| s.step_number)
                .collect();

            if !write_steps.is_empty() {
                risks.push(Risk {
                    risk_type: RiskType::GitConflict,
                    description: "Uncommitted changes may conflict with planned modifications"
                        .to_string(),
                    affected: vec!["working directory".to_string()],
                    mitigation:
                        "/git status to review, /stash or /commit to save, or /override to proceed"
                            .to_string(),
                    level: RiskLevel::Critical, // V1.5: Block execution - requires explicit override
                });
            }
        }

        // Check for missing context
        if let Some(chain) = persistence.get_active_chain() {
            if chain.selected_context_files.is_empty() && !chain.steps.is_empty() {
                risks.push(Risk {
                    risk_type: RiskType::MissingContext,
                    description: "No context files selected - plan may lack required information"
                        .to_string(),
                    affected: vec!["context assembly".to_string()],
                    mitigation: "/plan context to add files".to_string(),
                    level: RiskLevel::Caution,
                });
            }
        }

        // Check execution mode limitations
        let blocked_actions: Vec<_> = upcoming
            .iter()
            .filter(|s| {
                let action = match s.action_type {
                    StepActionType::Write => crate::state::StepAction::WriteFile {
                        path: String::new(),
                    },
                    StepActionType::Execute => crate::state::StepAction::RunCommand {
                        command: String::new(),
                    },
                    _ => crate::state::StepAction::None,
                };
                let (allowed, _) = state.execution.mode.check_action(&action);
                !allowed
            })
            .collect();

        if !blocked_actions.is_empty() {
            risks.push(Risk {
                risk_type: RiskType::ModeLimitation,
                description: format!(
                    "{} step(s) blocked by current execution mode",
                    blocked_actions.len()
                ),
                affected: vec!["execution mode".to_string()],
                mitigation: format!(
                    "Switch to {} mode",
                    if state.execution.mode == crate::state::ExecutionMode::Chat {
                        "EDIT or TASK".to_string()
                    } else {
                        "TASK".to_string()
                    }
                ),
                level: RiskLevel::Critical,
            });
        }

        risks
    }

    /// Predict overall outcome
    fn predict_outcome(upcoming: &[StepPreview], risks: &[Risk]) -> OutcomePrediction {
        let critical_count = risks
            .iter()
            .filter(|r| r.level == RiskLevel::Critical)
            .count();
        let warning_count = risks
            .iter()
            .filter(|r| r.level == RiskLevel::Warning)
            .count();

        if critical_count > 0 {
            return OutcomePrediction::FailureLikely;
        }

        if warning_count > 0 {
            return OutcomePrediction::SuccessWithWarnings;
        }

        let caution_count = upcoming
            .iter()
            .filter(|s| s.risk_level == RiskLevel::Caution)
            .count();
        if caution_count > 2 {
            return OutcomePrediction::SuccessWithWarnings;
        }

        OutcomePrediction::Success
    }

    /// Format preview for display
    pub fn format_preview(preview: &ExecutionPreview) -> String {
        let mut lines = vec!["→ Upcoming Execution Plan:".to_string()];

        for step in &preview.upcoming_steps {
            let icon = step.action_type.icon();
            let risk_icon = step.risk_level.icon();
            let approval = if step.requires_approval {
                " [APPROVAL]"
            } else {
                ""
            };
            lines.push(format!(
                "  {} {}. {} {}{}",
                risk_icon, step.step_number, icon, step.description, approval
            ));
        }

        if !preview.risks.is_empty() {
            lines.push(String::new());
            lines.push("⚠ Risks Detected:".to_string());
            for risk in &preview.risks {
                lines.push(format!(
                    "  {} {}: {}",
                    risk.level.icon(),
                    risk.risk_type.name(),
                    risk.description
                ));
                lines.push(format!("    → {}", risk.mitigation));
            }
        }

        lines.push(String::new());
        lines.push(format!(
            "→ Estimated Outcome: {} {}",
            preview.estimated_outcome.indicator(),
            preview.estimated_outcome.description()
        ));

        if preview.approvals_needed > 0 {
            lines.push(format!(
                "→ {} approval(s) will be required",
                preview.approvals_needed
            ));
        }

        if preview.safe_to_chain {
            lines.push(
                "→ Safe to auto-chain: All steps can execute without interruption".to_string(),
            );
        }

        lines.join("\n")
    }
}

/// Check for uncommitted git changes (helper)
/// Returns true if there are modified, staged, or untracked files
fn has_uncommitted_changes(state: &AppState) -> bool {
    // Check via git grounding if path available
    if !state.repo.path.is_empty() {
        let grounding = crate::repo::capture_git_grounding(&state.repo.path);
        return grounding.is_dirty
            || !grounding.modified_files.is_empty()
            || !grounding.staged_files.is_empty();
    }
    false
}

/// Enhanced explanation system - explains "why" decisions were made
pub struct ExplanationEngine;

impl ExplanationEngine {
    /// Generate detailed explanation for a recommended action
    pub fn explain_action(
        command: &str,
        state: &AppState,
        persistence: &PersistentState,
        confidence: ActionConfidence,
    ) -> String {
        let mut explanation = format!("→ About to run: {}\n\n", command);

        // System logic section
        explanation.push_str("Why:\n");
        explanation.push_str(&Self::explain_why(command, state, persistence));
        explanation.push('\n');

        // Confidence explanation
        explanation.push_str(&format!("Confidence: {}\n", confidence.label()));
        explanation.push_str(&Self::explain_confidence(
            command,
            state,
            persistence,
            confidence,
        ));
        explanation.push('\n');

        // Risk level
        let risk_level = Self::assess_risk_level(command, state, persistence);
        explanation.push_str(&format!(
            "Risk Level: {}\n",
            Self::risk_description(risk_level)
        ));

        // Contextual suggestions
        if let Some(preview) = LookaheadEngine::preview_execution(persistence, state) {
            explanation.push('\n');
            explanation.push_str(&Self::explain_context(&preview));
        }

        explanation
    }

    /// Explain why this action is suggested
    fn explain_why(command: &str, _state: &AppState, persistence: &PersistentState) -> String {
        if command.starts_with("/chain resume") {
            if let Some(chain) = persistence.get_active_chain() {
                let pending = chain
                    .steps
                    .iter()
                    .filter(|s| matches!(s.status, crate::persistence::ChainStepStatus::Pending))
                    .count();
                return format!(
                    "  • A plan exists with {} pending step(s)\n  • Execution has not been started\n  • Chain status: {:?}",
                    pending, chain.status
                );
            }
            return "  • No chain information available".to_string();
        }

        if command.starts_with("/approve") {
            return "  • A checkpoint is pending approval\n  • High-risk action requires explicit confirmation".to_string();
        }

        if command.starts_with("/replay") {
            return "  • Execution has completed\n  • Replay verifies determinism by comparing recorded vs actual".to_string();
        }

        if command.starts_with("/plan") {
            return "  • No active execution plan\n  • User intent suggests starting a new task"
                .to_string();
        }

        "  • Based on current system state".to_string()
    }

    /// Explain confidence assessment
    fn explain_confidence(
        _command: &str,
        state: &AppState,
        persistence: &PersistentState,
        confidence: ActionConfidence,
    ) -> String {
        let mut reasons = vec![];

        match confidence {
            ActionConfidence::High => {
                reasons.push("✓ Clear path forward".to_string());
                if !persistence.has_active_checkpoint() {
                    reasons.push("✓ No blocking conditions".to_string());
                }
            }
            ActionConfidence::Medium => {
                reasons.push("⚡ Proceed with awareness".to_string());
                if persistence.has_active_checkpoint() {
                    reasons.push("⚡ Approval checkpoint exists".to_string());
                }
            }
            ActionConfidence::Low => {
                reasons.push("❓ Uncertainty detected".to_string());
                if let Some(chain) = persistence.get_active_chain() {
                    // V1.5 UNIFICATION: Use ExecutionOutcome when set
                    let is_failed = chain
                        .get_outcome()
                        .map(|o| o == ExecutionOutcome::Failed)
                        .unwrap_or_else(|| {
                            matches!(
                                chain.status,
                                crate::persistence::ChainLifecycleStatus::Failed
                            )
                        });
                    if is_failed {
                        reasons.push("❌ Previous execution failed".to_string());
                    }
                }
            }
        }

        // Check mode compatibility
        if state.execution.state == crate::state::ExecutionState::Blocked {
            reasons.push("⚠ Execution is blocked".to_string());
        }

        reasons.join("\n")
    }

    /// Assess risk level for an action
    fn assess_risk_level(
        command: &str,
        state: &AppState,
        persistence: &PersistentState,
    ) -> RiskLevel {
        if command.starts_with("/approve") || command.starts_with("/chain resume") {
            if persistence.has_active_checkpoint() {
                return RiskLevel::Warning;
            }
        }

        if command.starts_with("/replay") {
            return RiskLevel::Safe;
        }

        if let Some(preview) = LookaheadEngine::preview_execution(persistence, state) {
            if !preview.risks.is_empty() {
                return preview.risks[0].level;
            }
        }

        RiskLevel::Safe
    }

    /// Get risk description
    fn risk_description(level: RiskLevel) -> &'static str {
        match level {
            RiskLevel::Safe => "Low - Safe to proceed",
            RiskLevel::Caution => "Medium - Minor concerns",
            RiskLevel::Warning => "Elevated - Attention needed",
            RiskLevel::Critical => "High - Requires careful review",
        }
    }

    /// Explain execution context
    fn explain_context(preview: &ExecutionPreview) -> String {
        let mut context = "Context:\n".to_string();

        if preview.upcoming_steps.len() > 1 {
            context.push_str(&format!(
                "  • This is step 1 of {} upcoming steps\n",
                preview.upcoming_steps.len()
            ));
        }

        if preview.safe_to_chain {
            context.push_str("  • All steps can execute without interruption\n");
        } else if preview.approvals_needed > 0 {
            context.push_str(&format!(
                "  • {} checkpoint(s) will require your approval\n",
                preview.approvals_needed
            ));
        }

        context
    }
}

/// Session memory - tracks patterns for adaptive behavior
#[derive(Debug, Clone, Default)]
pub struct SessionMemory {
    /// Commands used frequently
    pub command_frequency: std::collections::HashMap<String, u32>,
    /// Common workflow patterns detected
    pub workflow_patterns: Vec<WorkflowPattern>,
    /// User overrides of suggestions
    pub override_count: u32,
    /// Successful completions without issues
    pub clean_completions: u32,
    /// Session start time
    pub session_started: Option<chrono::DateTime<chrono::Local>>,
}

/// Detected workflow pattern
#[derive(Debug, Clone)]
pub struct WorkflowPattern {
    /// Pattern name
    pub name: String,
    /// Commands in the pattern
    pub commands: Vec<String>,
    /// How many times observed
    pub observed_count: u32,
    /// Last observed
    pub last_observed: chrono::DateTime<chrono::Local>,
}

impl SessionMemory {
    /// Create new session memory
    pub fn new() -> Self {
        Self {
            session_started: Some(chrono::Local::now()),
            ..Default::default()
        }
    }

    /// Record command usage
    pub fn record_command(&mut self, command: &str) {
        *self
            .command_frequency
            .entry(command.to_string())
            .or_insert(0) += 1;
    }

    /// Record user override of suggestion
    pub fn record_override(&mut self) {
        self.override_count += 1;
    }

    /// Record clean completion
    pub fn record_completion(&mut self) {
        self.clean_completions += 1;
    }

    /// Get most frequent command
    pub fn favorite_command(&self) -> Option<&String> {
        self.command_frequency
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(cmd, _)| cmd)
    }

    /// Check if user frequently runs a command after another
    pub fn predicts_next(&self, after_command: &str) -> Option<String> {
        // Simple pattern: if user often runs /replay after /chain resume
        if after_command.starts_with("/chain resume") {
            // Check if they frequently follow with /replay
            if self.command_frequency.get("/replay").unwrap_or(&0) > &2 {
                return Some("/replay".to_string());
            }
        }
        None
    }

    /// Get adaptive suggestion based on history
    pub fn adaptive_suggestion(&self, current_state: &str) -> Option<String> {
        match current_state {
            "completed" => {
                // User often replays after completion
                if self.command_frequency.get("/replay").unwrap_or(&0) > &2 {
                    Some(
                        "You usually run /replay after execution. Press Enter to continue."
                            .to_string(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Calculate flow mode intensity (0-1)
    pub fn flow_intensity(&self) -> f32 {
        // More clean completions = higher intensity
        let completion_factor = (self.clean_completions as f32 * 0.1).min(0.5);

        // More overrides = lower intensity
        let override_factor = (self.override_count as f32 * 0.1).min(0.3);

        // Base + completion bonus - override penalty
        (0.5 + completion_factor - override_factor).clamp(0.0, 1.0)
    }
}

/// Workflow chaining - safe multi-step execution
pub struct WorkflowChain;

impl WorkflowChain {
    /// Propose a chained workflow
    pub fn propose_chain(name: &str, steps: &[&str], preview: &ExecutionPreview) -> String {
        let mut proposal = format!("→ Ready to execute: {}\n", name);

        for (i, step) in steps.iter().enumerate() {
            proposal.push_str(&format!("  {}. {}\n", i + 1, step));
        }

        proposal.push('\n');
        proposal.push_str(&format!(
            "Risk Assessment: {}\n",
            preview.estimated_outcome.description()
        ));

        if preview.safe_to_chain {
            proposal.push_str("\nSafe to execute all steps.\nPress Enter to run full workflow.");
        } else {
            proposal.push_str(&format!(
                "\n{} checkpoint(s) will pause for approval.",
                preview.approvals_needed
            ));
        }

        proposal
    }
}

/// Enhanced interrupt state with recovery context
#[derive(Debug, Clone)]
pub struct InterruptContext {
    /// What was happening when interrupted
    pub operation_description: String,
    /// Step number (if in execution)
    pub current_step: Option<usize>,
    /// Total steps (if in execution)
    pub total_steps: Option<usize>,
    /// What the user was doing
    pub user_intent: String,
    /// How to resume
    pub resume_command: String,
    /// Alternative actions
    pub alternatives: Vec<String>,
    /// When interrupted
    pub interrupted_at: chrono::DateTime<chrono::Local>,
}

impl InterruptContext {
    /// Create interrupt context from execution state
    pub fn from_execution(step: Option<usize>, total: Option<usize>, description: &str) -> Self {
        Self {
            operation_description: description.to_string(),
            current_step: step,
            total_steps: total,
            user_intent: "Continue execution".to_string(),
            resume_command: "/chain resume".to_string(),
            alternatives: vec!["/replay".to_string(), "/plan".to_string()],
            interrupted_at: chrono::Local::now(),
        }
    }

    /// Format for display
    pub fn format(&self) -> String {
        let mut text = "⏸ Execution Paused\n\n".to_string();

        text.push_str(&format!("You were: {}\n", self.operation_description));

        if let (Some(current), Some(total)) = (self.current_step, self.total_steps) {
            text.push_str(&format!("Progress: Step {} of {}\n", current, total));
        }

        text.push('\n');
        text.push_str("→ Resume:\n");
        text.push_str(&format!("  {}\n", self.resume_command));

        if !self.alternatives.is_empty() {
            text.push_str("\n→ Or:\n");
            for alt in &self.alternatives {
                text.push_str(&format!("  {}\n", alt));
            }
        }

        text
    }

    /// V1.5: Create context from prepared action (Edge 2.1 fix)
    pub fn from_prepared_action(command: &str, reason: &str, _impact: &str) -> Self {
        let command_desc = command.to_string();

        Self {
            operation_description: format!("Prepared: {}", command_desc),
            current_step: None,
            total_steps: None,
            user_intent: reason.to_string(),
            resume_command: "/chain resume".to_string(),
            alternatives: vec!["/cancel".to_string()],
            interrupted_at: chrono::Local::now(),
        }
    }

    /// V1.5: Format prepared action context
    pub fn format_prepared(&self) -> String {
        let mut text = String::new();

        text.push_str(&format!("Action: {}\n", self.operation_description));
        if !self.user_intent.is_empty() {
            text.push_str(&format!("Reason: {}\n", self.user_intent));
        }

        text
    }
}

// =============================================================================
// V2.0: Goal-Driven Autonomous Operator
// =============================================================================

/// Goal status lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStatus {
    /// Goal stated but plan not yet generated
    Stated,
    /// Plan generation in progress
    Planning,
    /// Plan generated, waiting for operator review
    Proposed,
    /// Plan accepted, executing
    Executing,
    /// All steps completed successfully
    Completed,
    /// Execution failed, can retry
    Failed,
    /// Goal cancelled by operator
    Cancelled,
}

impl GoalStatus {
    /// Human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            GoalStatus::Stated => "Stated",
            GoalStatus::Planning => "Planning",
            GoalStatus::Proposed => "Proposed",
            GoalStatus::Executing => "Executing",
            GoalStatus::Completed => "Completed",
            GoalStatus::Failed => "Failed",
            GoalStatus::Cancelled => "Cancelled",
        }
    }

    /// Icon for UI display
    pub fn icon(&self) -> &'static str {
        match self {
            GoalStatus::Stated => "◯",
            GoalStatus::Planning => "◐",
            GoalStatus::Proposed => "◉",
            GoalStatus::Executing => "▶",
            GoalStatus::Completed => "✓",
            GoalStatus::Failed => "✗",
            GoalStatus::Cancelled => "⊘",
        }
    }
}

/// A stated goal from the operator
#[derive(Debug, Clone)]
pub struct Goal {
    /// Unique goal ID
    pub id: String,
    /// Natural language goal statement
    pub statement: String,
    /// Current status
    pub status: GoalStatus,
    /// When the goal was stated
    pub stated_at: chrono::DateTime<chrono::Local>,
    /// Associated chain ID (once materialized)
    pub chain_id: Option<String>,
    /// Associated conversation ID
    pub conversation_id: String,
    /// Context files relevant to the goal
    pub context_files: Vec<String>,
    /// Generated plan (once available)
    pub generated_plan: Option<GeneratedPlan>,
    /// Completion summary (once done)
    pub completion_summary: Option<String>,
    /// V2.2: Previous rejected plan (for diff comparison)
    pub previous_plan: Option<GeneratedPlan>,
    /// V2.2: Rejection reason (for learning)
    pub rejection_reason: Option<String>,
    /// V2.2: Failure reason (for learning)
    pub failure_reason: Option<String>,
}

impl Goal {
    /// Create a new goal from operator statement
    pub fn new(statement: impl Into<String>, conversation_id: impl Into<String>) -> Self {
        let id = format!("goal-{}", uuid::Uuid::new_v4());
        Self {
            id,
            statement: statement.into(),
            status: GoalStatus::Stated,
            stated_at: chrono::Local::now(),
            chain_id: None,
            conversation_id: conversation_id.into(),
            context_files: vec![],
            generated_plan: None,
            completion_summary: None,
            previous_plan: None,
            rejection_reason: None,
            failure_reason: None,
        }
    }

    /// Associate this goal with a chain
    pub fn attach_chain(&mut self, chain_id: impl Into<String>) {
        self.chain_id = Some(chain_id.into());
    }

    /// Mark plan as generated
    pub fn set_plan(&mut self, plan: GeneratedPlan) {
        self.generated_plan = Some(plan);
        self.status = GoalStatus::Proposed;
    }

    /// Mark as executing
    pub fn mark_executing(&mut self) {
        self.status = GoalStatus::Executing;
    }

    /// Mark as completed
    pub fn mark_completed(&mut self, summary: impl Into<String>) {
        self.status = GoalStatus::Completed;
        self.completion_summary = Some(summary.into());
    }

    /// Mark as failed
    pub fn mark_failed(&mut self) {
        self.status = GoalStatus::Failed;
    }

    /// V2.2: Mark as failed with reason
    pub fn mark_failed_with_reason(&mut self, reason: impl Into<String>) {
        self.status = GoalStatus::Failed;
        self.failure_reason = Some(reason.into());
    }
}

/// A generated plan with full explanation
#[derive(Debug, Clone)]
pub struct GeneratedPlan {
    /// Full original task prompt, preserved verbatim for execution grounding.
    pub raw_prompt: String,
    /// Objective summary (derived from goal)
    pub objective: String,
    /// Ordered steps to execute
    pub steps: Vec<PlanStep>,
    /// Risks detected during planning
    pub risks: Vec<Risk>,
    /// Steps likely to require approval
    pub approval_points: Vec<usize>,
    /// Context files needed
    pub required_context: Vec<String>,
    /// Estimated outcome
    pub estimated_outcome: OutcomePrediction,
    /// Whether this plan is safe to auto-chain
    pub safe_to_chain: bool,
    /// Why this plan makes sense
    pub reasoning: String,
}

/// A single step in a generated plan
#[derive(Debug, Clone)]
pub struct PlanStep {
    /// Step number (1-indexed)
    pub number: usize,
    /// Human-readable description
    pub description: String,
    /// Action type classification
    pub action_type: StepActionType,
    /// Estimated risk level
    pub risk_level: RiskLevel,
    /// Whether this step likely needs approval
    pub likely_approval_needed: bool,
    /// Files likely affected
    pub affected_files: Vec<String>,
}

/// Plan generation result - success or failure with explanation
#[derive(Debug, Clone)]
pub enum PlanGenerationResult {
    /// Plan generated successfully
    Success(GeneratedPlan),
    /// Plan generation failed with reason
    Failed { reason: String, suggestion: String },
}

/// V2.3: Plan Personality Consistency Templates
/// Ensures all plans feel like they come from the same "operator personality"
pub struct StepTemplates;

impl StepTemplates {
    // V2.3: Consistent verb choices for each action type
    // Avoids vague terms like "handle", "process", "do"

    /// Analysis steps - always start with these patterns
    pub fn analyze(subject: &str) -> String {
        format!("Analyze {}", subject)
    }

    pub fn review(subject: &str) -> String {
        format!("Review {}", subject)
    }

    pub fn examine(subject: &str) -> String {
        format!("Examine {}", subject)
    }

    pub fn identify(subject: &str) -> String {
        format!("Identify {}", subject)
    }

    /// Implementation steps - specific action verbs
    pub fn implement(subject: &str) -> String {
        format!("Implement {}", subject)
    }

    pub fn add(subject: &str) -> String {
        format!("Add {}", subject)
    }

    pub fn create(subject: &str) -> String {
        format!("Create {}", subject)
    }

    pub fn refactor(subject: &str) -> String {
        format!("Refactor {}", subject)
    }

    pub fn restructure(subject: &str) -> String {
        format!("Restructure {}", subject)
    }

    pub fn extract(subject: &str, target: &str) -> String {
        format!("Extract {} into {}", subject, target)
    }

    pub fn consolidate(subject: &str) -> String {
        format!("Consolidate {}", subject)
    }

    pub fn split(subject: &str, into: &str) -> String {
        format!("Split {} into {}", subject, into)
    }

    pub fn update(subject: &str, change: &str) -> String {
        format!("Update {} to {}", subject, change)
    }

    pub fn modify(subject: &str) -> String {
        format!("Modify {}", subject)
    }

    /// Validation steps - verification patterns
    pub fn validate(subject: &str) -> String {
        format!("Validate {}", subject)
    }

    pub fn verify(subject: &str) -> String {
        format!("Verify {}", subject)
    }

    pub fn test(subject: &str) -> String {
        format!("Test {}", subject)
    }

    pub fn confirm(subject: &str) -> String {
        format!("Confirm {}", subject)
    }

    pub fn check(subject: &str) -> String {
        format!("Check {}", subject)
    }

    /// Diagnostic steps - debugging/investigation
    pub fn diagnose(subject: &str) -> String {
        format!("Diagnose {}", subject)
    }

    pub fn trace(subject: &str) -> String {
        format!("Trace {}", subject)
    }

    pub fn investigate(subject: &str) -> String {
        format!("Investigate {}", subject)
    }

    pub fn locate(subject: &str) -> String {
        format!("Locate {}", subject)
    }

    /// Documentation steps
    pub fn document(subject: &str) -> String {
        format!("Document {}", subject)
    }

    pub fn describe(subject: &str) -> String {
        format!("Describe {}", subject)
    }

    pub fn explain(subject: &str, audience: &str) -> String {
        format!("Explain {} for {}", subject, audience)
    }

    /// Repair steps
    pub fn fix(subject: &str) -> String {
        format!("Fix {}", subject)
    }

    pub fn repair(subject: &str) -> String {
        format!("Repair {}", subject)
    }

    pub fn resolve(subject: &str) -> String {
        format!("Resolve {}", subject)
    }

    pub fn correct(subject: &str) -> String {
        format!("Correct {}", subject)
    }

    /// Cleanup steps
    pub fn clean(subject: &str) -> String {
        format!("Clean up {}", subject)
    }

    pub fn organize(subject: &str) -> String {
        format!("Organize {}", subject)
    }

    pub fn remove(subject: &str) -> String {
        format!("Remove {}", subject)
    }

    pub fn delete(subject: &str) -> String {
        format!("Delete {}", subject)
    }

    pub fn format(subject: &str) -> String {
        format!("Format {}", subject)
    }

    /// V2.3: Template for common step sequences
    pub fn standard_analysis(target: &str) -> String {
        format!("Analyze current {}", target)
    }

    pub fn standard_change(target: &str, action: &str) -> String {
        format!("{} {}", action, target)
    }

    pub fn standard_validation(target: &str) -> String {
        format!("Validate {}", target)
    }
}

/// V2.0 Plan Generation Engine with V2.3 personality consistency
pub struct PlanEngine;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteralCreationIntent {
    pub artifact_class: String,
    pub target_path: String,
    pub content_requirements: String,
}

impl LiteralCreationIntent {
    pub fn detect(statement: &str) -> Option<Self> {
        let lower = normalize_intent_text(statement);
        let has_creation_verb = lower.starts_with("create ")
            || lower.starts_with("write ")
            || lower.starts_with("make ")
            || lower.starts_with("add ")
            || lower.starts_with("please create ")
            || lower.starts_with("please write ")
            || lower.starts_with("can you create ")
            || lower.starts_with("can you write ");

        if !has_creation_verb || requires_full_planning(&lower) {
            return None;
        }

        if lower.contains("hello world") && lower.contains("script") {
            return Some(Self {
                artifact_class: "hello world script".to_string(),
                target_path: "hello_world.py".to_string(),
                content_requirements:
                    "Write a minimal script that prints exactly Hello, world! when run.".to_string(),
            });
        }

        if lower.contains("todo file")
            || lower.contains("to do file")
            || lower.contains("to-do file")
        {
            return Some(Self {
                artifact_class: "todo file".to_string(),
                target_path: "TODO.md".to_string(),
                content_requirements:
                    "Write a simple TODO list file with a short heading and placeholder tasks."
                        .to_string(),
            });
        }

        let asks_for_docs_note = (lower.contains("docs note")
            || lower.contains("doc note")
            || lower.contains("documentation note"))
            && lower.contains("file");
        if asks_for_docs_note {
            let target_path = if lower.contains("tiny") || lower.contains("small") {
                "docs/tiny-note.md"
            } else {
                "docs/note.md"
            };
            return Some(Self {
                artifact_class: "docs note".to_string(),
                target_path: target_path.to_string(),
                content_requirements:
                    "Write a small generic documentation note. Do not analyze, summarize, or document repository implementation locations, src files, or project structure."
                        .to_string(),
            });
        }

        None
    }

    fn to_plan(&self, original_statement: &str) -> GeneratedPlan {
        let create_description = format!(
            "Create the requested literal artifact. Target artifact: {}; Artifact class: {}; Content requirements: {}; Completion requires writing exactly this artifact path and not substituting source, implementation, or repo-analysis documentation.",
            self.target_path, self.artifact_class, self.content_requirements
        );
        let validate_description = format!(
            "Validate the literal artifact exists at {} and matches artifact class: {}.",
            self.target_path, self.artifact_class
        );

        GeneratedPlan {
            raw_prompt: original_statement.to_string(),
            objective: summarize_goal_objective(original_statement),
            steps: vec![
                PlanStep {
                    number: 1,
                    description: create_description,
                    action_type: StepActionType::Write,
                    risk_level: RiskLevel::Safe,
                    likely_approval_needed: false,
                    affected_files: vec![self.target_path.clone()],
                },
                PlanStep {
                    number: 2,
                    description: validate_description,
                    action_type: StepActionType::Validate,
                    risk_level: RiskLevel::Safe,
                    likely_approval_needed: false,
                    affected_files: vec![self.target_path.clone()],
                },
            ],
            risks: vec![],
            approval_points: vec![],
            required_context: vec![],
            estimated_outcome: OutcomePrediction::Success,
            safe_to_chain: true,
            reasoning: format!(
                "Detected simple literal creation intent for {} at {}; direct artifact creation is preferred over repository analysis.",
                self.artifact_class, self.target_path
            ),
        }
    }
}

pub fn extract_explicit_artifact_contract(statement: &str) -> Option<ArtifactCompletionContract> {
    let required_artifacts = extract_explicit_artifact_requirements(statement);
    let required_filenames = required_artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>();
    let required_count = detect_explicit_artifact_count(statement).or_else(|| {
        if required_filenames.len() > 1 {
            Some(required_filenames.len())
        } else {
            None
        }
    });

    let lower = statement.to_lowercase();
    let numbered_filename_lines = statement
        .lines()
        .filter(|line| line_contains_numbered_filename(line))
        .count();
    let has_contract_language = lower.contains("exactly")
        || lower.contains("precise filenames")
        || lower.contains("exact filenames")
        || lower.contains("all of these")
        || lower.contains("all of the following")
        || lower.contains("must be produced")
        || lower.contains("must produce")
        || lower.contains("deliverable set")
        || lower.contains("named markdown files")
        || lower.contains("required files")
        || lower.contains("required artifacts");

    let is_explicit_contract = required_filenames.len() > 1
        && (required_count.is_some() || has_contract_language || numbered_filename_lines >= 2);
    if !is_explicit_contract {
        return None;
    }

    Some(ArtifactCompletionContract {
        artifact_type: detect_artifact_type(statement, &required_filenames),
        required_count,
        required_filenames,
        required_artifacts,
        created_filenames: vec![],
        missing_filenames: vec![],
        empty_filenames: vec![],
        unexpected_filenames: vec![],
        actual_output_count: None,
        require_non_empty: true,
    })
}

/// Extract objective from task/action sections, excluding completion rules and validation sections.
/// This prevents "DONE means..." completion rules from being selected as the objective.
fn extract_objective_from_sections(statement: &str) -> Option<String> {
    let lines: Vec<&str> = statement.lines().collect();
    let _lower_statement = statement.to_lowercase();

    // Section header patterns that indicate task/objective sections (in priority order)
    let task_section_headers: &[&str] = &[
        "your task is",
        "task:",
        "objective:",
        "goal:",
        "mission:",
        "assignment:",
        "action required",
        "what you must do",
        "create or update",
        "produce exactly",
        "generate exactly",
        "implement",
        "build",
    ];

    // Section header patterns that indicate completion/validation sections (to exclude)
    let completion_section_headers: &[&str] = &[
        "done means",
        "completion rule",
        "completion criteria",
        "validation:",
        "validate:",
        "requirements:",
        "constraints:",
        "final instruction",
        "output contract",
        "success criteria",
        "verification:",
        "checklist:",
    ];

    // First, try to find an explicit task/objective section
    for (idx, line) in lines.iter().enumerate() {
        let lower_line = line.to_lowercase();

        // Check if this line starts a task section
        let is_task_header = task_section_headers
            .iter()
            .any(|header| lower_line.starts_with(header) || lower_line.contains(header));

        if is_task_header {
            // Extract content from this header line (after the colon if present)
            let content = if let Some(pos) = line.find(':') {
                line[pos + 1..].trim()
            } else {
                line.trim()
            };

            if !content.is_empty() && !is_completion_rule_line(content) {
                return Some(content.to_string());
            }

            // If header line is empty after colon, look at next lines
            for next_line in lines.iter().skip(idx + 1).take(3) {
                let trimmed = next_line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Stop if we hit another section header
                let lower_next = trimmed.to_lowercase();
                if task_section_headers.iter().any(|h| lower_next.starts_with(h))
                    || completion_section_headers
                        .iter()
                        .any(|h| lower_next.starts_with(h))
                {
                    break;
                }
                if !is_completion_rule_line(trimmed) {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    // Second pass: look for imperative sentences that indicate the objective
    for line in lines.iter() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Skip completion rule lines
        if is_completion_rule_line(trimmed) {
            continue;
        }

        let lower = trimmed.to_lowercase();

        // Look for action-oriented beginnings
        let action_prefixes = [
            "create ",
            "update ",
            "modify ",
            "implement ",
            "build ",
            "generate ",
            "produce ",
            "write ",
            "refactor ",
            "fix ",
            "add ",
        ];

        if action_prefixes.iter().any(|prefix| lower.starts_with(prefix)) {
            // Check this isn't just a file list item
            if !trimmed.starts_with(|c: char| c.is_ascii_digit() && trimmed.contains('.')) {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

/// Check if a line is a completion rule (not suitable as objective)
pub(crate) fn is_completion_rule_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    let completion_indicators = [
        "done means",
        "success means",
        "complete when",
        "finished when",
        "validation:",
        "verify that",
        "ensure that",
        "all tests pass",
        "file(s) exist",
        "exists and",
        "must exist",
        "required files",
    ];

    completion_indicators
        .iter()
        .any(|indicator| lower.contains(indicator))
}

pub fn summarize_goal_objective(statement: &str) -> String {
    if let Some(contract) = extract_explicit_artifact_contract(statement)
        && contract.has_requirements()
    {
        let artifact_label = contract
            .artifact_type
            .clone()
            .unwrap_or_else(|| "artifact".to_string());
        let mentions_purposes = contract
            .required_artifacts
            .iter()
            .any(|artifact| artifact.purpose.is_some());
        let suffix = if mentions_purposes {
            " and purposes"
        } else {
            ""
        };
        return format!(
            "Generate exactly {} {} file(s) with the specified filenames{}",
            contract.required_deliverable_count(),
            artifact_label,
            suffix
        );
    }

    if let Some(intent) = LiteralCreationIntent::detect(statement) {
        return format!(
            "Create {} at {}",
            intent.artifact_class, intent.target_path
        );
    }

    // Use section-aware extraction to find the objective
    if let Some(objective) = extract_objective_from_sections(statement) {
        let collapsed = objective.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() && !is_vague_objective_summary(&collapsed) {
            return crate::text::truncate_chars(&collapsed, 120);
        }
    }

    // Fallback: find first non-empty, non-completion-rule line
    let first_valid_line = statement
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !is_completion_rule_line(line)
                && !line.starts_with(|c: char| c == '#' || c == '-' || c == '*')
        })
        .unwrap_or(statement);

    let collapsed = first_valid_line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "Execute requested task".to_string()
    } else {
        crate::text::truncate_chars(&collapsed, 120)
    }
}

pub fn is_vague_objective_summary(summary: &str) -> bool {
    let normalized = summary
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let vague_phrases = [
        "begin your analysis now",
        "analyze goal requirements",
        "analyze requirements",
        "start analysis",
        "execute implementation",
        "perform the task",
        "work on the task",
    ];

    vague_phrases
        .iter()
        .any(|phrase| normalized == *phrase || normalized.contains(phrase))
}

pub fn build_objective_satisfaction(statement: &str) -> ObjectiveSatisfaction {
    let artifact_contract = extract_explicit_artifact_contract(statement);
    let required_surfaces = artifact_contract
        .as_ref()
        .map(|contract| {
            contract
                .required_filenames
                .iter()
                .cloned()
                .map(|path| RequiredSurface::FileExists { path })
                .collect()
        })
        .unwrap_or_default();

    let reason = artifact_contract.as_ref().map(|contract| {
        format!(
            "Explicit deliverable contract extracted: {} required artifact(s).",
            contract.required_deliverable_count()
        )
    });

    ObjectiveSatisfaction {
        required_surfaces,
        artifact_contract,
        reason,
        ..Default::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ArtifactCrudPlanState {
    existing_required_filenames: Vec<String>,
    missing_filenames: Vec<String>,
    empty_filenames: Vec<String>,
}

impl ArtifactCrudPlanState {
    fn operation_for_path(&self, path: &str) -> ArtifactCrudOperation {
        if self.empty_filenames.iter().any(|candidate| candidate == path) {
            ArtifactCrudOperation::ReplaceEmpty
        } else if self
            .existing_required_filenames
            .iter()
            .any(|candidate| candidate == path)
        {
            ArtifactCrudOperation::UpdateExisting
        } else {
            ArtifactCrudOperation::CreateMissing
        }
    }
}

impl PlanEngine {
    pub fn generate_explicit_artifact_plan(
        goal: &Goal,
        repo_path: Option<&str>,
    ) -> Option<GeneratedPlan> {
        let contract = extract_explicit_artifact_contract(&goal.statement)?;
        if contract.required_deliverable_count() <= 1 {
            return None;
        }

        let artifact_state = snapshot_artifact_plan_state(&contract, repo_path);
        let artifact_label = contract
            .artifact_type
            .clone()
            .unwrap_or_else(|| "artifact".to_string());
        let mut steps = Vec::with_capacity(contract.required_filenames.len() + 2);
        let all_targets = contract.required_filenames.clone();

        steps.push(PlanStep {
            number: 1,
            description: format!(
                "{} the explicit deliverable contract for {} {} artifact(s), inventory the required set, and keep the exact filename list authoritative. Current state: {} existing, {} missing, {} empty.",
                ArtifactCrudOperation::ListRequired.verb(),
                contract.required_deliverable_count(),
                artifact_label,
                artifact_state.existing_required_filenames.len(),
                artifact_state.missing_filenames.len(),
                artifact_state.empty_filenames.len()
            ),
            action_type: StepActionType::Read,
            risk_level: RiskLevel::Safe,
            likely_approval_needed: false,
            affected_files: all_targets.clone(),
        });

        for path in &contract.required_filenames {
            let operation = artifact_state.operation_for_path(path);
            let purpose_clause = contract
                .purpose_for_path(path)
                .map(|purpose| format!(" Required purpose: {}.", purpose))
                .unwrap_or_default();
            steps.push(PlanStep {
                number: steps.len() + 1,
                description: format!(
                    "{} required {} artifact {} and satisfy the exact filename contract with non-empty content. Operation intent: {}.{}",
                    operation.verb(),
                    artifact_label,
                    path,
                    operation.description(),
                    purpose_clause
                ),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![path.clone()],
            });
        }

        steps.push(PlanStep {
            number: steps.len() + 1,
            description: format!(
                "{} for the full deliverable contract: all {} required {} artifact(s) must exist, be non-empty, and the task cannot finish while any required filename is missing.",
                ArtifactCrudOperation::CheckCompleteness.verb(),
                contract.required_deliverable_count(),
                artifact_label
            ),
            action_type: StepActionType::Validate,
            risk_level: RiskLevel::Safe,
            likely_approval_needed: false,
            affected_files: all_targets.clone(),
        });

        Some(GeneratedPlan {
            raw_prompt: goal.statement.clone(),
            objective: summarize_goal_objective(&goal.statement),
            steps,
            risks: vec![],
            approval_points: vec![],
            required_context: all_targets,
            estimated_outcome: OutcomePrediction::Success,
            safe_to_chain: true,
            reasoning: format!(
                "Detected an explicit multi-artifact deliverable contract with {} exact filename(s); the plan decomposes work per required artifact, applies artifact CRUD semantics (create missing, update existing, replace empty, list required, check completeness), and defers completion until the full set is present.",
                contract.required_deliverable_count(),
            ),
        })
    }

    pub fn generate_literal_creation(goal: &Goal) -> Option<GeneratedPlan> {
        LiteralCreationIntent::detect(&goal.statement).map(|intent| intent.to_plan(&goal.statement))
    }

    /// Generate a plan from a stated goal
    ///
    /// This is a bounded plan generation that produces a concrete chain-ready plan.
    /// It uses context assembly, repo state, and goal analysis to produce steps.
    pub fn generate(
        goal: &Goal,
        state: &AppState,
        persistence: &PersistentState,
    ) -> PlanGenerationResult {
        let repo_path = if state.repo.path.trim().is_empty() {
            persistence.active_repo.as_deref()
        } else {
            Some(state.repo.path.as_str())
        };

        if let Some(plan) = Self::generate_explicit_artifact_plan(goal, repo_path) {
            return PlanGenerationResult::Success(plan);
        }

        if let Some(plan) = Self::generate_literal_creation(goal) {
            return PlanGenerationResult::Success(plan);
        }

        // Get repo context for grounding
        // Get repo context from state
        let repo_context = if state.repo.path.is_empty() {
            None
        } else {
            Some(state.repo.path.clone())
        };

        // Analyze goal to determine step patterns
        let goal_lower = goal.statement.to_lowercase();

        // Build plan based on goal type
        let (steps, risks, approval_points, reasoning) =
            Self::analyze_goal(&goal_lower, &repo_context, state, persistence);

        // Check for critical risks
        let has_critical = risks.iter().any(|r| r.level == RiskLevel::Critical);

        // Safe to chain only if no critical risks and no mandatory approvals
        let safe_to_chain = !has_critical && approval_points.is_empty();

        // Estimate outcome
        let estimated_outcome = if has_critical {
            OutcomePrediction::Uncertain
        } else if !approval_points.is_empty() {
            OutcomePrediction::SuccessWithWarnings
        } else {
            OutcomePrediction::Success
        };

        // Collect required context from affected files
        let mut unique_files = std::collections::HashSet::new();
        for step in &steps {
            for file in &step.affected_files {
                unique_files.insert(file.clone());
            }
        }
        let required_context: Vec<String> = unique_files.into_iter().collect();

        PlanGenerationResult::Success(GeneratedPlan {
            raw_prompt: goal.statement.clone(),
            objective: summarize_goal_objective(&goal.statement),
            steps,
            risks,
            approval_points,
            required_context,
            estimated_outcome,
            safe_to_chain,
            reasoning,
        })
    }

    /// Analyze goal and produce plan components
    fn analyze_goal(
        goal: &str,
        _repo_path: &Option<String>,
        _state: &AppState,
        persistence: &PersistentState,
    ) -> (Vec<PlanStep>, Vec<Risk>, Vec<usize>, String) {
        let mut steps = Vec::new();
        let mut risks = Vec::new();
        let mut approval_points = Vec::new();

        // Goal pattern analysis - V2.1 expanded patterns
        let is_config = goal.contains("config") || goal.contains("configuration");
        let is_refactor = goal.contains("refactor") || goal.contains("restructure");
        let is_add = goal.contains("add") || goal.contains("create");
        let is_fix = goal.contains("fix") || goal.contains("repair");
        let is_audit = goal.contains("audit") || goal.contains("review");
        let is_debug =
            goal.contains("debug") || goal.contains("investigate") || goal.contains("diagnose");
        let is_docs = goal.contains("document") || goal.contains("doc") || goal.contains("readme");
        let is_test =
            goal.contains("test") || goal.contains("coverage") || goal.contains("add test");
        let is_clean =
            goal.contains("clean up") || goal.contains("organize") || goal.contains("format");

        // Generate steps based on goal type
        if is_config {
            // Config system goal
            steps.push(PlanStep {
                number: 1,
                description: "Analyze existing config patterns".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec!["config.rs".to_string(), "settings.rs".to_string()],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Create new config module".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Caution,
                likely_approval_needed: true,
                affected_files: vec!["config/mod.rs".to_string()],
            });
            approval_points.push(2);
        } else if is_refactor {
            // Refactoring goal
            steps.push(PlanStep {
                number: 1,
                description: "Analyze current structure".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Refactor main components".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Warning,
                likely_approval_needed: true,
                affected_files: vec![],
            });
            approval_points.push(2);

            // Check for uncommitted changes - adds risk
            if let Some(chain) = persistence.get_active_chain() {
                if chain
                    .git_grounding
                    .as_ref()
                    .map(|g| g.is_dirty)
                    .unwrap_or(false)
                {
                    risks.push(Risk {
                        risk_type: RiskType::GitConflict,
                        description: "Uncommitted changes may conflict with refactoring"
                            .to_string(),
                        affected: vec!["working directory".to_string()],
                        mitigation: "/git status to review, commit before refactoring".to_string(),
                        level: RiskLevel::Warning,
                    });
                }
            }
        } else if is_add || is_fix {
            // Add new feature or fix
            steps.push(PlanStep {
                number: 1,
                description: "Identify implementation location".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Implement changes".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Caution,
                likely_approval_needed: true,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 3,
                description: "Validate implementation".to_string(),
                action_type: StepActionType::Validate,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            approval_points.push(2);
        } else if is_audit {
            // Audit/review - read-only, no approvals
            steps.push(PlanStep {
                number: 1,
                description: "Review current state".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Generate audit report".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
        } else if is_debug {
            // Debug/diagnose - analysis heavy, may need fix approval
            steps.push(PlanStep {
                number: 1,
                description: "Analyze error logs and state".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Identify root cause".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 3,
                description: "Implement fix".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Caution,
                likely_approval_needed: true,
                affected_files: vec![],
            });
            approval_points.push(3);
        } else if is_docs {
            // Documentation - low risk, no approvals needed
            steps.push(PlanStep {
                number: 1,
                description: "Identify documentation surface".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Create documentation".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Safe, // Docs are low-risk
                likely_approval_needed: false,
                affected_files: vec![],
            });
        } else if is_test {
            // Testing - adds test coverage, low risk
            steps.push(PlanStep {
                number: 1,
                description: "Identify test gaps".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Create tests".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Safe, // Tests don't modify production code
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 3,
                description: "Run tests to validate".to_string(),
                action_type: StepActionType::Execute,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
        } else if is_clean {
            // Cleanup/formatting - automated, low risk
            steps.push(PlanStep {
                number: 1,
                description: "Analyze current state".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Apply formatting and organization".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Caution, // Changes many files
                likely_approval_needed: true,   // Better safe for bulk changes
                affected_files: vec![],
            });
            approval_points.push(2);
        } else {
            // Generic goal handling
            steps.push(PlanStep {
                number: 1,
                description: "Analyze goal requirements".to_string(),
                action_type: StepActionType::Read,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            });
            steps.push(PlanStep {
                number: 2,
                description: "Execute implementation".to_string(),
                action_type: StepActionType::Write,
                risk_level: RiskLevel::Caution,
                likely_approval_needed: true,
                affected_files: vec![],
            });
            approval_points.push(2);
        }

        // Add validation step if there are write operations
        if steps.iter().any(|s| s.action_type == StepActionType::Write) {
            let validation_step = PlanStep {
                number: steps.len() + 1,
                description: "Validate changes".to_string(),
                action_type: StepActionType::Validate,
                risk_level: RiskLevel::Safe,
                likely_approval_needed: false,
                affected_files: vec![],
            };
            steps.push(validation_step);
        }

        // Generate reasoning
        let reasoning = format!(
            "Plan generated based on goal '{}'\n\
            Detected intent: {}\n\
            {} steps proposed with {} approval point(s).\n\
            Estimated risk: {} ({} critical risk(s))",
            goal,
            if is_config {
                "config system"
            } else if is_refactor {
                "refactoring"
            } else if is_add {
                "add feature"
            } else if is_fix {
                "fix issue"
            } else if is_audit {
                "audit/review"
            } else if is_debug {
                "debug/diagnose"
            } else if is_docs {
                "documentation"
            } else if is_test {
                "testing"
            } else if is_clean {
                "cleanup/formatting"
            } else {
                "general implementation"
            },
            steps.len(),
            approval_points.len(),
            if risks.is_empty() { "low" } else { "elevated" },
            risks
                .iter()
                .filter(|r| r.level == RiskLevel::Critical)
                .count()
        );

        (steps, risks, approval_points, reasoning)
    }
}

fn normalize_intent_text(statement: &str) -> String {
    statement
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_explicit_artifact_count(statement: &str) -> Option<usize> {
    let normalized = normalize_intent_text(statement);
    let words: Vec<&str> = normalized.split_whitespace().collect();

    for (idx, word) in words.iter().enumerate() {
        let Some(count) = parse_count_word(word) else {
            continue;
        };
        let previous = idx
            .checked_sub(1)
            .and_then(|offset| words.get(offset))
            .copied()
            .unwrap_or_default();
        let next = words.get(idx + 1).copied().unwrap_or_default();
        let next_two = words.get(idx + 2).copied().unwrap_or_default();

        if matches!(
            previous,
            "exactly" | "precisely" | "total" | "produce" | "creating"
        ) || matches!(
            next,
            "artifact" | "artifacts" | "doc" | "docs" | "document" | "documents" | "file"
                | "files" | "markdown"
        ) || matches!(
            next_two,
            "artifact" | "artifacts" | "doc" | "docs" | "document" | "documents" | "file"
                | "files" | "markdown"
        ) {
            return Some(count);
        }
    }

    None
}

fn parse_count_word(word: &str) -> Option<usize> {
    if let Ok(value) = word.parse::<usize>() {
        return Some(value);
    }

    match word {
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        "eleven" => Some(11),
        "twelve" => Some(12),
        "thirteen" => Some(13),
        "fourteen" => Some(14),
        "fifteen" => Some(15),
        "sixteen" => Some(16),
        "seventeen" => Some(17),
        "eighteen" => Some(18),
        "nineteen" => Some(19),
        "twenty" => Some(20),
        _ => None,
    }
}

fn extract_explicit_filenames(statement: &str) -> Vec<String> {
    extract_explicit_artifact_requirements(statement)
        .into_iter()
        .map(|artifact| artifact.path)
        .collect()
}

fn extract_explicit_artifact_requirements(statement: &str) -> Vec<ArtifactRequirement> {
    let lines: Vec<&str> = statement.lines().collect();
    let mut requirements = Vec::new();
    let mut seen = HashSet::new();

    for (index, line) in lines.iter().enumerate() {
        for candidate in extract_filename_candidates_from_line(line) {
            if seen.insert(candidate.clone()) {
                requirements.push(ArtifactRequirement {
                    path: candidate.clone(),
                    purpose: extract_artifact_requirement_purpose(&lines, index, &candidate),
                });
            }
        }
    }

    requirements
}

fn extract_artifact_requirement_purpose(
    lines: &[&str],
    line_index: usize,
    path: &str,
) -> Option<String> {
    let line = lines.get(line_index)?.trim();
    if let Some(start) = line.find(path) {
        let trailing = &line[start + path.len()..];
        if let Some(purpose) = normalize_artifact_purpose(trailing) {
            return Some(purpose);
        }
    }

    let mut continuation = Vec::new();
    for next_line in lines.iter().skip(line_index + 1) {
        let trimmed = next_line.trim();
        if trimmed.is_empty() {
            break;
        }
        if !extract_filename_candidates_from_line(trimmed).is_empty() {
            break;
        }

        if next_line
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace())
            || trimmed.starts_with('-')
            || trimmed.starts_with('*')
            || trimmed.starts_with('•')
            || trimmed.starts_with("Purpose")
            || trimmed.starts_with("purpose")
            || trimmed.starts_with("Focus")
            || trimmed.starts_with("focus")
            || trimmed.starts_with("Include")
            || trimmed.starts_with("include")
        {
            continuation.push(trimmed.to_string());
        } else {
            break;
        }

        if continuation.len() >= 3 {
            break;
        }
    }

    if continuation.is_empty() {
        None
    } else {
        normalize_artifact_purpose(&continuation.join(" "))
    }
}

fn normalize_artifact_purpose(fragment: &str) -> Option<String> {
    let cleaned = fragment
        .trim()
        .trim_start_matches(|ch: char| matches!(ch, '-' | ':' | '–' | '—' | ';' | ',' | '|'))
        .trim();
    if cleaned.is_empty() {
        return None;
    }

    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = collapsed.to_lowercase();
    if lower.starts_with("all of these")
        || lower.starts_with("all of the following")
        || lower.starts_with("must be produced")
    {
        None
    } else {
        Some(collapsed)
    }
}

fn line_contains_numbered_filename(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some((prefix, remainder)) = trimmed.split_once('.') else {
        return false;
    };
    if !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }

    !extract_filename_candidates_from_line(remainder).is_empty()
}

fn extract_filename_candidates_from_line(line: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut trimmed = line.trim();

    while matches!(trimmed.chars().next(), Some('#' | '-' | '*' | '•')) {
        trimmed = trimmed[1..].trim_start();
    }

    if let Some((prefix, remainder)) = trimmed.split_once('.') {
        if prefix.chars().all(|ch| ch.is_ascii_digit()) {
            trimmed = remainder.trim_start();
        }
    }

    if let Some((prefix, remainder)) = trimmed.split_once(')') {
        if prefix.chars().all(|ch| ch.is_ascii_digit()) {
            trimmed = remainder.trim_start();
        }
    }

    for token in trimmed.split_whitespace() {
        let candidate = normalize_artifact_token(token);
        if looks_like_filename(&candidate) {
            candidates.push(candidate);
        }
    }

    candidates
}

fn normalize_artifact_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '`' | '"' | '\'' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        })
        .trim_start_matches("./")
        .replace('\\', "/")
}

fn looks_like_filename(candidate: &str) -> bool {
    if candidate.is_empty()
        || candidate.starts_with("http://")
        || candidate.starts_with("https://")
        || candidate.starts_with('/')
        || !candidate.contains('.')
    {
        return false;
    }

    let Some((stem, extension)) = candidate.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty()
        || extension.len() < 2
        || extension.len() > 10
        || !extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        || !stem.chars().any(|ch| ch.is_ascii_alphabetic())
    {
        return false;
    }

    candidate.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.')
    })
}

fn detect_artifact_type(statement: &str, required_filenames: &[String]) -> Option<String> {
    let lower = statement.to_lowercase();
    if lower.contains("markdown") {
        return Some("markdown".to_string());
    }

    if required_filenames.is_empty() {
        return None;
    }

    if required_filenames
        .iter()
        .all(|path| path.ends_with(".md") || path.ends_with(".markdown"))
    {
        Some("markdown".to_string())
    } else if required_filenames.iter().all(|path| path.ends_with(".json")) {
        Some("json".to_string())
    } else if required_filenames.iter().all(|path| path.ends_with(".txt")) {
        Some("text".to_string())
    } else {
        required_filenames[0]
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_string())
    }
}

fn snapshot_artifact_plan_state(
    contract: &ArtifactCompletionContract,
    repo_path: Option<&str>,
) -> ArtifactCrudPlanState {
    let mut state = ArtifactCrudPlanState::default();

    for path in &contract.required_filenames {
        let resolved = resolve_contract_path(repo_path, path);
        if !resolved.exists() {
            state.missing_filenames.push(path.clone());
            continue;
        }

        if contract.require_non_empty && !path_has_content(&resolved) {
            state.empty_filenames.push(path.clone());
        } else {
            state.existing_required_filenames.push(path.clone());
        }
    }

    state
}

fn resolve_contract_path(repo_path: Option<&str>, path: &str) -> PathBuf {
    let path_ref = Path::new(path);
    if path_ref.is_absolute() {
        path_ref.to_path_buf()
    } else if let Some(repo_path) = repo_path.filter(|value| !value.trim().is_empty()) {
        Path::new(repo_path).join(path_ref)
    } else {
        path_ref.to_path_buf()
    }
}

fn path_has_content(path: &Path) -> bool {
    if let Ok(content) = std::fs::read_to_string(path) {
        return !content.trim().is_empty();
    }

    std::fs::metadata(path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
}

fn requires_full_planning(statement: &str) -> bool {
    const TERMS: &[&str] = &[
        "analyze",
        "api",
        "apis",
        "app",
        "application",
        "architect",
        "architecture",
        "audit",
        "auth",
        "authentication",
        "authorization",
        "backend",
        "build",
        "cli app",
        "database",
        "db",
        "debug",
        "design",
        "diagnose",
        "endpoint",
        "endpoints",
        "fix",
        "frontend",
        "implement",
        "integrate",
        "integration",
        "investigate",
        "login",
        "migration",
        "migrate",
        "multi file",
        "multi-file",
        "multiple files",
        "oauth",
        "persistence",
        "persistent",
        "persist",
        "refactor",
        "repair",
        "rest api",
        "review",
        "schema",
        "server",
        "service",
        "session",
        "site",
        "summarize repo",
        "system",
        "tool",
        "tooling",
        "update existing",
        "website",
        "wire",
        "wiring",
    ];

    TERMS
        .iter()
        .any(|term| contains_normalized_term(statement, term))
}

fn contains_normalized_term(statement: &str, term: &str) -> bool {
    let tokens = statement.split_whitespace().collect::<Vec<_>>();
    let term_tokens = term.split_whitespace().collect::<Vec<_>>();

    match term_tokens.as_slice() {
        [] => false,
        [single] => tokens.iter().any(|token| token == single),
        phrase => tokens.windows(phrase.len()).any(|window| window == phrase),
    }
}

/// V2.3: Plan Quality Scoring - Calibrated for human trust perception
/// A 90% score should feel like "I'd trust this instantly"
pub struct PlanQualityScorer;

/// V2.3: Calibrated scoring weights based on real usage patterns
const WEIGHT_CLARITY: u8 = 30; // Was 25 - clarity matters most for trust
const WEIGHT_SIZING: u8 = 25; // Was 25 - keep balanced
const WEIGHT_DEPENDENCIES: u8 = 25; // Was 25 - correctness critical
const WEIGHT_EFFICIENCY: u8 = 20; // Was 25 - less important than correctness

/// V2.3: Quality metrics with calibrated weights
#[derive(Debug, Clone)]
pub struct PlanQualityScore {
    /// Overall score 0-100
    pub overall: u8,
    /// Step clarity score (0-30) - V2.3: increased weight
    pub clarity: u8,
    /// Step size appropriateness (0-25)
    pub sizing: u8,
    /// Dependency correctness (0-25)
    pub dependencies: u8,
    /// Absence of unnecessary steps (0-20) - V2.3: decreased weight
    pub efficiency: u8,
    /// Specific issues identified
    pub issues: Vec<String>,
    /// Suggestions for improvement
    pub suggestions: Vec<String>,
    /// V2.3: Quality tier for UI presentation
    pub tier: QualityTier,
}

/// V2.3: Human-perceived quality tiers
#[derive(Debug, Clone)]
pub enum QualityTier {
    /// 90-100: "I'd trust this instantly"
    Excellent,
    /// 75-89: "Good plan, minor notes"
    Good,
    /// 60-74: "Needs work but usable"
    Fair,
    /// Below 60: "Significant concerns"
    NeedsImprovement,
}

impl QualityTier {
    pub fn from_score(score: u8) -> Self {
        match score {
            90..=100 => QualityTier::Excellent,
            75..=89 => QualityTier::Good,
            60..=74 => QualityTier::Fair,
            _ => QualityTier::NeedsImprovement,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            QualityTier::Excellent => "✓",
            QualityTier::Good => "○",
            QualityTier::Fair => "△",
            QualityTier::NeedsImprovement => "✗",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            QualityTier::Excellent => "Excellent",
            QualityTier::Good => "Good",
            QualityTier::Fair => "Fair",
            QualityTier::NeedsImprovement => "Needs Work",
        }
    }
}

impl PlanQualityScorer {
    /// Score a generated plan for quality issues
    pub fn score(plan: &GeneratedPlan) -> PlanQualityScore {
        let mut issues = Vec::new();
        let mut suggestions = Vec::new();

        // 1. Step Clarity Scoring (0-25)
        let clarity_score = Self::score_clarity(plan, &mut issues, &mut suggestions);

        // 2. Step Size Scoring (0-25)
        let sizing_score = Self::score_sizing(plan, &mut issues, &mut suggestions);

        // 3. Dependency Scoring (0-25)
        let dependency_score = Self::score_dependencies(plan, &mut issues, &mut suggestions);

        // 4. Efficiency Scoring (0-25)
        let efficiency_score = Self::score_efficiency(plan, &mut issues, &mut suggestions);

        // Calculate overall score
        let overall = clarity_score + sizing_score + dependency_score + efficiency_score;
        let tier = QualityTier::from_score(overall);

        PlanQualityScore {
            overall,
            clarity: clarity_score,
            sizing: sizing_score,
            dependencies: dependency_score,
            efficiency: efficiency_score,
            issues,
            suggestions,
            tier,
        }
    }

    /// V2.3: Calibrated clarity scoring (0-30)
    /// Less harsh on minor issues, focuses on real ambiguity
    fn score_clarity(
        plan: &GeneratedPlan,
        issues: &mut Vec<String>,
        _suggestions: &mut Vec<String>,
    ) -> u8 {
        let mut score = WEIGHT_CLARITY;
        let mut vague_count = 0;

        for (i, step) in plan.steps.iter().enumerate() {
            let step_num = i + 1;

            // V2.3: Refined vague verb detection - only flag truly ambiguous
            let vague_patterns = [
                "do ", "handle ", "process ", "manage ", "work on ", "fix ", "update ",
            ];
            for pattern in &vague_patterns {
                if step.description.to_lowercase().contains(pattern) {
                    vague_count += 1;
                    // Only report first 2 to reduce noise
                    if vague_count <= 2 {
                        issues.push(format!(
                            "Step {} could be more specific: '{}'",
                            step_num, step.description
                        ));
                    }
                    // V2.3: Reduced penalty - 3 points per vague step, max 9
                    score = score.saturating_sub(3);
                }
            }

            // V2.3: Calibrated length checks
            let word_count = step.description.split_whitespace().count();
            if word_count < 2 {
                // Very short descriptions are genuinely unclear
                issues.push(format!(
                    "Step {} is too brief ({} words)",
                    step_num, word_count
                ));
                score = score.saturating_sub(4);
            } else if word_count > 15 {
                // Only flag egregiously long descriptions
                issues.push(format!(
                    "Step {} is quite long ({} words) - consider if it needs splitting",
                    step_num, word_count
                ));
                score = score.saturating_sub(1);
            }
        }

        // Cap penalty from vague verbs
        score.max(WEIGHT_CLARITY - 9)
    }

    /// V2.3: Calibrated sizing scoring (0-25)
    /// Focuses on structural balance over arbitrary step counts
    fn score_sizing(
        plan: &GeneratedPlan,
        issues: &mut Vec<String>,
        suggestions: &mut Vec<String>,
    ) -> u8 {
        let mut score = WEIGHT_SIZING;
        let step_count = plan.steps.len();

        // V2.3: More forgiving step count ranges
        if step_count == 1 {
            // Single step plans are genuinely concerning
            issues.push("Single-step plan may be too large".to_string());
            suggestions.push("Consider what verification steps are needed".to_string());
            score = score.saturating_sub(8);
        } else if step_count > 10 {
            // Only flag truly excessive plans
            issues.push(format!(
                "Plan has {} steps - may benefit from breaking into sub-goals",
                step_count
            ));
            score = score.saturating_sub(3);
        }

        // V2.3: Structural balance check (most important)
        let read_steps = plan
            .steps
            .iter()
            .filter(|s| s.action_type == StepActionType::Read)
            .count();
        let write_steps = plan
            .steps
            .iter()
            .filter(|s| s.action_type == StepActionType::Write)
            .count();
        let validate_steps = plan
            .steps
            .iter()
            .filter(|s| s.action_type == StepActionType::Validate)
            .count();

        if write_steps > 0 && read_steps == 0 {
            // Blind writes are a real problem
            issues.push("Plan modifies without analyzing first".to_string());
            suggestions.push("Add an analysis step before changes".to_string());
            score = score.saturating_sub(7);
        }

        if write_steps > 0 && validate_steps == 0 {
            // No validation after writes
            suggestions.push("Consider adding validation after modifications".to_string());
            score = score.saturating_sub(2);
        }

        score
    }

    /// V2.3: Calibrated dependency scoring (0-25)
    /// Critical correctness issues weighted heavily
    fn score_dependencies(
        plan: &GeneratedPlan,
        issues: &mut Vec<String>,
        _suggestions: &mut Vec<String>,
    ) -> u8 {
        let mut score = WEIGHT_DEPENDENCIES;

        // Check for proper sequencing
        let mut found_write = false;
        let mut last_analysis_idx = None;

        for (i, step) in plan.steps.iter().enumerate() {
            if step.action_type == StepActionType::Read {
                last_analysis_idx = Some(i);
            }
            if step.action_type == StepActionType::Write {
                found_write = true;
            }
            // Validation should come after writes
            if step.action_type == StepActionType::Validate && !found_write && i > 0 {
                issues.push(format!(
                    "Step {} validates but nothing was modified yet",
                    i + 1
                ));
                score = score.saturating_sub(4);
            }
        }

        // V2.3: High-risk steps must have recent analysis
        for (i, step) in plan.steps.iter().enumerate() {
            if step.risk_level == RiskLevel::Warning || step.risk_level == RiskLevel::Critical {
                let has_recent_analysis = last_analysis_idx
                    .map(|idx| i - idx <= 2) // Analysis within 2 steps
                    .unwrap_or(false);

                if i == 0 || !has_recent_analysis {
                    issues.push(format!(
                        "Step {} is high-risk without recent analysis",
                        i + 1
                    ));
                    score = score.saturating_sub(6);
                }
            }
        }

        score
    }

    /// V2.3: Calibrated efficiency scoring (0-20)
    /// Only flag clear redundancy, ignore minor optimization opportunities
    fn score_efficiency(
        plan: &GeneratedPlan,
        issues: &mut Vec<String>,
        _suggestions: &mut Vec<String>,
    ) -> u8 {
        let mut score = WEIGHT_EFFICIENCY;

        // V2.3: Only flag 3+ consecutive reads as truly redundant
        let mut consecutive_reads = 0;
        let mut read_start = 0;

        for (i, step) in plan.steps.iter().enumerate() {
            if step.action_type == StepActionType::Read {
                if consecutive_reads == 0 {
                    read_start = i;
                }
                consecutive_reads += 1;
            } else {
                if consecutive_reads >= 3 {
                    issues.push(format!(
                        "Steps {}-{} are consecutive analysis - may be combinable",
                        read_start + 1,
                        i
                    ));
                    score = score.saturating_sub(4);
                }
                consecutive_reads = 0;
            }
        }

        score
    }

    /// V2.3: Generate quality report with tier-based presentation
    pub fn format_score(score: &PlanQualityScore) -> String {
        let mut text = String::new();

        // V2.3: Tier-based compact header
        text.push_str(&format!(
            "📊 Plan Quality: {} {} ({}%)\n",
            score.tier.icon(),
            score.tier.label(),
            score.overall
        ));

        // V2.3: Score breakdown with calibrated weights
        text.push_str(&format!(
            "   Clarity: {}/30 | Sizing: {}/25 | Dependencies: {}/25 | Efficiency: {}/20\n",
            score.clarity, score.sizing, score.dependencies, score.efficiency
        ));

        // Issues (if any)
        if !score.issues.is_empty() {
            text.push_str("\n⚠️  Issues Found:\n");
            for issue in score.issues.iter().take(3) {
                text.push_str(&format!("   • {}\n", issue));
            }
            if score.issues.len() > 3 {
                text.push_str(&format!("   ... and {} more\n", score.issues.len() - 3));
            }
        }

        // Suggestions (if quality < 90)
        if score.overall < 90 && !score.suggestions.is_empty() {
            text.push_str("\n💡 Suggestions:\n");
            for suggestion in score.suggestions.iter().take(2) {
                text.push_str(&format!("   • {}\n", suggestion));
            }
        }

        text
    }
}

/// V2.0 Plan Explanation - Before execution explanation
pub struct PlanExplanation;

impl PlanExplanation {
    /// Generate comprehensive plan explanation for operator review
    pub fn explain(plan: &GeneratedPlan, goal: &Goal) -> String {
        let mut text = String::new();

        // Header
        text.push_str(&format!("🎯 Proposed Plan for Goal\n"));
        text.push_str(&format!("   {}\n\n", goal.statement));

        // What it will do
        text.push_str("📋 What It Will Do\n");
        text.push_str(&format!("   {}\n", plan.objective));
        text.push_str(&format!("   {} steps total\n\n", plan.steps.len()));

        // Steps
        text.push_str("🔢 Execution Steps:\n");
        for step in &plan.steps {
            let risk_icon = match step.risk_level {
                RiskLevel::Safe => "✓",
                RiskLevel::Caution => "⚠",
                RiskLevel::Warning => "▲",
                RiskLevel::Critical => "✗",
            };
            let approval_icon = if step.likely_approval_needed {
                "⏸ "
            } else {
                "  "
            };
            text.push_str(&format!(
                "   {} {} {}. {}\n",
                approval_icon, risk_icon, step.number, step.description
            ));
        }
        text.push('\n');

        // Why this plan
        text.push_str("🧠 Why This Plan\n");
        text.push_str(&format!("   {}\n\n", plan.reasoning));

        // Risks
        if !plan.risks.is_empty() {
            text.push_str("⚠️  Risks Detected\n");
            for risk in &plan.risks {
                text.push_str(&format!(
                    "   {}: {}\n",
                    risk.level.icon(),
                    risk.risk_type.name()
                ));
                text.push_str(&format!("      {}\n", risk.description));
                text.push_str(&format!("      → {}\n", risk.mitigation));
            }
            text.push('\n');
        } else {
            text.push_str("✓ No significant risks detected\n\n");
        }

        // Approvals
        if !plan.approval_points.is_empty() {
            text.push_str(&format!(
                "⏸  Approvals Required ({} point{})\n",
                plan.approval_points.len(),
                if plan.approval_points.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
            for &point in &plan.approval_points {
                if let Some(step) = plan.steps.get(point - 1) {
                    text.push_str(&format!("   • Step {}: {}\n", point, step.description));
                }
            }
            text.push('\n');
        }

        // Expected outcome
        text.push_str(&format!(
            "📊 Expected Outcome: {}\n",
            plan.estimated_outcome.description()
        ));

        // Safe to chain indicator
        if plan.safe_to_chain {
            text.push_str("✓ Safe to auto-execute\n");
        } else {
            text.push_str("⚠️  Requires step-by-step execution\n");
        }

        // Next action
        text.push_str("\n▶ Next Action:\n");
        if plan.safe_to_chain {
            text.push_str("   /goal confirm - Accept plan and execute\n");
        } else {
            text.push_str("   /goal confirm - Accept plan (will stop at approvals)\n");
        }
        text.push_str("   /goal reject  - Reject and try different approach\n");
        text.push_str("   /preview      - See detailed execution preview\n");

        text
    }
}

/// V2.0 Completion Explanation - Post-execution report
pub struct CompletionExplanation;

impl CompletionExplanation {
    /// Generate end-to-end completion report
    /// V1.5 UNIFICATION: Renders entirely from authoritative ExecutionOutcome
    pub fn generate(
        goal: &Goal,
        chain: &crate::persistence::PersistentChain,
        outcome: crate::persistence::ExecutionOutcome,
    ) -> String {
        let mut text = String::new();

        // Header - from canonical outcome mapping
        let status_icon = outcome.icon();
        let status_label = outcome.label();
        text.push_str(&format!("{} {}\n\n", status_icon, status_label));
        text.push_str(&format!("Goal: {}\n", goal.statement));
        text.push_str(&format!("Chain: {}\n", chain.name));
        text.push('\n');

        // What was done
        text.push_str("📋 What Was Done\n");
        let completed_steps: Vec<_> = chain
            .steps
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.status, crate::persistence::ChainStepStatus::Completed))
            .collect();

        for (idx, step) in &completed_steps {
            text.push_str(&format!("   ✓ Step {}: {}\n", idx + 1, step.description));
        }

        let failed_steps: Vec<_> = chain
            .steps
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.status, crate::persistence::ChainStepStatus::Failed))
            .collect();

        if !failed_steps.is_empty() {
            text.push('\n');
            for (idx, step) in &failed_steps {
                text.push_str(&format!(
                    "   ✗ Step {}: {} - {}\n",
                    idx + 1,
                    step.description,
                    step.error_message
                        .as_ref()
                        .unwrap_or(&"Unknown error".to_string())
                ));
            }
        }
        text.push('\n');

        // Why it was done
        if let Some(plan) = &goal.generated_plan {
            text.push_str("🧠 Why It Was Done\n");
            text.push_str(&format!("   {}\n\n", plan.reasoning));
        }

        // What changed
        text.push_str("📝 Changes Made\n");
        // Summary from chain replay records
        let total_replay_records = chain
            .steps
            .iter()
            .filter(|s| s.replay_record.is_some())
            .count();
        text.push_str(&format!(
            "   {} step(s) have replay records\n",
            total_replay_records
        ));
        text.push_str("   /replay to see detailed changes\n\n");

        // Validation status
        text.push_str("✓ Validation Status\n");
        let all_validated = chain
            .steps
            .iter()
            .all(|s| s.validation_passed.unwrap_or(false));
        if all_validated {
            text.push_str("   All validations passed\n");
        } else {
            text.push_str("   Some validations pending or failed\n");
        }
        text.push('\n');

        // Replay/audit status
        text.push_str("🔄 Replay/Audit\n");
        if chain.archived {
            text.push_str("   ✓ Chain archived for audit\n");
        } else {
            text.push_str(&format!(
                "   Chain status: {} (available for replay)\n",
                chain.status_string()
            ));
        }
        text.push('\n');

        // Approvals required
        let checkpoint_count = chain.pending_checkpoint.as_ref().map(|_| 1).unwrap_or(0);
        if checkpoint_count > 0 {
            text.push_str(&format!(
                "⏸  Approvals Required: {} checkpoint(s) encountered\n\n",
                checkpoint_count
            ));
        }

        // Suggested next actions - from canonical outcome mapping
        text.push_str("→ Suggested Next Actions\n");
        match outcome {
            crate::persistence::ExecutionOutcome::Success => {
                text.push_str("   /goal <new goal> - Start next goal\n");
                text.push_str("   /replay          - Review execution\n");
                text.push_str("   /chain archive   - Archive completed chain\n");
            }
            crate::persistence::ExecutionOutcome::SuccessWithWarnings => {
                text.push_str("   /goal <new goal> - Start next goal\n");
                text.push_str("   /replay          - Review warnings\n");
                text.push_str("   /chain archive   - Archive completed chain\n");
            }
            crate::persistence::ExecutionOutcome::Blocked => {
                text.push_str("   /chain resume    - Continue blocked chain\n");
                text.push_str("   /replay          - Review and diagnose\n");
                text.push_str("   /goal <new goal> - Try different approach\n");
            }
            crate::persistence::ExecutionOutcome::Failed => {
                text.push_str("   /chain resume    - Retry failed step\n");
                text.push_str("   /replay          - Review and diagnose\n");
                text.push_str("   /goal <new goal> - Try different approach\n");
            }
        }

        text
    }
}

/// V2.0 Goal Manager - Tracks active goals
#[derive(Debug, Clone, Default)]
pub struct GoalManager {
    /// Active goals by ID
    pub goals: std::collections::HashMap<String, Goal>,
    /// Currently active goal ID
    pub active_goal_id: Option<String>,
}

impl GoalManager {
    /// Create new goal manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Stake a new goal
    pub fn stake_goal(
        &mut self,
        statement: impl Into<String>,
        conversation_id: impl Into<String>,
    ) -> Goal {
        let goal = Goal::new(statement, conversation_id);
        self.goals.insert(goal.id.clone(), goal.clone());
        self.active_goal_id = Some(goal.id.clone());
        goal
    }

    /// Get active goal
    pub fn active_goal(&self) -> Option<&Goal> {
        self.active_goal_id
            .as_ref()
            .and_then(|id| self.goals.get(id))
    }

    /// Get active goal mutably
    pub fn active_goal_mut(&mut self) -> Option<&mut Goal> {
        self.active_goal_id
            .as_ref()
            .and_then(|id| self.goals.get_mut(id))
    }

    /// Get goal by ID
    pub fn get(&self, id: &str) -> Option<&Goal> {
        self.goals.get(id)
    }

    /// Mark active goal as executing
    pub fn mark_executing(&mut self) {
        if let Some(goal) = self.active_goal_mut() {
            goal.mark_executing();
        }
    }

    /// Mark active goal as completed
    pub fn mark_completed(&mut self, summary: impl Into<String>) {
        if let Some(goal) = self.active_goal_mut() {
            goal.mark_completed(summary);
        }
    }

    /// Mark active goal as failed
    pub fn mark_failed(&mut self) {
        if let Some(goal) = self.active_goal_mut() {
            goal.mark_failed();
        }
    }

    /// V2.2: Store rejected plan for diff comparison
    pub fn store_rejected_plan(&mut self, plan: GeneratedPlan, reason: impl Into<String>) {
        if let Some(goal) = self.active_goal_mut() {
            goal.previous_plan = Some(plan);
            goal.rejection_reason = Some(reason.into());
        }
    }

    /// V2.2: Get previous rejected plan for comparison
    pub fn previous_plan(&self) -> Option<&GeneratedPlan> {
        self.active_goal().and_then(|g| g.previous_plan.as_ref())
    }

    /// V2.2: Get last rejection reason
    pub fn last_rejection_reason(&self) -> Option<String> {
        self.active_goal().and_then(|g| g.rejection_reason.clone())
    }

    /// V2.2: Get last goal statement (for continuity)
    pub fn last_goal_statement(&self) -> Option<String> {
        self.active_goal().map(|g| g.statement.clone())
    }
}

/// V2.2: Plan Diff - Compare new plan to previous rejected plan
pub struct PlanDiff;

impl PlanDiff {
    /// Generate diff between previous and current plan
    pub fn compare(previous: &GeneratedPlan, current: &GeneratedPlan) -> PlanDiffResult {
        let mut changes = Vec::new();

        // Compare step counts
        if previous.steps.len() != current.steps.len() {
            if current.steps.len() > previous.steps.len() {
                changes.push(PlanChange::StepSplit {
                    old_count: previous.steps.len(),
                    new_count: current.steps.len(),
                });
            } else {
                changes.push(PlanChange::StepsCombined {
                    old_count: previous.steps.len(),
                    new_count: current.steps.len(),
                });
            }
        }

        // Compare risk levels
        let prev_max_risk = previous
            .steps
            .iter()
            .map(|s| s.risk_level as u8)
            .max()
            .unwrap_or(0);
        let curr_max_risk = current
            .steps
            .iter()
            .map(|s| s.risk_level as u8)
            .max()
            .unwrap_or(0);
        if prev_max_risk != curr_max_risk {
            changes.push(PlanChange::RiskChanged {
                old_risk: Self::risk_label(prev_max_risk),
                new_risk: Self::risk_label(curr_max_risk),
            });
        }

        // Compare approval points
        if previous.approval_points.len() != current.approval_points.len() {
            changes.push(PlanChange::ApprovalCountChanged {
                old_count: previous.approval_points.len(),
                new_count: current.approval_points.len(),
            });
        }

        // Compare safe-to-chain
        if previous.safe_to_chain != current.safe_to_chain {
            if current.safe_to_chain {
                changes.push(PlanChange::NowAutoExecutable);
            } else {
                changes.push(PlanChange::NoLongerAutoExecutable);
            }
        }

        PlanDiffResult { changes }
    }

    fn risk_label(risk_val: u8) -> String {
        match risk_val {
            0 => "Safe".to_string(),
            1 => "Caution".to_string(),
            2 => "Warning".to_string(),
            3 => "Critical".to_string(),
            _ => "Unknown".to_string(),
        }
    }
}

/// A single change between plans
#[derive(Debug, Clone)]
pub enum PlanChange {
    /// Steps were split into more steps
    StepSplit { old_count: usize, new_count: usize },
    /// Steps were combined into fewer steps
    StepsCombined { old_count: usize, new_count: usize },
    /// Risk level changed
    RiskChanged { old_risk: String, new_risk: String },
    /// Approval point count changed
    ApprovalCountChanged { old_count: usize, new_count: usize },
    /// Plan became auto-executable
    NowAutoExecutable,
    /// Plan no longer auto-executable
    NoLongerAutoExecutable,
}

/// Result of comparing two plans
#[derive(Debug, Clone)]
pub struct PlanDiffResult {
    pub changes: Vec<PlanChange>,
}

impl PlanDiffResult {
    /// Format diff for display
    pub fn format(&self) -> String {
        if self.changes.is_empty() {
            return "Plan is similar to previous attempt".to_string();
        }

        let mut text = "Changes from previous plan:\n".to_string();
        for change in &self.changes {
            match change {
                PlanChange::StepSplit {
                    old_count,
                    new_count,
                } => {
                    text.push_str(&format!(
                        "  • Steps expanded: {} → {} steps\n",
                        old_count, new_count
                    ));
                }
                PlanChange::StepsCombined {
                    old_count,
                    new_count,
                } => {
                    text.push_str(&format!(
                        "  • Steps consolidated: {} → {} steps\n",
                        old_count, new_count
                    ));
                }
                PlanChange::RiskChanged { old_risk, new_risk } => {
                    text.push_str(&format!("  • Risk level: {} → {}\n", old_risk, new_risk));
                }
                PlanChange::ApprovalCountChanged {
                    old_count,
                    new_count,
                } => {
                    text.push_str(&format!(
                        "  • Approval points: {} → {}\n",
                        old_count, new_count
                    ));
                }
                PlanChange::NowAutoExecutable => {
                    text.push_str("  • Now safe to auto-execute ✓\n");
                }
                PlanChange::NoLongerAutoExecutable => {
                    text.push_str("  • Now requires step-by-step execution\n");
                }
            }
        }
        text
    }

    /// Check if any significant improvements were made
    pub fn has_improvements(&self) -> bool {
        self.changes.iter().any(|c| match c {
            PlanChange::StepsCombined { .. } => true,
            PlanChange::NowAutoExecutable => true,
            PlanChange::RiskChanged { new_risk, .. }
                if new_risk == "Safe" || new_risk == "Caution" =>
            {
                true
            }
            PlanChange::ApprovalCountChanged { new_count, .. } if *new_count == 0 => true,
            _ => false,
        })
    }
}

/// V2.3: Execution Drift Detection - Calibrated sensitivity with severity levels
/// Fires when a human would say "okay yeah something's off"
pub struct ExecutionDriftDetector {
    /// Expected approval points
    expected_approvals: Vec<usize>,
    /// Step execution times (step_num -> duration_ms)
    step_durations: std::collections::HashMap<usize, u64>,
    /// Unexpected approvals by severity
    unexpected_approvals: Vec<(usize, DriftSeverity)>,
    /// Slow steps by severity
    slow_steps: Vec<(usize, u64, DriftSeverity)>,
    /// Total steps in plan
    total_steps: usize,
}

/// V2.3: Drift severity classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftSeverity {
    /// Info: Notable but not concerning (1+ min delays, minor surprises)
    Info,
    /// Warning: Worth attention (3+ min delays, unexpected approvals)
    Warning,
    /// Critical: Execution significantly off track (5+ min, multiple surprises)
    Critical,
}

impl DriftSeverity {
    pub fn icon(&self) -> &'static str {
        match self {
            DriftSeverity::Info => "ℹ️",
            DriftSeverity::Warning => "⚠️",
            DriftSeverity::Critical => "🚨",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DriftSeverity::Info => "Note",
            DriftSeverity::Warning => "Warning",
            DriftSeverity::Critical => "Critical",
        }
    }
}

/// V2.3: Calibrated thresholds for meaningful drift detection
const THRESHOLD_SLOW_INFO: u64 = 60000; // 1 minute
const THRESHOLD_SLOW_WARNING: u64 = 180000; // 3 minutes
const THRESHOLD_SLOW_CRITICAL: u64 = 300000; // 5 minutes

impl ExecutionDriftDetector {
    /// Create new drift detector from a plan
    pub fn from_plan(plan: &GeneratedPlan) -> Self {
        Self {
            expected_approvals: plan.approval_points.clone(),
            step_durations: std::collections::HashMap::new(),
            unexpected_approvals: Vec::new(),
            slow_steps: Vec::new(),
            total_steps: plan.steps.len(),
        }
    }

    /// Record step completion with calibrated duration thresholds
    pub fn record_step_complete(&mut self, step_num: usize, duration_ms: u64) {
        self.step_durations.insert(step_num, duration_ms);

        // V2.3: Severity-based threshold detection
        let severity = if duration_ms >= THRESHOLD_SLOW_CRITICAL {
            DriftSeverity::Critical
        } else if duration_ms >= THRESHOLD_SLOW_WARNING {
            DriftSeverity::Warning
        } else if duration_ms >= THRESHOLD_SLOW_INFO {
            DriftSeverity::Info
        } else {
            return; // Not slow enough to track
        };

        self.slow_steps.push((step_num, duration_ms, severity));
    }

    /// Record an approval checkpoint with severity
    pub fn record_approval(&mut self, step_num: usize, risk_level: RiskLevel) {
        // V2.3: Only flag unexpected approvals
        if self.expected_approvals.contains(&step_num) {
            return;
        }

        // V2.3: Severity based on risk level
        let severity = match risk_level {
            RiskLevel::Critical => DriftSeverity::Critical,
            RiskLevel::Warning => DriftSeverity::Warning,
            _ => DriftSeverity::Info,
        };

        self.unexpected_approvals.push((step_num, severity));
    }

    /// V2.3: Get maximum drift severity
    pub fn max_severity(&self) -> Option<DriftSeverity> {
        let approval_max = self
            .unexpected_approvals
            .iter()
            .map(|(_, s)| *s)
            .max_by_key(|s| match s {
                DriftSeverity::Info => 0,
                DriftSeverity::Warning => 1,
                DriftSeverity::Critical => 2,
            });

        let slow_max = self
            .slow_steps
            .iter()
            .map(|(_, _, s)| *s)
            .max_by_key(|s| match s {
                DriftSeverity::Info => 0,
                DriftSeverity::Warning => 1,
                DriftSeverity::Critical => 2,
            });

        // Return highest severity
        match (approval_max, slow_max) {
            (Some(DriftSeverity::Critical), _) | (_, Some(DriftSeverity::Critical)) => {
                Some(DriftSeverity::Critical)
            }
            (Some(DriftSeverity::Warning), _) | (_, Some(DriftSeverity::Warning)) => {
                Some(DriftSeverity::Warning)
            }
            (Some(DriftSeverity::Info), _) | (_, Some(DriftSeverity::Info)) => {
                Some(DriftSeverity::Info)
            }
            _ => None,
        }
    }

    /// V2.3: Check if drift warrants notification (Warning or higher)
    pub fn has_significant_drift(&self) -> bool {
        matches!(
            self.max_severity(),
            Some(DriftSeverity::Warning) | Some(DriftSeverity::Critical)
        )
    }

    /// V2.3: Generate calibrated drift report
    pub fn generate_report(&self) -> Option<String> {
        let severity = self.max_severity()?;

        // V2.3: Only report Warning or higher (suppress Info noise)
        if severity == DriftSeverity::Info {
            return None;
        }

        let mut text = String::new();
        text.push_str(&format!(
            "{} Execution Drift: {}\n\n",
            severity.icon(),
            severity.label()
        ));

        // Critical: Multiple unexpected approvals
        let critical_approvals: Vec<_> = self
            .unexpected_approvals
            .iter()
            .filter(|(_, s)| *s == DriftSeverity::Critical)
            .collect();
        if !critical_approvals.is_empty() {
            text.push_str("Unexpected high-risk approvals at:\n");
            for (step, _) in critical_approvals {
                text.push_str(&format!("  • Step {}\n", step));
            }
            text.push('\n');
        }

        // Warning: Other unexpected approvals
        let warning_approvals: Vec<_> = self
            .unexpected_approvals
            .iter()
            .filter(|(_, s)| *s == DriftSeverity::Warning)
            .collect();
        if !warning_approvals.is_empty() {
            text.push_str("Additional approvals needed at:\n");
            for (step, _) in warning_approvals {
                text.push_str(&format!("  • Step {}\n", step));
            }
            text.push('\n');
        }

        // Slow steps
        let slow_warnings: Vec<_> = self
            .slow_steps
            .iter()
            .filter(|(_, _, s)| *s != DriftSeverity::Info)
            .collect();
        if !slow_warnings.is_empty() {
            text.push_str("Steps taking longer than expected:\n");
            for (step, duration, sev) in slow_warnings {
                let seconds = duration / 1000;
                let minutes = seconds / 60;
                let secs = seconds % 60;
                text.push_str(&format!(
                    "  • Step {}: {}m{}s {}\n",
                    step,
                    minutes,
                    secs,
                    sev.icon()
                ));
            }
            text.push('\n');
        }

        if severity == DriftSeverity::Critical {
            text.push_str("Plan significantly off track. Consider:\n");
            text.push_str("  • /stop to halt execution\n");
            text.push_str("  • /goal to replan with new information\n");
        } else {
            text.push_str("Execution slower than planned.\n");
            text.push_str("If stuck: /goal <refined approach>\n");
        }

        Some(text)
    }
}

/// V2.2: Goal Memory for continuity and learning
#[derive(Debug, Clone, Default)]
pub struct GoalMemory {
    /// Last goal statement
    pub last_goal: Option<String>,
    /// Last rejection reason
    pub last_rejection: Option<String>,
    /// Last failure reason
    pub last_failure: Option<String>,
    /// Pattern of rejections (for trend detection)
    pub rejection_count: u32,
    /// Successful goal patterns
    pub successful_patterns: Vec<String>,
}

impl GoalMemory {
    /// Record a goal rejection
    pub fn record_rejection(&mut self, goal: &str, reason: &str) {
        self.last_goal = Some(goal.to_string());
        self.last_rejection = Some(reason.to_string());
        self.rejection_count += 1;
    }

    /// Record a goal success
    pub fn record_success(&mut self, goal: &str) {
        self.last_goal = Some(goal.to_string());
        self.last_rejection = None;
        self.successful_patterns.push(goal.to_string());
    }

    /// Record a goal failure
    pub fn record_failure(&mut self, goal: &str, reason: &str) {
        self.last_goal = Some(goal.to_string());
        self.last_failure = Some(reason.to_string());
    }

    /// Check if user is in a rejection spiral
    pub fn is_rejection_spiral(&self) -> bool {
        self.rejection_count >= 3
    }

    /// Generate guidance based on memory
    pub fn guidance(&self) -> Option<String> {
        if self.is_rejection_spiral() {
            return Some(
                "🔄 Multiple rejections detected. Consider:\n  • Breaking into smaller goals\n  • Using /plan for manual control\n  • Checking current state with /status".to_string()
            );
        }

        if let Some(ref last) = self.last_rejection {
            if last.contains("risk") {
                return Some(
                    "💡 Last goal was rejected due to risk. Try a safer, smaller first step."
                        .to_string(),
                );
            }
        }

        None
    }
}

/// V2.3: Execution Confidence Signal - Emotional anchor before confirmation
/// Provides clear confidence indicator based on plan quality, goal class, and memory
pub struct ExecutionConfidence;

/// Confidence level for execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceLevel {
    /// High: 90%+ quality, safe pattern, no memory issues
    High,
    /// Medium: 75-89% quality, or known pattern
    Medium,
    /// Low: <75% quality, or similar goal previously rejected
    Low,
    /// Uncertain: Critical risks, or multiple recent rejections
    Uncertain,
}

impl ConfidenceLevel {
    pub fn icon(&self) -> &'static str {
        match self {
            ConfidenceLevel::High => "🟢",
            ConfidenceLevel::Medium => "🟡",
            ConfidenceLevel::Low => "🟠",
            ConfidenceLevel::Uncertain => "🔴",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ConfidenceLevel::High => "High",
            ConfidenceLevel::Medium => "Medium",
            ConfidenceLevel::Low => "Low",
            ConfidenceLevel::Uncertain => "Uncertain",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ConfidenceLevel::High => "Plan quality is excellent and matches successful patterns",
            ConfidenceLevel::Medium => "Plan is solid with minor considerations",
            ConfidenceLevel::Low => "Plan has issues or similar goal was rejected before",
            ConfidenceLevel::Uncertain => "Significant risks or multiple recent rejections",
        }
    }
}

impl ExecutionConfidence {
    /// Calculate confidence level for a plan
    pub fn calculate(
        plan: &GeneratedPlan,
        quality_score: &PlanQualityScore,
        memory: Option<&GoalMemory>,
    ) -> ConfidenceLevel {
        // Start with quality-based confidence
        let base_confidence = if quality_score.overall >= 90 {
            ConfidenceLevel::High
        } else if quality_score.overall >= 75 {
            ConfidenceLevel::Medium
        } else if quality_score.overall >= 60 {
            ConfidenceLevel::Low
        } else {
            ConfidenceLevel::Uncertain
        };

        // Adjust based on critical risks
        let has_critical = plan.risks.iter().any(|r| r.level == RiskLevel::Critical);
        if has_critical {
            return ConfidenceLevel::Uncertain;
        }

        // Adjust based on memory
        if let Some(mem) = memory {
            // If similar goal was rejected recently, lower confidence
            if mem.rejection_count > 0 {
                return match base_confidence {
                    ConfidenceLevel::High => ConfidenceLevel::Medium,
                    ConfidenceLevel::Medium => ConfidenceLevel::Low,
                    _ => ConfidenceLevel::Uncertain,
                };
            }

            // If in rejection spiral, uncertain
            if mem.is_rejection_spiral() {
                return ConfidenceLevel::Uncertain;
            }
        }

        // Safe to chain boosts confidence if not already high
        if plan.safe_to_chain && matches!(base_confidence, ConfidenceLevel::Medium) {
            return ConfidenceLevel::High;
        }

        base_confidence
    }

    /// Format confidence for display in plan explanation
    pub fn format(confidence: ConfidenceLevel) -> String {
        format!(
            "Confidence: {} {}\n{}",
            confidence.icon(),
            confidence.label(),
            confidence.description()
        )
    }
}

/// V2.3: Smarter Replanning - Use memory to avoid rejected patterns
pub struct SmartReplanning;

impl SmartReplanning {
    /// Generate explicit guidance when replanning based on previous rejection
    pub fn generate_replanning_guidance(
        previous_plan: Option<&GeneratedPlan>,
        rejection_reason: Option<&str>,
        new_plan: &GeneratedPlan,
    ) -> Option<String> {
        let prev = previous_plan?;
        let _reason = rejection_reason?;

        let mut guidance = Vec::new();

        // Check what changed in a meaningful way
        let prev_steps = prev.steps.len();
        let new_steps = new_plan.steps.len();
        let prev_approvals = prev.approval_points.len();
        let new_approvals = new_plan.approval_points.len();
        let prev_risk = prev
            .steps
            .iter()
            .map(|s| s.risk_level as u8)
            .max()
            .unwrap_or(0);
        let new_risk = new_plan
            .steps
            .iter()
            .map(|s| s.risk_level as u8)
            .max()
            .unwrap_or(0);

        // Explicitly call out improvements
        if new_steps > prev_steps {
            guidance.push(format!(
                "Plan broken into {} smaller steps (was {})",
                new_steps, prev_steps
            ));
        }

        if new_approvals < prev_approvals {
            guidance.push(format!(
                "Reduced to {} approval point(s) (was {})",
                new_approvals, prev_approvals
            ));
        }

        if new_risk < prev_risk {
            guidance.push("Lower risk approach selected".to_string());
        }

        if new_plan.safe_to_chain && !prev.safe_to_chain {
            guidance.push("Now safe to auto-execute".to_string());
        }

        if guidance.is_empty() {
            return None;
        }

        let mut text = "🔄 Changes based on previous feedback:\n".to_string();
        for item in guidance {
            text.push_str(&format!("  • {}\n", item));
        }

        Some(text)
    }

    /// Generate warning if new plan repeats patterns from rejected plan
    pub fn detect_repeated_patterns(
        previous_plan: Option<&GeneratedPlan>,
        new_plan: &GeneratedPlan,
    ) -> Option<String> {
        let prev = previous_plan?;

        // Check if plans are too similar
        let prev_descriptions: Vec<_> = prev.steps.iter().map(|s| s.description.clone()).collect();
        let new_descriptions: Vec<_> = new_plan
            .steps
            .iter()
            .map(|s| s.description.clone())
            .collect();

        // If step descriptions are identical, warn
        if prev_descriptions == new_descriptions {
            return Some(
                "⚠️  New plan is identical to rejected plan.\n\
                 Consider a different approach or be more specific about what to change."
                    .to_string(),
            );
        }

        // If approval count is the same and >0, note it
        if prev.approval_points.len() == new_plan.approval_points.len()
            && !prev.approval_points.is_empty()
        {
            return Some(format!(
                "ℹ️  Still requires {} approval point(s).\n\
                 If this was an issue, try a safer first step.",
                new_plan.approval_points.len()
            ));
        }

        None
    }
}

// Helper trait for chain status display
pub trait ChainStatusDisplay {
    fn status_string(&self) -> String;
}

impl ChainStatusDisplay for crate::persistence::PersistentChain {
    fn status_string(&self) -> String {
        format!("{:?}", self.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn literal_plan(statement: &str) -> GeneratedPlan {
        let goal = Goal::new(statement, "test-conversation");
        match PlanEngine::generate(&goal, &AppState::default(), &PersistentState::default()) {
            PlanGenerationResult::Success(plan) => plan,
            PlanGenerationResult::Failed { reason, .. } => {
                panic!("expected literal plan, got failure: {}", reason)
            }
        }
    }

    fn assert_full_planning_route(statement: &str) {
        let goal = Goal::new(statement, "test-conversation");
        assert!(
            PlanEngine::generate_literal_creation(&goal).is_none(),
            "expected full planner route for: {}",
            statement
        );

        let result = PlanEngine::generate(&goal, &AppState::default(), &PersistentState::default());
        let PlanGenerationResult::Success(plan) = result else {
            panic!("expected generated full plan for: {}", statement);
        };

        let all_steps = plan
            .steps
            .iter()
            .map(|step| step.description.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !all_steps.contains("Target artifact:"),
            "full planner route must not use literal artifact marker for: {}",
            statement
        );
    }

    fn assert_direct_literal_plan(plan: &GeneratedPlan, target_path: &str, artifact_class: &str) {
        assert!(plan.safe_to_chain);
        assert!(plan.approval_points.is_empty());
        assert!(plan.required_context.is_empty());
        assert_eq!(plan.steps[0].action_type, StepActionType::Write);
        assert!(plan.steps[0].description.contains("Target artifact:"));
        assert!(plan.steps[0].description.contains(target_path));
        assert!(plan.steps[0].description.contains(artifact_class));
        assert!(
            plan.steps
                .iter()
                .any(|step| step.affected_files.contains(&target_path.to_string()))
        );

        let all_steps = plan
            .steps
            .iter()
            .map(|step| step.description.to_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!all_steps.contains("identify implementation location"));
        assert!(!all_steps.contains("analyze goal requirements"));
        assert!(!all_steps.contains("repository analysis"));
    }

    #[test]
    fn extracts_numbered_markdown_filename_contract() {
        let statement = "Produce exactly 15 markdown files.\n\
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
All of these must be produced.";

        let contract =
            extract_explicit_artifact_contract(statement).expect("explicit contract detected");

        assert_eq!(contract.artifact_type.as_deref(), Some("markdown"));
        assert_eq!(contract.required_count, Some(15));
        assert_eq!(contract.required_filenames.len(), 15);
        assert_eq!(
            contract.required_filenames.first().map(String::as_str),
            Some("docs/01_PROJECT_OVERVIEW.md")
        );
        assert_eq!(
            contract.required_filenames.last().map(String::as_str),
            Some("docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md")
        );
    }

    #[test]
    fn explicit_artifact_plan_decomposes_each_required_file() {
        let statement = "Create exactly 15 markdown files with these precise filenames:\n\
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
15. docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md";
        let goal = Goal::new(statement, "test-conversation");
        let plan = PlanEngine::generate_explicit_artifact_plan(&goal, None)
            .expect("explicit artifact plan generated");

        assert_eq!(plan.steps.len(), 17);
        assert_eq!(plan.steps[0].action_type, StepActionType::Read);
        assert_eq!(
            plan.steps.last().map(|step| step.action_type),
            Some(StepActionType::Validate)
        );
        assert!(plan.required_context.contains(&"docs/01_PROJECT_OVERVIEW.md".to_string()));
        assert!(plan.steps.iter().any(|step| {
            step.description
                .contains("docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md")
                && step.action_type == StepActionType::Write
        }));
        assert!(
            plan.reasoning.contains("artifact CRUD semantics"),
            "reasoning should explain why the plan is decomposed"
        );
    }

    #[test]
    fn explicit_artifact_plan_uses_create_update_and_replace_semantics() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("docs")).expect("docs dir");
        fs::write(
            temp.path().join("docs/01_PROJECT_OVERVIEW.md"),
            "# existing overview\n",
        )
        .expect("existing file");
        fs::write(temp.path().join("docs/02_ARCHITECTURE.md"), "   ").expect("empty file");

        let statement = "Create exactly 3 markdown files with these precise filenames:\n\
1. docs/01_PROJECT_OVERVIEW.md\n\
2. docs/02_ARCHITECTURE.md\n\
3. docs/03_TECHNOLOGY_STACK.md";
        let goal = Goal::new(statement, "test-conversation");
        let plan = PlanEngine::generate_explicit_artifact_plan(
            &goal,
            Some(temp.path().to_string_lossy().as_ref()),
        )
        .expect("explicit artifact plan generated");

        assert!(plan.steps.iter().any(|step| {
            step.description.contains("Update existing required markdown artifact docs/01_PROJECT_OVERVIEW.md")
        }));
        assert!(plan.steps.iter().any(|step| {
            step.description.contains("Replace empty required markdown artifact docs/02_ARCHITECTURE.md")
        }));
        assert!(plan.steps.iter().any(|step| {
            step.description.contains("Create missing required markdown artifact docs/03_TECHNOLOGY_STACK.md")
        }));
        assert!(
            plan.steps[0]
                .description
                .contains("Current state: 1 existing, 1 missing, 1 empty"),
            "inventory step should report current CRUD state"
        );
    }

    #[test]
    fn objective_summary_and_raw_prompt_are_preserved_for_large_contracts() {
        let statement = "Create exactly 15 markdown files with these precise filenames:\n\
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
All of these must be produced.";
        let goal = Goal::new(statement, "test-conversation");
        let plan = PlanEngine::generate_explicit_artifact_plan(&goal, None)
            .expect("explicit artifact plan generated");

        assert_eq!(plan.raw_prompt, statement);
        assert_eq!(
            plan.objective,
            "Generate exactly 15 markdown file(s) with the specified filenames"
        );
        assert!(!is_vague_objective_summary(&plan.objective));
    }

    #[test]
    fn extracts_filename_purposes_from_numbered_contract_lines() {
        let statement = "Create exactly 2 markdown files:\n\
1. docs/01_PROJECT_OVERVIEW.md - explain the product scope, operators, and outcomes.\n\
2. docs/02_ARCHITECTURE.md: describe the runtime architecture, boundaries, and critical flows.\n\
All of these must be produced.";

        let contract =
            extract_explicit_artifact_contract(statement).expect("explicit contract detected");

        assert_eq!(
            contract.purpose_for_path("docs/01_PROJECT_OVERVIEW.md"),
            Some("explain the product scope, operators, and outcomes.")
        );
        assert_eq!(
            contract.purpose_for_path("docs/02_ARCHITECTURE.md"),
            Some("describe the runtime architecture, boundaries, and critical flows.")
        );
    }

    #[test]
    fn explicit_artifact_plan_carries_required_file_purpose_into_step_description() {
        let statement = "Create exactly 2 markdown files:\n\
1. docs/01_PROJECT_OVERVIEW.md - explain the product scope, operators, and outcomes.\n\
2. docs/02_ARCHITECTURE.md: describe the runtime architecture, boundaries, and critical flows.\n\
All of these must be produced.";
        let goal = Goal::new(statement, "test-conversation");
        let plan = PlanEngine::generate_explicit_artifact_plan(&goal, None)
            .expect("explicit artifact plan generated");

        assert!(plan.steps.iter().any(|step| {
            step.description.contains("docs/01_PROJECT_OVERVIEW.md")
                && step
                    .description
                    .contains("Required purpose: explain the product scope, operators, and outcomes.")
        }));
    }

    #[test]
    fn literal_creation_plan_for_tiny_docs_note_file() {
        let plan = literal_plan("create a tiny docs note file");

        assert_direct_literal_plan(&plan, "docs/tiny-note.md", "docs note");
        assert!(plan.steps[0].description.contains("Do not analyze"));
        assert!(plan.steps[0].description.contains("src files"));
    }

    #[test]
    fn literal_creation_plan_for_todo_file() {
        let plan = literal_plan("create a todo file");

        assert_direct_literal_plan(&plan, "TODO.md", "todo file");
        assert!(plan.steps[0].description.contains("TODO list"));
    }

    #[test]
    fn literal_creation_plan_for_hello_world_script() {
        let plan = literal_plan("write a hello world script");

        assert_direct_literal_plan(&plan, "hello_world.py", "hello world script");
        assert!(
            plan.steps[0]
                .description
                .contains("prints exactly Hello, world!")
        );
    }

    #[test]
    fn literal_creation_classifier_rejects_implementation_heavy_requests() {
        for statement in [
            "build a Python CLI app",
            "create an auth system",
            "make a docs site",
            "create a REST API",
            "write a migration script for users table",
        ] {
            assert_full_planning_route(statement);
        }
    }

    #[test]
    fn literal_creation_classifier_rejects_mixed_complex_artifact_requests() {
        for statement in [
            "create a tiny docs note file for auth system architecture",
            "create a todo file for API integration work",
            "write a hello world script with CLI app wiring",
        ] {
            assert_full_planning_route(statement);
        }
    }

    // ============================================================================
    // REGRESSION TESTS: Objective extraction and completion rule handling
    // ============================================================================

    #[test]
    fn objective_extraction_prefers_task_section_over_completion_rule() {
        // Regression test: 15-doc prompt should extract task objective, not "DONE means..."
        let prompt = "Your task is to create/update exactly 15 canonical Markdown files inside docs/.\n\n\
1. docs/01_PROJECT_OVERVIEW.md\n\
2. docs/02_ARCHITECTURE.md\n\n\
DONE means all 15 docs exist and are non-empty.";

        let objective = summarize_goal_objective(prompt);

        // Should contain the task, not the completion rule
        assert!(
            objective.to_lowercase().contains("markdown") || objective.contains("15"),
            "objective should contain task description, got: {}",
            objective
        );
        assert!(
            !objective.to_lowercase().contains("done means"),
            "objective should NOT contain completion rule 'DONE means', got: {}",
            objective
        );
    }

    #[test]
    fn objective_extraction_excludes_validation_sections() {
        // Test that validation sections are not selected as objectives
        let prompt = "TASK: Create a configuration file\n\n\
VALIDATION:\n\
- File must be valid JSON\n\
- Must contain 'version' field\n\n\
Create the config.json file.";

        let objective = summarize_goal_objective(prompt);

        assert!(
            !objective.to_lowercase().contains("validation"),
            "objective should NOT be validation section, got: {}",
            objective
        );
        assert!(
            !objective.to_lowercase().contains("must contain"),
            "objective should NOT be validation constraint, got: {}",
            objective
        );
    }

    #[test]
    fn completion_rule_detection_works() {
        // Test is_completion_rule_line correctly identifies completion rules
        let completion_lines = [
            "DONE means all 15 docs exist",
            "Validation: all tests pass",
            "Success means the build completes",
            "Verify that files exist",
            "Ensure that all required files are present",
        ];

        for line in &completion_lines {
            assert!(
                is_completion_rule_line(line),
                "'{}' should be detected as completion rule",
                line
            );
        }

        let task_lines = [
            "Create 15 markdown files",
            "Implement the feature",
            "Build the application",
            "Update the configuration",
        ];

        for line in &task_lines {
            assert!(
                !is_completion_rule_line(line),
                "'{}' should NOT be detected as completion rule",
                line
            );
        }
    }

    #[test]
    fn fifteen_doc_prompt_objective_is_correct() {
        // Full 15-doc prompt regression test
        let prompt = "Create/update exactly 15 canonical Markdown docs inside docs/.\n\n\
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
15. docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md\n\n\
All of these must be produced.\n\n\
DONE means all 15 docs exist, are non-empty, and match the canonical structure.";

        let objective = summarize_goal_objective(prompt);

        // Expected: artifact contract based objective
        assert!(
            objective.contains("15") && objective.to_lowercase().contains("markdown"),
            "15-doc prompt should produce artifact-based objective, got: {}",
            objective
        );
        assert!(
            !objective.to_lowercase().contains("done means"),
            "objective should NOT be completion rule, got: {}",
            objective
        );
    }
}
