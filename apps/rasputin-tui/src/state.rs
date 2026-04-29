use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Editing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeStatus {
    Idle,
    Running,
    Completed,
    Error,
}

impl RuntimeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeStatus::Idle => "Idle",
            RuntimeStatus::Running => "Running",
            RuntimeStatus::Completed => "Completed",
            RuntimeStatus::Error => "Error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Chat,
    Edit,
    Task,
}

impl ExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionMode::Chat => "CHAT",
            ExecutionMode::Edit => "EDIT",
            ExecutionMode::Task => "TASK",
        }
    }

    /// Check if a StepAction is allowed in this mode
    /// Returns (allowed, reason) tuple
    pub fn check_action(&self, action: &StepAction) -> (bool, Option<String>) {
        match self {
            ExecutionMode::Chat => {
                // Chat mode: only Parse, Validate, Chat, Plan are allowed
                match action {
                    StepAction::Parse
                    | StepAction::Validate
                    | StepAction::Chat
                    | StepAction::Plan
                    | StepAction::Search { .. }
                    | StepAction::ReadFile { .. }
                    | StepAction::None => (true, None),
                    _ => (
                        false,
                        Some(format!(
                            "{} not allowed in CHAT mode. Switch to EDIT or TASK mode.",
                            action.as_str()
                        )),
                    ),
                }
            }
            ExecutionMode::Edit => {
                // Edit mode: allows file ops but not commands/servers/installs
                match action {
                    StepAction::RunCommand { .. }
                    | StepAction::StartServer { .. }
                    | StepAction::Install { .. }
                    | StepAction::Build
                    | StepAction::Test
                    | StepAction::ValidateProject => (
                        false,
                        Some(format!(
                            "{} not allowed in EDIT mode. Use /task or switch to TASK mode for commands.",
                            action.as_str()
                        )),
                    ),
                    _ => (true, None),
                }
            }
            ExecutionMode::Task => {
                // Task mode: everything allowed
                (true, None)
            }
        }
    }

    /// Get description of what this mode allows
    pub fn capability_summary(&self) -> &'static str {
        match self {
            ExecutionMode::Chat => "Chat only (no file changes)",
            ExecutionMode::Edit => "File edits allowed (no commands)",
            ExecutionMode::Task => "Full execution (files, commands, servers)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionState {
    Idle,
    Planning,
    Executing,
    Validating,
    Responding,
    /// V1.5 PROGRESS: Repairing state for auto-fix/recovery loops
    Repairing,
    /// V1.5 PROGRESS: Waiting for user approval on checkpoint
    WaitingForApproval,
    Done,
    Failed,
    Blocked,
    PreconditionFailed,
}

impl ExecutionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionState::Idle => "IDLE",
            ExecutionState::Planning => "PLANNING",
            ExecutionState::Executing => "EXECUTING",
            ExecutionState::Validating => "VALIDATING",
            ExecutionState::Responding => "RESPONDING",
            ExecutionState::Repairing => "REPAIRING",
            ExecutionState::WaitingForApproval => "WAITING_FOR_APPROVAL",
            ExecutionState::Done => "DONE",
            ExecutionState::Failed => "FAILED",
            ExecutionState::Blocked => "BLOCKED",
            ExecutionState::PreconditionFailed => "PRECONDITION_FAILED",
        }
    }

    /// V1.5 PROGRESS: Returns true if this is a terminal state (outcome should be used instead)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Blocked | Self::PreconditionFailed
        )
    }

    /// V1.5 PROGRESS: Returns true if this is an active progress state (non-terminal)
    pub fn is_active(&self) -> bool {
        !self.is_terminal() && !matches!(self, Self::Idle)
    }

    /// V1.5 PROGRESS: Returns true if stale terminal/recovery metadata should be cleared
    pub fn requires_clean_state(&self) -> bool {
        matches!(
            self,
            Self::Planning | Self::Executing | Self::Validating | Self::Repairing
        )
    }

    pub fn from_runtime_success(success: bool) -> Self {
        if success { Self::Done } else { Self::Failed }
    }
}

/// V1.5 STATE MACHINE: Canonical progress state transition event
/// Events that can drive ExecutionState transitions
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressTransitionEvent {
    /// Start a new task/execution run
    NewRun {
        task: String,
    },
    /// Planning phase activities
    PlanningIteration {
        iteration: usize,
    },
    PreflightComplete,
    ContextAssembled,
    PlannerOutput,
    /// Execution phase activities
    ToolCalling {
        name: String,
    },
    ToolExecuting {
        name: String,
    },
    ToolResult {
        success: bool,
    },
    BrowserPreview,
    MutationsDetected {
        count: usize,
    },
    StateCommitting,
    /// Validation phase activities
    ValidationRunning,
    ValidationResult {
        accepted: bool,
    },
    ValidationStage,
    /// Repair loop
    RepairLoop {
        attempt: usize,
        max: usize,
    },
    /// Completion/terminal events
    CompletionGate,
    RuntimeFinished {
        success: bool,
    },
    RuntimeFailure {
        reason: String,
    },
    /// Approval checkpoint
    ApprovalRequired {
        checkpoint_type: String,
    },
    ApprovalResolved {
        approved: bool,
    },
    /// External control
    OperatorStopped,
    ResetToIdle,
}

/// V1.5 STATE MACHINE: Result of a state transition attempt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionResult {
    /// Transition was allowed and applied
    Applied(ExecutionState),
    /// Transition was rejected - current state unchanged
    Rejected {
        current: ExecutionState,
        reason: &'static str,
    },
    /// Transition was normalized to a different allowed state
    Normalized {
        to: ExecutionState,
        reason: &'static str,
    },
}

/// V1.6 AUDIT: Types of audit events that can be recorded in the execution timeline
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    /// State transition was applied
    StateTransitionApplied,
    /// State transition was rejected
    StateTransitionRejected,
    /// State transition was normalized
    StateTransitionNormalized,
    /// Outcome was finalized
    OutcomeFinalized,
    /// Step started
    StepStarted,
    /// Step completed
    StepCompleted,
    /// Approval requested
    ApprovalRequested,
    /// Approval resolved (approved/denied)
    ApprovalResolved,
    /// Repair loop triggered
    RepairTriggered,
    /// Repair loop completed
    RepairCompleted,
    /// Validation started
    ValidationStarted,
    /// Validation completed
    ValidationCompleted,
    /// Runtime event received
    RuntimeEventReceived,
    /// Chain lifecycle event
    ChainLifecycle { event: String },
}

/// V1.6 AUDIT: Single audit event in the execution timeline
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Event timestamp (UTC)
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Type of audit event
    pub event_type: AuditEventType,
    /// Previous execution state (if applicable)
    pub previous_state: Option<ExecutionState>,
    /// New execution state (if applicable)
    pub next_state: Option<ExecutionState>,
    /// Triggering event (for transitions)
    pub triggering_event: Option<String>,
    /// Context: step ID
    pub step_id: Option<String>,
    /// Context: chain ID
    pub chain_id: Option<String>,
    /// Context: task description
    pub task: Option<String>,
    /// Optional reason for rejected/normalized transitions or outcome
    pub reason: Option<String>,
    /// Additional metadata (JSON-serialized)
    pub metadata: Option<String>,
}

impl AuditEvent {
    /// Create a new state transition audit event
    pub fn state_transition(
        event_type: AuditEventType,
        previous: ExecutionState,
        next: ExecutionState,
        triggering: &ProgressTransitionEvent,
        reason: Option<&'static str>,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            event_type,
            previous_state: Some(previous),
            next_state: Some(next),
            triggering_event: Some(format!("{:?}", triggering)),
            step_id: None,
            chain_id: None,
            task: None,
            reason: reason.map(|r| r.to_string()),
            metadata: None,
        }
    }

    /// Create a new outcome finalization audit event
    pub fn outcome_finalized(
        outcome: crate::persistence::ExecutionOutcome,
        reason: Option<String>,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            event_type: AuditEventType::OutcomeFinalized,
            previous_state: None,
            next_state: None,
            triggering_event: None,
            step_id: None,
            chain_id: None,
            task: None,
            reason: Some(format!("{:?}", outcome)),
            metadata: reason,
        }
    }

    /// Create a new lifecycle audit event
    pub fn lifecycle(event: &str, step_id: Option<String>, chain_id: Option<String>) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            event_type: AuditEventType::ChainLifecycle {
                event: event.to_string(),
            },
            previous_state: None,
            next_state: None,
            triggering_event: Some(event.to_string()),
            step_id,
            chain_id,
            task: None,
            reason: None,
            metadata: None,
        }
    }

    /// Create a new approval audit event
    pub fn approval(
        event_type: AuditEventType,
        checkpoint_type: &str,
        approved: Option<bool>,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            event_type,
            previous_state: None,
            next_state: None,
            triggering_event: Some(checkpoint_type.to_string()),
            step_id: None,
            chain_id: None,
            task: None,
            reason: approved.map(|a| {
                if a {
                    "approved".to_string()
                } else {
                    "denied".to_string()
                }
            }),
            metadata: None,
        }
    }
}

/// V1.6 AUDIT: Immutable append-only audit log for execution timeline
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditLog {
    events: Vec<AuditEvent>,
}

impl AuditLog {
    /// Create a new empty audit log
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Append an event (immutable - cannot modify past events)
    pub fn append(&mut self, event: AuditEvent) {
        self.events.push(event);
    }

    /// Get the last n events (most recent first)
    pub fn get_last_n(&self, n: usize) -> Vec<&AuditEvent> {
        self.events.iter().rev().take(n).collect()
    }

    /// Get all events in chronological order
    pub fn get_all_events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Get transition history (only state transition events)
    pub fn get_transition_history(&self) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    AuditEventType::StateTransitionApplied
                        | AuditEventType::StateTransitionRejected
                        | AuditEventType::StateTransitionNormalized
                )
            })
            .collect()
    }

    /// Get outcome trace (outcome finalization and key lifecycle events)
    pub fn get_outcome_trace(&self) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    AuditEventType::OutcomeFinalized
                        | AuditEventType::StepStarted
                        | AuditEventType::StepCompleted
                        | AuditEventType::RepairTriggered
                        | AuditEventType::RepairCompleted
                        | AuditEventType::ApprovalRequested
                        | AuditEventType::ApprovalResolved
                )
            })
            .collect()
    }

    /// Get events by type
    pub fn get_events_by_type(&self, event_type: AuditEventType) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.event_type == event_type)
            .collect()
    }

    /// Get the count of events
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get the most recent event
    pub fn last_event(&self) -> Option<&AuditEvent> {
        self.events.last()
    }

    /// Replay audit log to reconstruct final state (for testing/verification)
    /// Returns the final ExecutionState by replaying all applied transitions
    pub fn replay_transitions(&self, initial_state: ExecutionState) -> ExecutionState {
        let mut state = initial_state;

        for event in &self.events {
            match event.event_type {
                AuditEventType::StateTransitionApplied => {
                    if let Some(new_state) = event.next_state {
                        state = new_state;
                    }
                }
                AuditEventType::StateTransitionNormalized => {
                    if let Some(new_state) = event.next_state {
                        state = new_state;
                    }
                }
                _ => {} // Other events don't change state
            }
        }

        state
    }

    /// V1.6 REPLAY: Full deterministic replay from audit log
    /// Reconstructs execution progression, final state, and outcome purely from audit events
    pub fn replay(&self, initial_state: ExecutionState) -> ReplayResult {
        replay_audit_log(initial_state, self)
    }

    /// V1.6 CHECKPOINT: Replay only the audit prefix included by a checkpoint cursor.
    pub fn replay_to_cursor(&self, initial_state: ExecutionState, cursor: usize) -> ReplayResult {
        replay_audit_events(initial_state, &self.events[..cursor.min(self.events.len())])
    }
}

/// V1.6 REPLAY: Result of replaying an audit log
/// Contains complete reconstruction of execution from structured audit events only
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayResult {
    /// Initial state before replay
    pub initial_state: ExecutionState,
    /// Final reconstructed execution state
    pub final_state: ExecutionState,
    /// Final reconstructed outcome (if any)
    pub final_outcome: Option<crate::persistence::ExecutionOutcome>,
    /// All applied transitions (state changed)
    pub applied_transitions: Vec<ReplayedTransition>,
    /// All rejected transitions (state unchanged, with reason)
    pub rejected_transitions: Vec<ReplayedTransition>,
    /// All normalized transitions (state changed to normalized target)
    pub normalized_transitions: Vec<ReplayedTransition>,
    /// Outcome finalization event (if any)
    pub outcome_event: Option<ReplayedOutcome>,
    /// Replay warnings/inconsistencies detected
    pub warnings: Vec<ReplayWarning>,
    /// Whether replay represents a complete execution
    pub is_complete: bool,
    /// Total events processed
    pub events_processed: usize,
}

impl ReplayResult {
    /// Check if replay is deterministic (no inconsistencies detected)
    pub fn replay_is_deterministic(&self) -> bool {
        // Replay is deterministic if there are no inconsistency warnings
        !self.warnings.iter().any(|w| {
            matches!(
                w,
                ReplayWarning::InconsistentTransition { .. }
                    | ReplayWarning::ImpossibleSequence { .. }
            )
        })
    }

    /// Get total number of state transitions (applied + normalized)
    pub fn total_state_changes(&self) -> usize {
        self.applied_transitions.len() + self.normalized_transitions.len()
    }

    /// Check if replay reached a terminal state
    pub fn is_terminal(&self) -> bool {
        self.final_state.is_terminal()
    }
}

/// V1.6 REPLAY: A single replayed transition
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayedTransition {
    /// Event timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Transition type
    pub transition_type: ReplayTransitionType,
    /// State before transition
    pub from_state: ExecutionState,
    /// State after transition (same as from_state for rejected)
    pub to_state: ExecutionState,
    /// Triggering event description
    pub trigger: Option<String>,
    /// Reason for rejection/normalization (if applicable)
    pub reason: Option<String>,
    /// Step ID context
    pub step_id: Option<String>,
    /// Task context
    pub task: Option<String>,
    /// Event index in audit log
    pub event_index: usize,
}

/// V1.6 REPLAY: Type of transition during replay
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayTransitionType {
    Applied,
    Rejected,
    Normalized,
}

/// V1.6 REPLAY: Outcome finalization as replayed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayedOutcome {
    /// Event timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Outcome value
    pub outcome: crate::persistence::ExecutionOutcome,
    /// Reason/description
    pub reason: Option<String>,
    /// Event index in audit log
    pub event_index: usize,
}

/// V1.6 REPLAY: Warning during replay
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayWarning {
    /// Missing outcome finalization
    MissingOutcome,
    /// Inconsistent state transition detected
    InconsistentTransition {
        event_index: usize,
        expected: ExecutionState,
        actual: ExecutionState,
    },
    /// Multiple outcome finalizations (should only be one)
    MultipleOutcomes {
        first_index: usize,
        second_index: usize,
    },
    /// Impossible audit sequence
    ImpossibleSequence {
        event_index: usize,
        description: String,
    },
    /// Gap in audit log detected
    GapDetected { after_event_index: usize },
}

/// V1.6 REPLAY: Deterministic replay of audit log
/// Reconstructs execution progression purely from structured audit entries
pub fn replay_audit_log(initial_state: ExecutionState, audit_log: &AuditLog) -> ReplayResult {
    replay_audit_events(initial_state, audit_log.get_all_events())
}

/// V1.6 REPLAY: Deterministic replay of an audit event slice.
fn replay_audit_events(initial_state: ExecutionState, events: &[AuditEvent]) -> ReplayResult {
    let mut state = initial_state;
    let mut final_outcome: Option<crate::persistence::ExecutionOutcome> = None;
    let mut applied = Vec::new();
    let mut rejected = Vec::new();
    let mut normalized = Vec::new();
    let mut warnings = Vec::new();
    let mut outcome_event: Option<ReplayedOutcome> = None;
    let mut events_processed = 0;

    for (index, event) in events.iter().enumerate() {
        events_processed += 1;

        match event.event_type {
            AuditEventType::StateTransitionApplied => {
                let from_state = event.previous_state.unwrap_or(state);

                // Check for inconsistency: if previous_state is recorded but doesn't match current
                if let Some(prev) = event.previous_state {
                    if prev != state {
                        warnings.push(ReplayWarning::InconsistentTransition {
                            event_index: index,
                            expected: state,
                            actual: prev,
                        });
                    }
                }

                let to_state = event.next_state.unwrap_or(state);
                state = to_state;

                applied.push(ReplayedTransition {
                    timestamp: event.timestamp,
                    transition_type: ReplayTransitionType::Applied,
                    from_state,
                    to_state,
                    trigger: event.triggering_event.clone(),
                    reason: event.reason.clone(),
                    step_id: event.step_id.clone(),
                    task: event.task.clone(),
                    event_index: index,
                });
            }

            AuditEventType::StateTransitionRejected => {
                let from_state = event.previous_state.unwrap_or(state);

                // Rejected transitions don't change state
                rejected.push(ReplayedTransition {
                    timestamp: event.timestamp,
                    transition_type: ReplayTransitionType::Rejected,
                    from_state,
                    to_state: from_state, // State unchanged
                    trigger: event.triggering_event.clone(),
                    reason: event.reason.clone(),
                    step_id: event.step_id.clone(),
                    task: event.task.clone(),
                    event_index: index,
                });
            }

            AuditEventType::StateTransitionNormalized => {
                let from_state = event.previous_state.unwrap_or(state);
                let to_state = event.next_state.unwrap_or(state);
                state = to_state;

                normalized.push(ReplayedTransition {
                    timestamp: event.timestamp,
                    transition_type: ReplayTransitionType::Normalized,
                    from_state,
                    to_state,
                    trigger: event.triggering_event.clone(),
                    reason: event.reason.clone(),
                    step_id: event.step_id.clone(),
                    task: event.task.clone(),
                    event_index: index,
                });
            }

            AuditEventType::OutcomeFinalized => {
                // Check for multiple outcomes
                if outcome_event.is_some() {
                    warnings.push(ReplayWarning::MultipleOutcomes {
                        first_index: outcome_event.as_ref().unwrap().event_index,
                        second_index: index,
                    });
                }

                // Parse outcome from metadata or reason
                let outcome = parse_outcome_from_event(event);
                final_outcome = Some(outcome);

                outcome_event = Some(ReplayedOutcome {
                    timestamp: event.timestamp,
                    outcome,
                    reason: event.reason.clone(),
                    event_index: index,
                });
            }

            _ => {
                // Other event types don't affect replay state
                // but we could track them for completeness
            }
        }
    }

    // Check for missing outcome on terminal states
    if state.is_terminal() && outcome_event.is_none() {
        warnings.push(ReplayWarning::MissingOutcome);
    }

    // Determine if replay is complete
    // A replay is complete if:
    // - It has an outcome finalization (terminal truth recorded), OR
    // - It's still in progress (non-terminal state with events)
    // A replay is INCOMPLETE if:
    // - It reached a terminal state but has no outcome (missing finalization)
    // - It has no events (nothing happened)
    let is_complete = outcome_event.is_some() || (!state.is_terminal() && events_processed > 0);

    ReplayResult {
        initial_state,
        final_state: state,
        final_outcome,
        applied_transitions: applied,
        rejected_transitions: rejected,
        normalized_transitions: normalized,
        outcome_event,
        warnings,
        is_complete,
        events_processed,
    }
}

/// V1.6 REPLAY: Parse outcome from audit event metadata/reason
fn parse_outcome_from_event(event: &AuditEvent) -> crate::persistence::ExecutionOutcome {
    // First try to parse from metadata
    if let Some(ref metadata) = event.metadata {
        if metadata.contains("Success") {
            if metadata.contains("Warning") || metadata.contains("warning") {
                return crate::persistence::ExecutionOutcome::SuccessWithWarnings;
            }
            return crate::persistence::ExecutionOutcome::Success;
        }
        if metadata.contains("Failed") || metadata.contains("failed") {
            return crate::persistence::ExecutionOutcome::Failed;
        }
        if metadata.contains("Blocked") || metadata.contains("blocked") {
            return crate::persistence::ExecutionOutcome::Blocked;
        }
    }

    // Fall back to reason field
    if let Some(ref reason) = event.reason {
        let lower = reason.to_lowercase();
        if lower.contains("success") && lower.contains("warning") {
            return crate::persistence::ExecutionOutcome::SuccessWithWarnings;
        }
        if lower.contains("success") {
            return crate::persistence::ExecutionOutcome::Success;
        }
        if lower.contains("fail") || lower.contains("error") {
            return crate::persistence::ExecutionOutcome::Failed;
        }
        if lower.contains("block") {
            return crate::persistence::ExecutionOutcome::Blocked;
        }
    }

    // Default to success if we can't determine
    crate::persistence::ExecutionOutcome::Success
}

/// V1.6 REPLAY: Summarize replay result for human consumption
pub fn summarize_replay_result(result: &ReplayResult) -> String {
    let mut lines = Vec::new();

    lines.push("Replay Summary".to_string());
    lines.push("==============".to_string());
    lines.push(String::new());
    lines.push(format!("Initial state: {:?}", result.initial_state));
    lines.push(format!("Final state: {:?}", result.final_state));

    if let Some(outcome) = result.final_outcome {
        lines.push(format!("Outcome: {:?}", outcome));
    } else {
        lines.push("Outcome: (not finalized)".to_string());
    }

    lines.push(String::new());
    lines.push("Transitions:".to_string());
    lines.push(format!("  Applied: {}", result.applied_transitions.len()));
    lines.push(format!("  Rejected: {}", result.rejected_transitions.len()));
    lines.push(format!(
        "  Normalized: {}",
        result.normalized_transitions.len()
    ));
    lines.push(format!("  Events processed: {}", result.events_processed));

    if !result.warnings.is_empty() {
        lines.push(String::new());
        lines.push(format!("Warnings ({}):", result.warnings.len()));
        for warning in &result.warnings {
            lines.push(format!("  • {}", format_replay_warning(warning)));
        }
    }

    if result.is_complete {
        lines.push(String::new());
        lines.push("✓ Replay complete".to_string());
    } else {
        lines.push(String::new());
        lines.push("⚠ Replay incomplete".to_string());
    }

    lines.join("\n")
}

/// V1.6 REPLAY: Format a replay warning as human-readable string
fn format_replay_warning(warning: &ReplayWarning) -> String {
    match warning {
        ReplayWarning::MissingOutcome => "Missing outcome finalization".to_string(),
        ReplayWarning::InconsistentTransition {
            event_index,
            expected,
            actual,
        } => {
            format!(
                "Inconsistent transition at event {}: expected {:?}, found {:?}",
                event_index, expected, actual
            )
        }
        ReplayWarning::MultipleOutcomes {
            first_index,
            second_index,
        } => {
            format!(
                "Multiple outcomes detected at events {} and {}",
                first_index, second_index
            )
        }
        ReplayWarning::ImpossibleSequence {
            event_index,
            description,
        } => {
            format!(
                "Impossible sequence at event {}: {}",
                event_index, description
            )
        }
        ReplayWarning::GapDetected { after_event_index } => {
            format!("Gap detected after event {}", after_event_index)
        }
    }
}

/// V1.6 REPLAY: Validate replay against stored terminal truth
/// Returns true if replayed result matches stored result
pub fn validate_replay_against_stored(
    replay: &ReplayResult,
    stored_state: ExecutionState,
    stored_outcome: Option<crate::persistence::ExecutionOutcome>,
) -> Result<(), ReplayValidationError> {
    // Check state match
    if replay.final_state != stored_state {
        return Err(ReplayValidationError::StateMismatch {
            replayed: replay.final_state,
            stored: stored_state,
        });
    }

    // Check outcome match
    // Only fail validation if outcomes actually contradict
    // Missing replay outcome is a warning, not a validation failure
    match (replay.final_outcome, stored_outcome) {
        (Some(replayed), Some(stored)) if replayed != stored => {
            // Outcomes actually contradict - this is a divergence
            return Err(ReplayValidationError::OutcomeMismatch { replayed, stored });
        }
        _ => {} // All other cases pass: matching, both None, or replay missing
    }

    Ok(())
}

/// V1.6 REPLAY: Error when replay validation fails
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayValidationError {
    StateMismatch {
        replayed: ExecutionState,
        stored: ExecutionState,
    },
    OutcomeMismatch {
        replayed: crate::persistence::ExecutionOutcome,
        stored: crate::persistence::ExecutionOutcome,
    },
    MissingReplayOutcome,
}

impl std::fmt::Display for ReplayValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayValidationError::StateMismatch { replayed, stored } => {
                write!(
                    f,
                    "State mismatch: replayed {:?} vs stored {:?}",
                    replayed, stored
                )
            }
            ReplayValidationError::OutcomeMismatch { replayed, stored } => {
                write!(
                    f,
                    "Outcome mismatch: replayed {:?} vs stored {:?}",
                    replayed, stored
                )
            }
            ReplayValidationError::MissingReplayOutcome => {
                write!(f, "Replay missing outcome that stored state has")
            }
        }
    }
}

impl std::error::Error for ReplayValidationError {}

/// V1.5 STATE MACHINE: Canonical ExecutionState transition reducer
/// All execution state changes must flow through this function
pub fn reduce_execution_state(
    current: ExecutionState,
    event: ProgressTransitionEvent,
    has_terminal_outcome: bool,
) -> TransitionResult {
    // Terminal outcome is sticky - cannot transition away from terminal states
    // unless explicitly starting a new run
    if current.is_terminal() && !matches!(event, ProgressTransitionEvent::NewRun { .. }) {
        return TransitionResult::Rejected {
            current,
            reason: "terminal state is sticky - use NewRun to reset",
        };
    }

    // Terminal outcome prevents any active state transitions
    if has_terminal_outcome
        && !matches!(
            event,
            ProgressTransitionEvent::NewRun { .. }
                | ProgressTransitionEvent::RuntimeFinished { .. }
        )
    {
        return TransitionResult::Rejected {
            current,
            reason: "terminal outcome set - cannot transition to active states",
        };
    }

    match (&current, &event) {
        // Idle can only transition to Planning (new run start)
        (ExecutionState::Idle, ProgressTransitionEvent::NewRun { .. }) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::Idle, _) => TransitionResult::Rejected {
            current,
            reason: "Idle state requires NewRun event to transition",
        },

        // Planning can transition to more planning, executing, or terminal
        (ExecutionState::Planning, ProgressTransitionEvent::PlanningIteration { .. }) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::PreflightComplete) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::ContextAssembled) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::PlannerOutput) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::ToolCalling { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::ToolExecuting { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::ValidationRunning) => {
            TransitionResult::Applied(ExecutionState::Validating)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::RuntimeFailure { .. }) => {
            TransitionResult::Applied(ExecutionState::Failed)
        }
        (ExecutionState::Planning, ProgressTransitionEvent::OperatorStopped) => {
            TransitionResult::Applied(ExecutionState::Blocked)
        }

        // Executing can transition to validation, repair, approval, or terminal
        (ExecutionState::Executing, ProgressTransitionEvent::ToolCalling { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::ToolExecuting { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::ToolResult { success }) => {
            if *success {
                TransitionResult::Applied(ExecutionState::Executing)
            } else {
                TransitionResult::Applied(ExecutionState::Executing)
            }
        }
        (ExecutionState::Executing, ProgressTransitionEvent::BrowserPreview) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::MutationsDetected { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::StateCommitting) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::ValidationRunning) => {
            TransitionResult::Applied(ExecutionState::Validating)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::RepairLoop { .. }) => {
            TransitionResult::Applied(ExecutionState::Repairing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::ApprovalRequired { .. }) => {
            TransitionResult::Applied(ExecutionState::WaitingForApproval)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::RuntimeFailure { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::RuntimeFinished { success }) => {
            TransitionResult::Applied(ExecutionState::from_runtime_success(*success))
        }
        (ExecutionState::Executing, ProgressTransitionEvent::CompletionGate) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Executing, ProgressTransitionEvent::OperatorStopped) => {
            TransitionResult::Applied(ExecutionState::Blocked)
        }

        // Validating can stay validating, go to executing (more work), or terminal
        (ExecutionState::Validating, ProgressTransitionEvent::ValidationRunning) => {
            TransitionResult::Applied(ExecutionState::Validating)
        }
        (ExecutionState::Validating, ProgressTransitionEvent::ValidationStage) => {
            TransitionResult::Applied(ExecutionState::Validating)
        }
        (ExecutionState::Validating, ProgressTransitionEvent::ValidationResult { accepted }) => {
            if *accepted {
                TransitionResult::Applied(ExecutionState::Validating)
            } else {
                TransitionResult::Applied(ExecutionState::Validating)
            }
        }
        (ExecutionState::Validating, ProgressTransitionEvent::ToolCalling { .. }) => {
            // Validation found issues, going back to execution for fixes
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Validating, ProgressTransitionEvent::RepairLoop { .. }) => {
            TransitionResult::Applied(ExecutionState::Repairing)
        }
        (ExecutionState::Validating, ProgressTransitionEvent::CompletionGate) => {
            TransitionResult::Applied(ExecutionState::Validating)
        }
        (ExecutionState::Validating, ProgressTransitionEvent::RuntimeFinished { success }) => {
            TransitionResult::Applied(ExecutionState::from_runtime_success(*success))
        }
        (ExecutionState::Validating, ProgressTransitionEvent::RuntimeFailure { .. }) => {
            TransitionResult::Applied(ExecutionState::Validating)
        }

        // Repairing is a specialized executing state
        (ExecutionState::Repairing, ProgressTransitionEvent::RepairLoop { .. }) => {
            TransitionResult::Applied(ExecutionState::Repairing)
        }
        (ExecutionState::Repairing, ProgressTransitionEvent::ToolCalling { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Repairing, ProgressTransitionEvent::ToolExecuting { .. }) => {
            TransitionResult::Applied(ExecutionState::Executing)
        }
        (ExecutionState::Repairing, ProgressTransitionEvent::ValidationRunning) => {
            TransitionResult::Applied(ExecutionState::Validating)
        }
        (ExecutionState::Repairing, ProgressTransitionEvent::RuntimeFailure { .. }) => {
            TransitionResult::Applied(ExecutionState::Repairing)
        }

        // WaitingForApproval is sticky - cannot be overwritten by running events
        (
            ExecutionState::WaitingForApproval,
            ProgressTransitionEvent::ApprovalResolved { approved },
        ) => {
            if *approved {
                TransitionResult::Applied(ExecutionState::Executing)
            } else {
                TransitionResult::Applied(ExecutionState::Blocked)
            }
        }
        (ExecutionState::WaitingForApproval, ProgressTransitionEvent::ToolCalling { .. })
        | (ExecutionState::WaitingForApproval, ProgressTransitionEvent::ToolExecuting { .. })
        | (ExecutionState::WaitingForApproval, ProgressTransitionEvent::ValidationRunning)
        | (ExecutionState::WaitingForApproval, ProgressTransitionEvent::PlannerOutput)
        | (ExecutionState::WaitingForApproval, ProgressTransitionEvent::PlanningIteration { .. }) => {
            TransitionResult::Rejected {
                current,
                reason: "WaitingForApproval cannot be overwritten by runtime events - must resolve approval first",
            }
        }
        (ExecutionState::WaitingForApproval, ProgressTransitionEvent::OperatorStopped) => {
            TransitionResult::Applied(ExecutionState::Blocked)
        }

        // Responding is similar to executing
        (ExecutionState::Responding, ProgressTransitionEvent::RuntimeFinished { success }) => {
            TransitionResult::Applied(ExecutionState::from_runtime_success(*success))
        }
        (ExecutionState::Responding, _) => {
            // Responding can transition to most states
            TransitionResult::Normalized {
                to: ExecutionState::Executing,
                reason: "Responding transitions normalized to Executing",
            }
        }

        // Terminal states - already handled by sticky check at start
        (ExecutionState::Done, ProgressTransitionEvent::NewRun { .. }) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::Failed, ProgressTransitionEvent::NewRun { .. }) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::Blocked, ProgressTransitionEvent::NewRun { .. }) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }
        (ExecutionState::PreconditionFailed, ProgressTransitionEvent::NewRun { .. }) => {
            TransitionResult::Applied(ExecutionState::Planning)
        }

        // Reset events
        (_, ProgressTransitionEvent::ResetToIdle) => {
            TransitionResult::Applied(ExecutionState::Idle)
        }

        // Catch-all: reject unexpected transitions
        _ => TransitionResult::Rejected {
            current,
            reason: "transition not defined in state machine",
        },
    }
}

/// V1.6 AUDIT: Instrumented reducer that produces audit events
/// Returns the transition result and an optional audit event
pub fn reduce_execution_state_with_audit(
    current: ExecutionState,
    event: ProgressTransitionEvent,
    has_terminal_outcome: bool,
) -> (TransitionResult, Option<AuditEvent>) {
    let result = reduce_execution_state(current, event.clone(), has_terminal_outcome);

    let audit_event = match &result {
        TransitionResult::Applied(new_state) => Some(AuditEvent::state_transition(
            AuditEventType::StateTransitionApplied,
            current,
            *new_state,
            &event,
            None,
        )),
        TransitionResult::Rejected { current: _, reason } => {
            Some(AuditEvent::state_transition(
                AuditEventType::StateTransitionRejected,
                current,
                current, // State doesn't change on rejection
                &event,
                Some(reason),
            ))
        }
        TransitionResult::Normalized { to, reason } => Some(AuditEvent::state_transition(
            AuditEventType::StateTransitionNormalized,
            current,
            *to,
            &event,
            Some(reason),
        )),
    };

    (result, audit_event)
}

/// Specific host action a step performs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepAction {
    /// No specific action (informational step)
    None,
    /// Parse/understand input
    Parse,
    /// Validate context/state
    Validate,
    /// Create directory on disk
    CreateDirectory { path: String },
    /// Write file to disk
    WriteFile { path: String },
    /// Read file from disk
    ReadFile { path: String },
    /// Apply patch to file
    PatchFile { path: String },
    /// Execute shell command
    RunCommand { command: String },
    /// Generate plan/intent
    Plan,
    /// AI chat/response generation
    Chat,
    /// Search across files
    Search { query: String },
    /// Validation pipeline
    ValidateProject,
    /// Install dependencies
    Install { package_manager: String },
    /// Start dev server
    StartServer { command: String },
    /// Build project
    Build,
    /// Test project
    Test,
    /// Git operations
    Git { operation: String },
    /// Fix/recovery step for self-healing
    Fix {
        issue_description: String,
        affected_files: Vec<String>,
        error_context: Option<String>,
    },
}

impl StepAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepAction::None => "none",
            StepAction::Parse => "parse",
            StepAction::Validate => "validate",
            StepAction::CreateDirectory { .. } => "mkdir",
            StepAction::WriteFile { .. } => "write",
            StepAction::ReadFile { .. } => "read",
            StepAction::PatchFile { .. } => "patch",
            StepAction::RunCommand { .. } => "exec",
            StepAction::Plan => "plan",
            StepAction::Chat => "chat",
            StepAction::Search { .. } => "search",
            StepAction::ValidateProject => "validate",
            StepAction::Install { .. } => "install",
            StepAction::StartServer { .. } => "serve",
            StepAction::Build => "build",
            StepAction::Test => "test",
            StepAction::Git { .. } => "git",
            StepAction::Fix { .. } => "fix",
        }
    }
}

/// Classification of step execution outcome for self-healing decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepOutcomeClass {
    /// Step succeeded completely
    Success,
    /// Step failed but may be recoverable
    Failure,
    /// Step partially succeeded (e.g., some tests passed)
    Partial,
    /// Step blocked waiting for external action
    Blocked,
    /// Recovery step that fixes a previous failure
    Recovery,
}

impl Default for StepOutcomeClass {
    fn default() -> Self {
        StepOutcomeClass::Success
    }
}

/// Structured failure reason for recovery planning
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureReason {
    /// Compilation/build error
    CompileError {
        language: String,
        error_summary: String,
        line: Option<u32>,
    },
    /// Test failure
    TestFailure {
        test_name: Option<String>,
        failure_message: String,
    },
    /// Syntax error
    SyntaxError {
        file_path: String,
        error_message: String,
    },
    /// Command not found or not executable
    CommandNotFound { command: String },
    /// Permission denied
    PermissionDenied { path: String },
    /// Timeout
    Timeout { duration_ms: u64 },
    /// Validation gate rejection
    ValidationRejection { stage: String, message: String },
    /// Unknown/unclassified failure
    Unknown { message: String },
}

/// Execution evidence captured for audit and replay
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionEvidence {
    /// Standard output from command/tool
    pub stdout: Option<String>,
    /// Standard error from command/tool
    pub stderr: Option<String>,
    /// Command exit code if applicable
    pub exit_code: Option<i32>,
    /// Validation stage that failed (if any)
    pub failed_validation_stage: Option<String>,
    /// Detailed validation failure message
    pub validation_failure_details: Option<String>,
    /// Build/test error summary
    pub error_summary: Option<String>,
    /// Suggested fix from validation engine
    pub suggested_fix: Option<String>,
}

/// Result metadata for a completed step - proof of completion
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepResult {
    /// Files/directories affected by this step
    pub affected_paths: Vec<String>,
    /// Exit code for command executions
    pub exit_code: Option<i32>,
    /// Bytes written or count of items affected
    pub bytes_affected: Option<usize>,
    /// Validation outcome (passed/failed/skipped/unavailable)
    pub validation_result: Option<String>,
    /// Error message if step failed
    pub error_message: Option<String>,
    /// Artifact references (server URLs, generated files, etc.)
    pub artifact_urls: Vec<String>,
    /// Duration of step execution in milliseconds
    pub duration_ms: Option<u64>,
    /// Classification of step outcome
    pub outcome_class: StepOutcomeClass,
    /// Structured failure reason (if failed)
    pub failure_reason: Option<FailureReason>,
    /// Comprehensive execution evidence
    pub evidence: ExecutionEvidence,
    /// Retry attempt number (0 = original, 1+ = retries)
    pub retry_attempt: u32,
    /// ID of step this is recovering (if recovery step)
    pub recovery_for_step_id: Option<String>,
}

/// Completion confidence after recovery - distinguishes step success from objective completion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompletionConfidence {
    /// Step recovered successfully, but objective requires more work
    PartialRecovery,
    /// Recovery succeeded and objective appears complete
    ObjectiveSatisfied,
    /// Recovery succeeded but cannot determine completion - requires explicit decision
    Uncertain,
    /// No recovery needed - normal step success
    #[default]
    NotApplicable,
}

/// Required surface that must exist for objective completion
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequiredSurface {
    /// A file must exist at path
    FileExists { path: String },
    /// A test must pass
    TestPasses { name: String },
    /// Build must succeed
    BuildSucceeds,
    /// Validation must pass
    ValidationPasses,
    /// Custom condition with description
    Custom { description: String, check: String },
}

/// Explicit artifact-set contract for completion truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArtifactCompletionContract {
    /// Human-readable artifact type (e.g. "markdown").
    pub artifact_type: Option<String>,
    /// Exact required deliverable count when the task specified one.
    pub required_count: Option<usize>,
    /// Exact filenames or relative paths required by the task.
    pub required_filenames: Vec<String>,
    /// Structured required artifact metadata, including per-file purpose when present.
    #[serde(default)]
    pub required_artifacts: Vec<ArtifactRequirement>,
    /// Required filenames currently present and non-empty.
    #[serde(default)]
    pub created_filenames: Vec<String>,
    /// Required filenames not yet present.
    #[serde(default)]
    pub missing_filenames: Vec<String>,
    /// Required filenames that exist but are empty/blank.
    #[serde(default)]
    pub empty_filenames: Vec<String>,
    /// Produced filenames that do not belong to the explicit contract.
    #[serde(default)]
    pub unexpected_filenames: Vec<String>,
    /// Count of produced artifacts attributed to this contract.
    #[serde(default)]
    pub actual_output_count: Option<usize>,
    /// Whether every required file must be non-empty.
    #[serde(default = "default_require_non_empty_artifacts")]
    pub require_non_empty: bool,
}

/// Structured metadata for one required artifact in an explicit completion contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArtifactRequirement {
    /// Exact required filename or relative path.
    pub path: String,
    /// Purpose or content intent extracted from the prompt when present.
    #[serde(default)]
    pub purpose: Option<String>,
}

/// Artifact-oriented CRUD operation used when decomposing explicit deliverable work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactCrudOperation {
    CreateMissing,
    UpdateExisting,
    ReplaceEmpty,
    ListRequired,
    CheckCompleteness,
}

impl ArtifactCrudOperation {
    pub fn verb(self) -> &'static str {
        match self {
            Self::CreateMissing => "Create missing",
            Self::UpdateExisting => "Update existing",
            Self::ReplaceEmpty => "Replace empty",
            Self::ListRequired => "List required",
            Self::CheckCompleteness => "Check completeness",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::CreateMissing => "write the required artifact when it does not exist yet",
            Self::UpdateExisting => "edit an existing required artifact without changing the filename contract",
            Self::ReplaceEmpty => "replace an empty required artifact with substantive content",
            Self::ListRequired => "inventory the required artifact set and current filesystem state",
            Self::CheckCompleteness => "validate that the full artifact contract is satisfied",
        }
    }
}

fn default_require_non_empty_artifacts() -> bool {
    true
}

impl ArtifactCompletionContract {
    pub fn required_deliverable_count(&self) -> usize {
        self.required_count
            .unwrap_or_else(|| self.required_filenames.len())
    }

    pub fn purpose_for_path(&self, path: &str) -> Option<&str> {
        self.required_artifacts
            .iter()
            .find(|artifact| artifact.path == path)
            .and_then(|artifact| artifact.purpose.as_deref())
    }

    pub fn is_satisfied(&self) -> bool {
        self.missing_filenames.is_empty()
            && self.empty_filenames.is_empty()
            && self.unexpected_filenames.is_empty()
            && self
                .actual_output_count
                .map(|count| count == self.required_deliverable_count())
                .unwrap_or_else(|| self.created_filenames.len() == self.required_deliverable_count())
    }

    pub fn has_requirements(&self) -> bool {
        !self.required_filenames.is_empty() || self.required_count.is_some()
    }
}

/// Objective satisfaction check - what must be true for chain to be "done"
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ObjectiveSatisfaction {
    /// Required surfaces that must exist
    pub required_surfaces: Vec<RequiredSurface>,
    /// Structured explicit artifact contract derived from the objective.
    #[serde(default)]
    pub artifact_contract: Option<ArtifactCompletionContract>,
    /// Minimum validation stage that must pass
    pub min_validation_stage: Option<String>,
    /// Objective-specific completion check
    pub objective_complete: bool,
    /// Last time completion was checked
    pub checked_at: Option<DateTime<Local>>,
    /// Confidence level in completion assessment
    pub confidence: CompletionConfidence,
    /// Reason for completion/non-completion decision
    pub reason: Option<String>,
}

/// Recovery path entry - one step in the self-healing chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPathEntry {
    /// Step ID that failed
    pub failed_step_id: String,
    /// Failure evidence summary
    pub failure_evidence: String,
    /// Classification of failure
    pub failure_class: StepOutcomeClass,
    /// Whether failure was deemed recoverable
    pub was_recoverable: bool,
    /// Recovery step generated
    pub recovery_step_id: Option<String>,
    /// Description of recovery action
    pub recovery_description: Option<String>,
    /// Retry attempt number
    pub retry_attempt: u32,
    /// Policy applied at this point
    pub retry_policy: String,
    /// Result of recovery (success/failure)
    pub recovery_result: Option<String>,
    /// Completion confidence after recovery
    pub completion_confidence: Option<CompletionConfidence>,
    /// Decision made (Continue, Finalize, Halt)
    pub decision: Option<String>,
    /// Timestamp
    pub timestamp: DateTime<Local>,
}

/// Recovery state - tracks self-healing and completion-confidence decisions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryState {
    /// Full recovery path showing all recovery attempts
    pub recovery_path: Vec<RecoveryPathEntry>,
    /// Current retry count for active step
    pub current_retry_attempt: u32,
    /// Maximum retries allowed by policy
    pub max_retries: u32,
    /// Current recovery depth
    pub current_recovery_depth: u32,
    /// Maximum recovery depth allowed
    pub max_recovery_depth: u32,
    /// Last failure evidence for display
    pub last_failure_evidence: Option<ExecutionEvidence>,
    /// Last completion-confidence decision
    pub last_completion_decision: Option<String>,
    /// Last completion reason
    pub last_completion_reason: Option<String>,
    /// Whether recovery is currently active
    pub recovery_in_progress: bool,
    /// Summary for operator display
    pub operator_summary: String,
}

impl RecoveryState {
    /// Create new recovery state from policy
    pub fn from_policy(policy: &crate::persistence::ChainPolicy) -> Self {
        Self {
            max_retries: policy.max_auto_retries_per_step,
            max_recovery_depth: policy.max_chain_recovery_depth,
            ..Default::default()
        }
    }

    /// Add a recovery path entry
    pub fn add_recovery_entry(&mut self, entry: RecoveryPathEntry) {
        self.current_retry_attempt = entry.retry_attempt;
        if entry.recovery_result.is_some() {
            self.recovery_in_progress = false;
        } else {
            self.recovery_in_progress = true;
        }
        self.operator_summary = format!(
            "Recovery #{} for {}: {}",
            entry.retry_attempt,
            entry.failed_step_id,
            entry.decision.as_deref().unwrap_or("in progress")
        );
        self.recovery_path.push(entry);
    }

    /// Record completion confidence decision
    pub fn record_completion_decision(&mut self, decision: &str, reason: &str) {
        self.last_completion_decision = Some(decision.to_string());
        self.last_completion_reason = Some(reason.to_string());
        if let Some(last) = self.recovery_path.last_mut() {
            last.decision = Some(decision.to_string());
        }
    }

    /// Get formatted summary for TUI display
    pub fn format_summary(&self) -> String {
        if self.recovery_path.is_empty() {
            if self.current_retry_attempt > 0 {
                return format!(
                    "Retry {}/{} | Depth {}/{} | recovery pending | active",
                    self.current_retry_attempt,
                    self.max_retries,
                    self.current_recovery_depth,
                    self.max_recovery_depth
                );
            }
            return "No recovery activity".to_string();
        }

        let last = self.recovery_path.last().unwrap();
        format!(
            "Retry {}/{} | Depth {}/{} | {} | {}",
            self.current_retry_attempt,
            self.max_retries,
            self.current_recovery_depth,
            self.max_recovery_depth,
            last.decision.as_deref().unwrap_or("recovering"),
            self.last_completion_reason.as_deref().unwrap_or("active")
        )
    }

    /// Check if retries exhausted
    pub fn retries_exhausted(&self) -> bool {
        self.current_retry_attempt >= self.max_retries
    }

    /// Check if recovery depth exceeded
    pub fn depth_exceeded(&self) -> bool {
        self.current_recovery_depth >= self.max_recovery_depth
    }

    /// Get Normal-mode recovery summary - calm, user-facing language
    ///
    /// Examples:
    /// - "I hit an error and am trying a fix."
    /// - "The fix worked. Finishing the remaining steps."
    /// - "I fixed the error, but there's still more to do."
    /// - "I'm not confident the task is complete, so I stopped."
    pub fn format_summary_normal(&self) -> String {
        if self.recovery_path.is_empty() {
            return String::new(); // No recovery activity, no message needed
        }

        let last = self.recovery_path.last().unwrap();
        let decision = last.decision.as_deref();
        let has_recovery_result = last.recovery_result.is_some();
        let is_recovery_in_progress = self.recovery_in_progress;

        // Recovery is currently in progress
        if is_recovery_in_progress || decision.is_none() {
            return "I hit an error and am trying a fix.".to_string();
        }

        // Recovery completed - check the decision
        match decision {
            Some("Finalize") => "The fix worked. Task is complete.".to_string(),
            Some("Continue") => {
                if has_recovery_result {
                    "I fixed the error, but there's still more to do.".to_string()
                } else {
                    "I'm continuing with the remaining steps.".to_string()
                }
            }
            Some("HaltForClarification") => {
                "I'm not confident the task is complete, so I stopped.".to_string()
            }
            Some(other) => {
                // Unknown decision state - use a safe default
                format!("Working on it... ({})", other)
            }
            None => "Processing...".to_string(),
        }
    }
}

/// Failure context for error reporting
#[derive(Debug, Clone, Default)]
pub struct FailureContext {
    pub affected_paths: Vec<String>,
}

impl StepResult {
    pub fn with_error(error: impl Into<String>) -> Self {
        Self {
            error_message: Some(error.into()),
            outcome_class: StepOutcomeClass::Failure,
            ..Default::default()
        }
    }

    pub fn success_summary(&self) -> String {
        if let Some(code) = self.exit_code {
            format!("exit {}", code)
        } else if !self.affected_paths.is_empty() {
            format!("{} affected", self.affected_paths.len())
        } else if let Some(bytes) = self.bytes_affected {
            if bytes < 1024 {
                format!("{} bytes", bytes)
            } else {
                format!("{:.1} KB", bytes as f64 / 1024.0)
            }
        } else {
            "completed".to_string()
        }
    }

    /// Determine if this failure is recoverable based on failure reason
    pub fn is_recoverable(&self) -> bool {
        match &self.failure_reason {
            None => false, // No failure reason = not a failure
            Some(reason) => match reason {
                FailureReason::CompileError { .. } => true,
                FailureReason::TestFailure { .. } => true,
                FailureReason::SyntaxError { .. } => true,
                FailureReason::ValidationRejection { .. } => true,
                FailureReason::CommandNotFound { .. } => false, // Can't recover if command doesn't exist
                FailureReason::PermissionDenied { .. } => false, // Can't recover without permission change
                FailureReason::Timeout { .. } => true,           // Retry might succeed
                FailureReason::Unknown { .. } => false,          // Don't retry unknown failures
            },
        }
    }

    /// Generate a recovery step description based on failure reason
    pub fn generate_recovery_description(&self) -> Option<String> {
        match &self.failure_reason {
            Some(FailureReason::CompileError {
                language,
                error_summary,
                ..
            }) => Some(format!(
                "Fix {} compilation error: {}",
                language, error_summary
            )),
            Some(FailureReason::TestFailure { test_name, .. }) => Some(format!(
                "Fix failing test: {}",
                test_name.as_deref().unwrap_or("unknown test")
            )),
            Some(FailureReason::SyntaxError { file_path, .. }) => {
                Some(format!("Fix syntax error in {}", file_path))
            }
            Some(FailureReason::ValidationRejection { stage, message }) => {
                Some(format!("Fix {} validation issue: {}", stage, message))
            }
            Some(FailureReason::Timeout { .. }) => Some("Retry timed-out operation".to_string()),
            _ => None,
        }
    }

    /// Extract suggested fix from evidence if available
    pub fn suggested_fix(&self) -> Option<&str> {
        self.evidence.suggested_fix.as_deref()
    }
}

/// A single step in an execution plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStep {
    pub id: String,
    pub description: String,
    /// The specific host action this step performs
    pub action: StepAction,
    pub status: ExecutionStepStatus,
    /// User-facing output/result description
    pub output: Option<String>,
    /// Structured result metadata for proof of completion
    pub result: Option<StepResult>,
    pub started_at: Option<DateTime<Local>>,
    pub completed_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// A unified execution plan - all user input becomes a plan with steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub id: String,
    pub intent: String,
    pub objective: String,
    pub steps: Vec<ExecutionStep>,
    pub current_step_index: usize,
    pub created_at: DateTime<Local>,
    pub started_at: Option<DateTime<Local>>,
    pub completed_at: Option<DateTime<Local>>,
    pub final_result: Option<String>,
}

impl ExecutionPlan {
    pub fn new(intent: impl Into<String>, objective: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            intent: intent.into(),
            objective: objective.into(),
            steps: vec![],
            current_step_index: 0,
            created_at: Local::now(),
            started_at: None,
            completed_at: None,
            final_result: None,
        }
    }

    pub fn add_step(
        &mut self,
        description: impl Into<String>,
        action: StepAction,
    ) -> &mut ExecutionStep {
        let step = ExecutionStep {
            id: uuid::Uuid::new_v4().to_string(),
            description: description.into(),
            action,
            status: ExecutionStepStatus::Pending,
            output: None,
            result: None,
            started_at: None,
            completed_at: None,
        };
        self.steps.push(step);
        self.steps.last_mut().unwrap()
    }

    pub fn start(&mut self) {
        self.started_at = Some(Local::now());
    }

    pub fn start_step(&mut self, index: usize) -> Option<&mut ExecutionStep> {
        if let Some(step) = self.steps.get_mut(index) {
            step.status = ExecutionStepStatus::Running;
            step.started_at = Some(Local::now());
            self.current_step_index = index;
        }
        self.steps.get_mut(index)
    }

    /// Complete a step with full result metadata
    pub fn complete_step_with_result(
        &mut self,
        index: usize,
        output: Option<String>,
        result: StepResult,
    ) {
        if let Some(step) = self.steps.get_mut(index) {
            step.status = ExecutionStepStatus::Completed;
            step.output = output;
            step.result = Some(result);
            step.completed_at = Some(Local::now());
        }
    }

    /// Fail a step with structured error info
    pub fn fail_step(&mut self, index: usize, error: impl Into<String>) {
        if let Some(step) = self.steps.get_mut(index) {
            let error_msg = error.into();
            step.status = ExecutionStepStatus::Failed;
            step.output = Some(error_msg.clone());
            step.result = Some(StepResult::with_error(&error_msg));
            step.completed_at = Some(Local::now());
        }
    }

    /// Get failed step info for error reporting
    pub fn get_failure_info(&self) -> Option<(usize, &ExecutionStep, String)> {
        self.steps
            .iter()
            .enumerate()
            .find(|(_, s)| matches!(s.status, ExecutionStepStatus::Failed))
            .map(|(i, s)| {
                let context = if i > 0 {
                    format!("after completing: {}", self.steps[i - 1].description)
                } else {
                    "at start of execution".to_string()
                };
                (i, s, context)
            })
    }

    pub fn format_summary(&self) -> String {
        let completed = self
            .steps
            .iter()
            .filter(|s| matches!(s.status, ExecutionStepStatus::Completed))
            .count();
        let failed = self
            .steps
            .iter()
            .filter(|s| matches!(s.status, ExecutionStepStatus::Failed))
            .count();
        let total = self.steps.len();

        if failed > 0 {
            format!(
                "{} of {} steps completed, {} failed",
                completed, total, failed
            )
        } else {
            format!("{} of {} steps completed", completed, total)
        }
    }

    /// Generate smart next actions based on execution outcome
    pub fn generate_next_actions(&self) -> Vec<NextAction> {
        let mut actions = vec![];

        // Check if there's a failed step
        if let Some((failed_idx, failed_step, _)) = self.get_failure_info() {
            actions.push(NextAction::RetryFailedStep {
                step_id: failed_idx,
            });
            actions.push(NextAction::ShowLogs);

            // If failed step was a file operation, suggest manual edit
            if matches!(
                failed_step.action,
                StepAction::WriteFile { .. }
                    | StepAction::PatchFile { .. }
                    | StepAction::CreateDirectory { .. }
            ) {
                actions.push(NextAction::ExplainChanges);
            }

            return actions;
        }

        // Success case - generate actions based on what was done
        let has_file_writes = self.steps.iter().any(|s| {
            matches!(
                s.action,
                StepAction::WriteFile { .. } | StepAction::PatchFile { .. }
            )
        });
        let has_server_start = self
            .steps
            .iter()
            .any(|s| matches!(s.action, StepAction::StartServer { .. }));
        let has_validation = self
            .steps
            .iter()
            .any(|s| matches!(s.action, StepAction::ValidateProject));
        let has_build = self
            .steps
            .iter()
            .any(|s| matches!(s.action, StepAction::Build));

        // Get last written file for OpenFile suggestion
        if has_file_writes {
            if let Some(last_write) = self
                .steps
                .iter()
                .rev()
                .find(|s| matches!(s.action, StepAction::WriteFile { .. }))
                && let StepAction::WriteFile { path } = &last_write.action
            {
                actions.push(NextAction::OpenFile { path: path.clone() });
            }
            actions.push(NextAction::OpenDiff);
        }

        if has_server_start {
            actions.push(NextAction::ContinueTask); // Could be "Open browser"
            actions.push(NextAction::StopServer);
        }

        if has_validation || has_build {
            actions.push(NextAction::RunValidation);
        }

        // Always offer explain
        if has_file_writes {
            actions.push(NextAction::ExplainChanges);
        }

        actions
    }

    /// Get all artifacts produced by this execution
    pub fn get_artifacts(&self) -> Vec<Artifact> {
        let mut artifacts = vec![];

        for step in &self.steps {
            if let Some(result) = &step.result {
                // File artifacts
                for path in &result.affected_paths {
                    artifacts.push(Artifact::File(path.clone()));
                }
                // URL artifacts
                for url in &result.artifact_urls {
                    artifacts.push(Artifact::Url(url.clone()));
                }
            }
        }

        artifacts
    }

    /// Format "What changed" section for post-run summary
    pub fn format_what_changed(&self) -> Vec<String> {
        let mut changes = vec![];

        for step in &self.steps {
            if let StepAction::WriteFile { path } = &step.action {
                if step.status == ExecutionStepStatus::Completed {
                    changes.push(format!("+ {}", path));
                }
            } else if let StepAction::CreateDirectory { path } = &step.action {
                if step.status == ExecutionStepStatus::Completed {
                    changes.push(format!("+ {}/", path));
                }
            } else if let StepAction::PatchFile { path } = &step.action
                && step.status == ExecutionStepStatus::Completed
            {
                changes.push(format!("~ {}", path));
            }
        }

        changes
    }
}

/// Actions the user can take after execution completes or fails
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NextAction {
    /// Retry a specific failed step
    RetryFailedStep { step_id: usize },
    /// Continue with the next part of a task
    ContinueTask,
    /// Open a specific file
    OpenFile { path: String },
    /// Open the diff view
    OpenDiff,
    /// Run the validation pipeline
    RunValidation,
    /// Start a server
    StartServer { command: String },
    /// Stop a running server
    StopServer,
    /// Explain what changes were made
    ExplainChanges,
    /// Show execution logs
    ShowLogs,
    /// Undo the last action
    UndoLastAction,
    /// Accept all changes
    AcceptChanges,
    /// Reject/discards changes
    RejectChanges,
}

impl NextAction {
    /// Display label for the action
    pub fn label(&self) -> String {
        match self {
            NextAction::RetryFailedStep { step_id } => format!("Retry failed step {}", step_id + 1),
            NextAction::ContinueTask => "Continue task".to_string(),
            NextAction::OpenFile { path } => format!("Open file: {}", path),
            NextAction::OpenDiff => "Open diff".to_string(),
            NextAction::RunValidation => "Run validation".to_string(),
            NextAction::StartServer { command } => format!("Start server: {}", command),
            NextAction::StopServer => "Stop server".to_string(),
            NextAction::ExplainChanges => "Explain changes".to_string(),
            NextAction::ShowLogs => "Show logs".to_string(),
            NextAction::UndoLastAction => "Undo last action".to_string(),
            NextAction::AcceptChanges => "Accept changes".to_string(),
            NextAction::RejectChanges => "Reject changes".to_string(),
        }
    }
}

/// Artifacts produced during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Artifact {
    File(String),
    Url(String),
    Log(String),
}

/// Final outcome of an execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    pub success: bool,
    pub summary: String,
    pub affected_files: Vec<String>,
    pub artifacts: Vec<Artifact>,
    pub next_actions: Vec<NextAction>,
    pub what_changed: Vec<String>,
}

impl ExecutionOutcome {
    pub fn from_plan(plan: &ExecutionPlan, success: bool) -> Self {
        let affected_files: Vec<_> = plan
            .steps
            .iter()
            .filter(|s| s.status == ExecutionStepStatus::Completed)
            .filter_map(|s| s.result.as_ref())
            .flat_map(|r| r.affected_paths.clone())
            .collect();

        let artifacts = plan.get_artifacts();
        let next_actions = plan.generate_next_actions();
        let what_changed = plan.format_what_changed();

        Self {
            success,
            summary: plan.format_summary(),
            affected_files,
            artifacts,
            next_actions,
            what_changed,
        }
    }

    /// Format the post-run summary for display
    pub fn format_post_run_summary(&self) -> String {
        let mut lines = vec![];

        if self.success {
            lines.push("✓ Execution complete".to_string());
        } else {
            lines.push("✗ Execution failed".to_string());
        }

        lines.push("".to_string());
        lines.push(self.summary.clone());

        if !self.affected_files.is_empty() {
            lines.push(format!("{} files affected", self.affected_files.len()));
        }

        if !self.artifacts.is_empty() {
            let url_artifacts: Vec<_> = self
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
        }

        if !self.what_changed.is_empty() {
            lines.push("".to_string());
            lines.push("What changed:".to_string());
            for change in &self.what_changed {
                lines.push(format!("  {}", change));
            }
        }

        if !self.next_actions.is_empty() {
            lines.push("".to_string());
            lines.push("Next:".to_string());
            for action in &self.next_actions[..self.next_actions.len().min(4)] {
                lines.push(format!("  → {}", action.label()));
            }
        }

        lines.join("\n")
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionTrace {
    pub mode: ExecutionMode,
    pub state: ExecutionState,
    pub active_objective: Option<String>,
    pub last_action: String,
    pub current_step: Option<String>,
    pub step_index: Option<u32>,
    pub step_total: Option<u32>,
    pub active_tool: Option<String>,
    pub planner_output: Vec<String>,
    pub tool_calls: Vec<String>,
    pub file_writes: Vec<String>,
    pub validation_summary: Option<String>,
    pub block_reason: Option<String>,
    pub block_fix: Option<String>,
    pub block_command: Option<String>,
    /// Unified execution plan - replaces fragmented flow
    pub current_plan: Option<ExecutionPlan>,
}

impl ExecutionTrace {
    pub fn new() -> Self {
        Self {
            mode: ExecutionMode::Task,
            state: ExecutionState::Idle,
            active_objective: None,
            last_action: "none".to_string(),
            current_step: None,
            step_index: None,
            step_total: None,
            active_tool: None,
            planner_output: vec![],
            tool_calls: vec![],
            file_writes: vec![],
            validation_summary: None,
            block_reason: None,
            block_fix: None,
            block_command: None,
            current_plan: None,
        }
    }

    pub fn clear_runtime_activity(&mut self) {
        self.current_step = None;
        self.step_index = None;
        self.step_total = None;
        self.active_tool = None;
        self.planner_output.clear();
        self.tool_calls.clear();
        self.file_writes.clear();
        self.validation_summary = None;
        self.current_plan = None;
    }

    pub fn clear_block(&mut self) {
        self.block_reason = None;
        self.block_fix = None;
        self.block_command = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String, // Reserved for transcript persistence and deduplication
    pub role: MessageRole,
    /// Raw source text (for copy/paste) - Phase 2
    pub source_text: String,
    /// Display content (may differ from source_text for rendering)
    pub content: String,
    pub timestamp: DateTime<Local>,
    /// Optional run card for structured Forge execution display
    pub run_card: Option<RunCard>,
}

/// Structured Forge execution display - groups all runtime events into one transcript entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCard {
    pub task: String,
    pub session_id: String,
    pub status: RuntimeStatus,
    pub phase: String,
    pub events: Vec<String>,
    pub iterations: u32,
    pub model: Option<String>,
    pub started_at: DateTime<Local>,
    pub finished_at: Option<DateTime<Local>>,
    pub result_message: Option<String>,
    #[serde(default)]
    pub current_step: Option<String>,
    #[serde(default)]
    pub active_tool: Option<String>,
    #[serde(default)]
    pub validation_summary: Option<String>,
}

impl RunCard {
    pub fn new(task: String, session_id: String, model: Option<String>) -> Self {
        Self {
            task,
            session_id,
            status: RuntimeStatus::Running,
            phase: "starting".to_string(),
            events: vec![],
            iterations: 0,
            model,
            started_at: Local::now(),
            finished_at: None,
            result_message: None,
            current_step: Some("initializing".to_string()),
            active_tool: None,
            validation_summary: None,
        }
    }

    pub fn add_event(&mut self, event: String) {
        // Keep last 6 events to prevent overflow
        if self.events.len() >= 6 {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    pub fn finish(&mut self, success: bool, message: String, iterations: u32) {
        self.status = if success {
            RuntimeStatus::Completed
        } else {
            RuntimeStatus::Error
        };
        self.phase = if success { "completed" } else { "failed" }.to_string();
        self.result_message = Some(message);
        self.iterations = iterations;
        self.finished_at = Some(Local::now());
    }

    pub fn duration_secs(&self) -> u64 {
        let end = self.finished_at.unwrap_or_else(Local::now);
        let duration = end.signed_duration_since(self.started_at);
        duration.num_seconds() as u64
    }
}

#[derive(Debug, Clone)]
pub struct RepoContext {
    pub name: String,
    pub path: String,
    pub display_path: String,
    pub branch: Option<String>,
    pub git_detected: bool,
}

impl Default for RepoContext {
    fn default() -> Self {
        Self {
            name: "No repo".to_string(),
            path: "~".to_string(),
            display_path: "~".to_string(),
            branch: None,
            git_detected: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModelContext {
    pub configured: Option<String>,
    pub active: Option<String>,
    pub available: Vec<String>,
    pub connected: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeEvent {
    pub timestamp: DateTime<Local>,
    pub stage: String,
    pub status: RuntimeStatus,
}

#[derive(Debug, Clone)]
pub struct ValidationStage {
    pub name: String,
    pub status: RuntimeStatus,
    pub detail: Option<String>,
    pub duration_ms: Option<u64>,
}

impl ValidationStage {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            status: RuntimeStatus::Idle,
            detail: None,
            duration_ms: None,
        }
    }
}

pub fn default_validation_stages() -> Vec<ValidationStage> {
    ["protocol", "validation", "syntax", "lint", "build", "test"]
        .into_iter()
        .map(ValidationStage::new)
        .collect()
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: DateTime<Local>,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructuredOutputKind {
    Objective,
    Plan,
    ArtifactManifest,
    Audit,
    Checkpoint,
    Recovery,
    Status,
    Markdown,
    Unknown,
}

impl StructuredOutputKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Objective => "Objective",
            Self::Plan => "Plan",
            Self::ArtifactManifest => "Artifact",
            Self::Audit => "Audit",
            Self::Checkpoint => "Checkpoint",
            Self::Recovery => "Recovery",
            Self::Status => "Status",
            Self::Markdown => "Markdown",
            Self::Unknown => "Structured",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredOutput {
    pub id: String,
    pub kind: StructuredOutputKind,
    pub title: String,
    pub source: String,
    pub content: String,
    pub timestamp: DateTime<Local>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug, // Reserved for detailed debugging output
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    // Input state
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub cursor_position: usize,

    // Chat state
    pub messages: Vec<Message>,
    pub scroll_offset: usize, // Legacy - migrating to chat_scroll

    // Phase 1: Chat scroll state
    pub chat_scroll: crate::app::ChatScrollState,

    // Phase 1: Focus management
    pub focus_state: crate::app::FocusState,

    // Context
    pub repo: RepoContext,
    pub model: ModelContext,
    pub runtime_status: RuntimeStatus,
    pub ollama_connected: bool,
    pub execution: ExecutionTrace,

    // RLEF: Reinforcement Learning from Execution Feedback
    pub rlef_memory: RLEFMemory,
    pub rlef_transparency: Option<PlanRLEFTransparency>,

    // Inspector data
    pub runtime_events: Vec<RuntimeEvent>,
    pub validation_stages: Vec<ValidationStage>,
    pub logs: Vec<LogEntry>,
    pub structured_outputs: Vec<StructuredOutput>,
    pub active_inspector_tab: InspectorTab,

    // Scroll positions for inspector tabs (preserved across renders)
    pub runtime_tab_scroll: usize,
    pub validation_tab_scroll: usize,
    pub logs_tab_scroll: usize,
    pub preview_tab_scroll: usize,
    pub diff_tab_scroll: usize,
    // New observability tab scroll positions
    pub timeline_tab_scroll: usize,
    pub audit_tab_scroll: usize,
    pub failure_tab_scroll: usize,
    pub steps_tab_scroll: usize,
    pub planner_trace_tab_scroll: usize,
    pub replay_tab_scroll: usize,
    pub debug_bundle_tab_scroll: usize,

    // Observability view models
    pub execution_timeline: Option<crate::observability::ExecutionTimelineView>,
    pub failure_explanation: Option<crate::observability::FailureExplanationView>,
    pub step_summaries: Vec<crate::observability::StepSummaryView>,
    pub planner_traces: Vec<crate::observability::PlannerTraceView>,
    pub replay_comparison: Option<crate::observability::ReplayComparisonView>,
    pub debug_bundle_path: Option<String>,
    pub debug_bundle_exported: bool,

    // Step navigation state
    pub selected_step_index: Option<usize>,
    pub focused_step_index: Option<usize>, // Step with failure or first mismatch

    // Browser preview servers
    pub preview_servers: Vec<crate::browser::PreviewServer>,

    // File mutation diffs for inspector
    pub diff_store: crate::diff::DiffStore,

    // Chain execution tracking
    pub current_chain_id: Option<String>,
    pub current_chain_step_id: Option<String>,
    // V2.5: When current chain started (for cooldown in auto-resume)
    pub current_chain_start_time: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Runtime,
    Validation,
    Logs,
    Preview,
    Diff,
    // New observability tabs
    Timeline,
    Failure,
    Steps,
    PlannerTrace,
    Replay,
    DebugBundle,
    Audit,      // V1.6: Execution audit timeline
    Checkpoint, // V1.6: Checkpoint validation and recovery truth
    Recovery,   // V1.6: Self-healing and completion-confidence decisions
}

impl InspectorTab {
    pub fn as_str(&self) -> &'static str {
        match self {
            InspectorTab::Runtime => "Runtime",
            InspectorTab::Validation => "Validation",
            InspectorTab::Logs => "Logs",
            InspectorTab::Preview => "Preview",
            InspectorTab::Diff => "Diff",
            InspectorTab::Timeline => "Timeline",
            InspectorTab::Failure => "Failure",
            InspectorTab::Steps => "Steps",
            InspectorTab::PlannerTrace => "Planner",
            InspectorTab::Replay => "Replay",
            InspectorTab::DebugBundle => "Bundle",
            InspectorTab::Audit => "Audit",
            InspectorTab::Checkpoint => "Checkpoint",
            InspectorTab::Recovery => "Recovery",
        }
    }

    /// Get the next tab in the cycle
    pub fn next(self) -> Self {
        match self {
            Self::Runtime => Self::Validation,
            Self::Validation => Self::Logs,
            Self::Logs => Self::Preview,
            Self::Preview => Self::Diff,
            Self::Diff => Self::Timeline,
            Self::Timeline => Self::Steps,
            Self::Steps => Self::PlannerTrace,
            Self::PlannerTrace => Self::Replay,
            Self::Replay => Self::DebugBundle,
            Self::DebugBundle => Self::Audit,
            Self::Audit => Self::Checkpoint,
            Self::Checkpoint => Self::Recovery,
            Self::Recovery => Self::Failure,
            Self::Failure => Self::Runtime,
        }
    }

    /// Get the previous tab in the cycle
    pub fn previous(self) -> Self {
        match self {
            Self::Runtime => Self::Failure,
            Self::Validation => Self::Runtime,
            Self::Logs => Self::Validation,
            Self::Preview => Self::Logs,
            Self::Diff => Self::Preview,
            Self::Timeline => Self::Diff,
            Self::Steps => Self::Timeline,
            Self::PlannerTrace => Self::Steps,
            Self::Replay => Self::PlannerTrace,
            Self::DebugBundle => Self::Replay,
            Self::Audit => Self::DebugBundle,
            Self::Checkpoint => Self::Audit,
            Self::Recovery => Self::Checkpoint,
            Self::Failure => Self::Recovery,
        }
    }
}

/// Browser preview server state
#[derive(Debug, Clone)]
pub struct BrowserPreviewServer {
    pub url: String,
    pub port: u16,
    pub directory: String,
    pub started_at: DateTime<Local>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            input_mode: InputMode::Editing,
            input_buffer: String::new(),
            cursor_position: 0,
            messages: vec![],
            scroll_offset: 0,
            repo: RepoContext::default(),
            model: ModelContext::default(),
            runtime_status: RuntimeStatus::Idle,
            ollama_connected: false,
            execution: ExecutionTrace::new(),
            rlef_memory: RLEFMemory::new(),
            rlef_transparency: None,
            runtime_events: vec![],
            validation_stages: default_validation_stages(),
            logs: vec![],
            structured_outputs: vec![],
            active_inspector_tab: InspectorTab::Runtime,

            // Scroll positions initialized to top
            runtime_tab_scroll: 0,
            validation_tab_scroll: 0,
            logs_tab_scroll: 0,
            preview_tab_scroll: 0,
            diff_tab_scroll: 0,
            // New observability tab scroll positions
            timeline_tab_scroll: 0,
            audit_tab_scroll: 0,
            failure_tab_scroll: 0,
            steps_tab_scroll: 0,
            planner_trace_tab_scroll: 0,
            replay_tab_scroll: 0,
            debug_bundle_tab_scroll: 0,

            // Observability view models initialized empty
            execution_timeline: None,
            failure_explanation: None,
            step_summaries: vec![],
            planner_traces: vec![],
            replay_comparison: None,
            debug_bundle_path: None,
            debug_bundle_exported: false,

            // Step navigation state
            selected_step_index: None,
            focused_step_index: None,

            preview_servers: vec![],
            diff_store: crate::diff::DiffStore::new(50),

            chat_scroll: crate::app::ChatScrollState::new(),
            focus_state: crate::app::FocusState::new(),

            // Chain execution tracking - initialized to None
            current_chain_id: None,
            current_chain_step_id: None,
            current_chain_start_time: None,
        }
    }

    pub fn format_time(&self, dt: &DateTime<Local>) -> String {
        dt.format("%H:%M:%S").to_string()
    }

    // ============================================================================
    // Failure-First UX Methods
    // ============================================================================

    /// Switch to the Failure tab and focus on the failure point
    pub fn focus_failure(&mut self) {
        self.active_inspector_tab = InspectorTab::Failure;

        // Find the failing step index
        if let Some(timeline) = &self.execution_timeline {
            if let Some(failure_idx) = timeline.first_failure_index() {
                self.failure_tab_scroll = failure_idx;

                // Also focus the corresponding step
                if let Some(step_idx) = timeline
                    .entries
                    .get(failure_idx)
                    .and_then(|e| e.step_index.map(|s| s as usize))
                {
                    self.focused_step_index = Some(step_idx);
                    self.selected_step_index = Some(step_idx);
                }
            }
        }
    }

    /// Check if there's an active failure that should auto-focus
    pub fn should_auto_focus_failure(&self) -> bool {
        if let Some(timeline) = &self.execution_timeline {
            timeline.has_failure() && self.active_inspector_tab != InspectorTab::Failure
        } else {
            false
        }
    }

    /// Get compact failure notice for footer display
    pub fn failure_notice(&self) -> Option<String> {
        if let Some(exp) = &self.failure_explanation {
            let step_info = exp
                .step_index
                .map(|s| format!(" at step {}", s + 1))
                .unwrap_or_default();

            Some(format!(
                "❌ {} failed{}: {} | {}",
                exp.class, step_info, exp.short_message, exp.suggested_next_action
            ))
        } else {
            None
        }
    }

    // ============================================================================
    // Step Navigation Methods
    // ============================================================================

    /// Navigate to the next step
    pub fn next_step(&mut self) {
        if let Some(current) = self.selected_step_index {
            let max_step = self.step_summaries.len().saturating_sub(1);
            if current < max_step {
                self.selected_step_index = Some(current + 1);
            }
        } else if !self.step_summaries.is_empty() {
            self.selected_step_index = Some(0);
        }
    }

    /// Navigate to the previous step
    pub fn previous_step(&mut self) {
        if let Some(current) = self.selected_step_index {
            if current > 0 {
                self.selected_step_index = Some(current - 1);
            }
        }
    }

    /// Jump to the first failing step
    pub fn jump_to_first_failure(&mut self) {
        if let Some(timeline) = &self.execution_timeline {
            if let Some(failure_idx) = timeline.first_failure_index() {
                self.selected_step_index = Some(failure_idx);
                self.focused_step_index = Some(failure_idx);
                self.active_inspector_tab = InspectorTab::Steps;
            }
        }
    }

    /// Jump to the first reverted step
    pub fn jump_to_first_revert(&mut self) {
        if let Some(idx) = self
            .step_summaries
            .iter()
            .position(|s| matches!(s.status, crate::observability::StepStatus::Reverted))
        {
            self.selected_step_index = Some(idx);
            self.active_inspector_tab = InspectorTab::Steps;
        }
    }

    /// Jump to the last committed step
    pub fn jump_to_last_commit(&mut self) {
        if let Some(idx) = self
            .step_summaries
            .iter()
            .rposition(|s| matches!(s.status, crate::observability::StepStatus::Passed))
        {
            self.selected_step_index = Some(idx);
            self.active_inspector_tab = InspectorTab::Steps;
        }
    }

    /// Get the currently selected step view
    pub fn selected_step(&self) -> Option<&crate::observability::StepSummaryView> {
        self.selected_step_index
            .and_then(|idx| self.step_summaries.get(idx))
    }

    // ============================================================================
    // Tab Navigation
    // ============================================================================

    /// Switch to the next inspector tab
    pub fn next_inspector_tab(&mut self) {
        self.active_inspector_tab = self.active_inspector_tab.next();
    }

    /// Switch to the previous inspector tab
    pub fn previous_inspector_tab(&mut self) {
        self.active_inspector_tab = self.active_inspector_tab.previous();
    }

    // ============================================================================
    // Replay Workflow Methods
    // ============================================================================

    /// Jump to replay mismatch section
    pub fn jump_to_replay_mismatch(&mut self) {
        if let Some(replay) = &self.replay_comparison {
            if !replay.matched {
                self.active_inspector_tab = InspectorTab::Replay;
            }
        }
    }

    /// Get replay status summary
    pub fn replay_status_summary(&self) -> String {
        if let Some(replay) = &self.replay_comparison {
            if replay.matched {
                "✓ Replay matched".to_string()
            } else if let Some(first) = &replay.first_mismatch {
                format!("✗ Replay diverged at {}", first.section)
            } else {
                "✗ Replay mismatch".to_string()
            }
        } else {
            "○ Replay not checked".to_string()
        }
    }

    // ============================================================================
    // Debug Bundle Methods
    // ============================================================================

    /// Mark debug bundle as exported
    pub fn mark_bundle_exported(&mut self, path: String) {
        self.debug_bundle_exported = true;
        self.debug_bundle_path = Some(path);
    }

    /// Check if a debug bundle is available
    pub fn has_debug_bundle(&self) -> bool {
        self.debug_bundle_exported || self.debug_bundle_path.is_some()
    }

    /// Get debug bundle status text
    pub fn debug_bundle_status(&self) -> String {
        if self.debug_bundle_exported {
            if let Some(path) = &self.debug_bundle_path {
                format!("✓ Bundle exported: {}", path)
            } else {
                "✓ Bundle exported".to_string()
            }
        } else {
            "○ Bundle not exported".to_string()
        }
    }

    // ============================================================================
    // Compact Run Summary
    // ============================================================================

    /// Generate compact end-of-run summary
    pub fn run_summary(&self) -> crate::observability::RunSummaryView {
        use crate::observability::RunSummaryView;

        // Determine outcome from timeline
        if let Some(timeline) = &self.execution_timeline {
            match timeline.outcome {
                crate::observability::TimelineOutcome::Success => {
                    RunSummaryView::success(timeline.entries.len(), timeline.entries.len())
                }
                _ => {
                    let failed_step = self.focused_step_index.unwrap_or(0);
                    RunSummaryView::failure(failed_step, "Execution failed")
                }
            }
        } else {
            RunSummaryView::success(0, 0)
        }
    }

    /// Update run summary with replay status
    pub fn update_summary_with_replay(
        &self,
        mut summary: crate::observability::RunSummaryView,
    ) -> crate::observability::RunSummaryView {
        summary.replay_status = self.replay_status_summary();
        summary
    }

    /// Update run summary with bundle status
    pub fn update_summary_with_bundle(
        &self,
        mut summary: crate::observability::RunSummaryView,
    ) -> crate::observability::RunSummaryView {
        summary.bundle_exported = self.has_debug_bundle();
        summary.bundle_path = self.debug_bundle_path.clone();
        summary
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// RLEF (Reinforcement Learning from Execution Feedback) System
// =============================================================================

/// Tight, actionable failure taxonomy for RLEF
/// Each variant implies what the planner should do differently
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionFailureClassV1 {
    // Validation failures - planner should structure differently
    ValidationRejectMissingField,
    ValidationRejectTypeMismatch,
    ValidationRejectSyntaxError,
    ValidationRejectDependencyConflict,

    // Tool failures - planner should use different tools/patterns
    ToolFailureFileNotFound,
    ToolFailurePermissionDenied,
    ToolFailureCommandNotFound,
    ToolFailureCommandExitNonZero,
    ToolFailureNetworkUnreachable,
    ToolFailureTimeout,

    // Mode/policy blocks - planner should adjust approach
    ModeBlockedCommandExecution,
    ModeBlockedFileWrite,
    ModeBlockedServerStart,

    // Step ordering failures - planner should reorder
    StepOrderReadBeforeWrite,
    StepOrderValidateBeforeGenerate,
    StepOrderMissingDependency,

    // Resource failures - planner should provision differently
    ResourceFailureDiskFull,
    ResourceFailureMemoryExceeded,
    ResourceFailurePortInUse,

    // Unknown/unclassified
    UnknownFailure,
}

impl ExecutionFailureClassV1 {
    /// Get actionable guidance for the planner
    pub fn planner_guidance(&self) -> &'static str {
        match self {
            Self::ValidationRejectMissingField => {
                "Always include required fields in generated files"
            }
            Self::ValidationRejectTypeMismatch => "Validate types before generating config",
            Self::ValidationRejectSyntaxError => "Verify syntax with linter before writing",
            Self::ValidationRejectDependencyConflict => "Check dependency versions before install",

            Self::ToolFailureFileNotFound => "Verify file exists before reading/patching",
            Self::ToolFailurePermissionDenied => "Check permissions before write operations",
            Self::ToolFailureCommandNotFound => "Verify command available before execution",
            Self::ToolFailureCommandExitNonZero => "Add error handling for command execution",
            Self::ToolFailureNetworkUnreachable => "Handle network failures gracefully",
            Self::ToolFailureTimeout => "Add timeout handling for long operations",

            Self::ModeBlockedCommandExecution => "Respect execution mode boundaries",
            Self::ModeBlockedFileWrite => "Request mode upgrade for file operations",
            Self::ModeBlockedServerStart => "Request mode upgrade for server operations",

            Self::StepOrderReadBeforeWrite => "Read existing files before overwriting",
            Self::StepOrderValidateBeforeGenerate => "Validate context before generating",
            Self::StepOrderMissingDependency => "Ensure dependencies exist before use",

            Self::ResourceFailureDiskFull => "Check disk space before large operations",
            Self::ResourceFailureMemoryExceeded => "Monitor memory during build/test",
            Self::ResourceFailurePortInUse => "Check port availability before starting server",

            Self::UnknownFailure => "Review execution logs for patterns",
        }
    }

    /// Classify from error message (best-effort heuristic)
    pub fn classify_from_error(error: &str, step_action: &StepAction) -> Self {
        let error_lower = error.to_lowercase();

        // Validation patterns
        if error_lower.contains("missing field") || error_lower.contains("required field") {
            return Self::ValidationRejectMissingField;
        }
        if error_lower.contains("type mismatch") || error_lower.contains("invalid type") {
            return Self::ValidationRejectTypeMismatch;
        }
        if error_lower.contains("syntax error") || error_lower.contains("parse error") {
            return Self::ValidationRejectSyntaxError;
        }

        // Tool patterns
        if error_lower.contains("file not found") || error_lower.contains("no such file") {
            return Self::ToolFailureFileNotFound;
        }
        if error_lower.contains("permission denied") || error_lower.contains("access denied") {
            return Self::ToolFailurePermissionDenied;
        }
        if error_lower.contains("command not found") || error_lower.contains("not installed") {
            return Self::ToolFailureCommandNotFound;
        }
        if error_lower.contains("exit code") || error_lower.contains("failed with code") {
            return Self::ToolFailureCommandExitNonZero;
        }
        if error_lower.contains("network") || error_lower.contains("unreachable") {
            return Self::ToolFailureNetworkUnreachable;
        }
        if error_lower.contains("timeout") || error_lower.contains("timed out") {
            return Self::ToolFailureTimeout;
        }

        // Mode patterns
        if error_lower.contains("blocked by mode") || error_lower.contains("not allowed in") {
            match step_action {
                StepAction::RunCommand { .. } => return Self::ModeBlockedCommandExecution,
                StepAction::WriteFile { .. } | StepAction::PatchFile { .. } => {
                    return Self::ModeBlockedFileWrite;
                }
                StepAction::StartServer { .. } => return Self::ModeBlockedServerStart,
                _ => return Self::ModeBlockedCommandExecution,
            }
        }

        // Resource patterns
        if error_lower.contains("disk full") || error_lower.contains("no space") {
            return Self::ResourceFailureDiskFull;
        }
        if error_lower.contains("memory") || error_lower.contains("out of memory") {
            return Self::ResourceFailureMemoryExceeded;
        }
        if error_lower.contains("port") || error_lower.contains("address already in use") {
            return Self::ResourceFailurePortInUse;
        }

        Self::UnknownFailure
    }
}

/// Individual execution feedback entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionFeedback {
    pub timestamp: DateTime<Local>,
    pub plan_id: String,
    pub step_index: usize,
    pub step_action: StepAction,
    pub success: bool,
    pub failure_class: Option<ExecutionFailureClassV1>,
    pub error_message: Option<String>,
    pub context: Option<String>,
}

/// Compressed learning signal from RLEF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RLEFHint {
    /// The failure class this hint addresses
    pub failure_class: ExecutionFailureClassV1,
    /// Actionable guidance for the planner
    pub guidance: String,
    /// Evidence count (how many times this pattern was observed)
    pub evidence_count: u32,
    /// Confidence score (0.0 - 1.0), decays over time
    pub confidence: f32,
    /// When this hint was first created
    pub first_seen: DateTime<Local>,
    /// When this hint was last reinforced
    pub last_reinforced: DateTime<Local>,
    /// Number of times this hint helped avoid the failure
    pub success_count: u32,
    /// Whether this hint is currently active
    pub active: bool,
    /// Source plan IDs that contributed to this hint
    pub source_plans: Vec<String>,
}

impl RLEFHint {
    /// Create a new hint from feedback
    pub fn from_feedback(feedback: &ExecutionFeedback, guidance: &str) -> Self {
        let now = Local::now();
        Self {
            failure_class: feedback
                .failure_class
                .unwrap_or(ExecutionFailureClassV1::UnknownFailure),
            guidance: guidance.to_string(),
            evidence_count: 1,
            confidence: 0.5,
            first_seen: now,
            last_reinforced: now,
            success_count: 0,
            active: true,
            source_plans: vec![feedback.plan_id.clone()],
        }
    }

    /// Reinforce this hint with new evidence
    pub fn reinforce(&mut self, feedback: &ExecutionFeedback) {
        self.evidence_count += 1;
        self.last_reinforced = Local::now();

        // Confidence increases with evidence, caps at 0.95
        self.confidence = (self.confidence + 0.1).min(0.95);

        // Track source
        if !self.source_plans.contains(&feedback.plan_id) {
            self.source_plans.push(feedback.plan_id.clone());
        }
    }

    /// Check if hint meets minimum evidence threshold
    pub fn meets_threshold(&self, min_evidence: u32) -> bool {
        self.active && self.evidence_count >= min_evidence
    }
}

/// RLEF memory with compression and governance
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RLEFMemory {
    /// All hints indexed by failure class
    pub hints: std::collections::HashMap<ExecutionFailureClassV1, Vec<RLEFHint>>,
    /// Raw feedback history (last 100 entries for analysis)
    pub recent_feedback: Vec<ExecutionFeedback>,
    /// Maximum feedback history size
    max_feedback_history: usize,
    /// Minimum evidence threshold for hint activation
    min_evidence_threshold: u32,
}

impl RLEFMemory {
    pub fn new() -> Self {
        Self {
            hints: std::collections::HashMap::new(),
            recent_feedback: Vec::new(),
            max_feedback_history: 100,
            min_evidence_threshold: 3, // Require 3 occurrences before hinting
        }
    }

    /// Record feedback and update hints
    pub fn record_feedback(&mut self, feedback: ExecutionFeedback) {
        // Add to recent feedback
        self.recent_feedback.push(feedback.clone());
        if self.recent_feedback.len() > self.max_feedback_history {
            self.recent_feedback.remove(0);
        }

        // Skip if not a failure or no classification
        if feedback.success || feedback.failure_class.is_none() {
            return;
        }

        let failure_class = feedback.failure_class.unwrap();
        let guidance = failure_class.planner_guidance();

        // Find or create hint
        let hints_for_class = self.hints.entry(failure_class).or_default();

        if let Some(existing) = hints_for_class.iter_mut().find(|h| h.guidance == guidance) {
            // Reinforce existing hint
            existing.reinforce(&feedback);
        } else {
            // Create new hint
            let hint = RLEFHint::from_feedback(&feedback, guidance);
            hints_for_class.push(hint);
        }
    }

    /// Get active hints that meet the evidence threshold
    pub fn get_active_hints(&self) -> Vec<&RLEFHint> {
        self.hints
            .values()
            .flat_map(|hints| hints.iter())
            .filter(|h| h.meets_threshold(self.min_evidence_threshold))
            .collect()
    }

    /// Clear all memory (nuclear option)
    pub fn clear(&mut self) {
        self.hints.clear();
        self.recent_feedback.clear();
    }

    /// Get hint statistics for transparency
    pub fn stats(&self) -> RLEFStats {
        let total_hints: usize = self.hints.values().map(|v| v.len()).sum();
        let active_hints: usize = self.get_active_hints().len();

        RLEFStats {
            total_hints,
            active_hints,
            total_feedback: self.recent_feedback.len(),
            min_evidence_threshold: self.min_evidence_threshold,
        }
    }
}

/// Statistics for RLEF transparency
#[derive(Debug, Clone)]
pub struct RLEFStats {
    pub total_hints: usize,
    pub active_hints: usize,
    pub total_feedback: usize,
    pub min_evidence_threshold: u32,
}

/// Transparency entry for showing RLEF influence on a plan
#[derive(Debug, Clone)]
pub struct HintTransparency {
    /// Which hint was applied
    pub hint: RLEFHint,
    /// How it influenced this specific step
    pub influence_description: String,
    /// Step index that was modified
    pub step_index: usize,
}

/// Tracks RLEF influence on a plan generation
#[derive(Debug, Clone, Default)]
pub struct PlanRLEFTransparency {
    /// Whether RLEF influenced this plan
    pub influenced: bool,
    /// Which hints were applied
    pub applied_hints: Vec<HintTransparency>,
    /// General guidance from RLEF
    pub general_guidance: Vec<String>,
}

impl PlanRLEFTransparency {
    /// Create from active hints
    pub fn from_active_hints(hints: &[&RLEFHint], plan_steps: &[ExecutionStep]) -> Self {
        let mut transparency = Self::default();

        for hint in hints {
            // Find which step this hint might apply to
            for (idx, step) in plan_steps.iter().enumerate() {
                if Self::hint_applies_to_step(hint, step) {
                    transparency.applied_hints.push(HintTransparency {
                        hint: (*hint).clone(),
                        influence_description: format!(
                            "Learned from {} prior failures",
                            hint.evidence_count
                        ),
                        step_index: idx,
                    });
                    transparency.influenced = true;
                }
            }

            transparency.general_guidance.push(hint.guidance.clone());
        }

        transparency
    }

    /// Check if a hint applies to a specific step
    fn hint_applies_to_step(hint: &RLEFHint, step: &ExecutionStep) -> bool {
        match hint.failure_class {
            ExecutionFailureClassV1::ValidationRejectMissingField
            | ExecutionFailureClassV1::ValidationRejectSyntaxError => {
                matches!(
                    step.action,
                    StepAction::WriteFile { .. } | StepAction::PatchFile { .. }
                )
            }
            ExecutionFailureClassV1::ToolFailureFileNotFound => {
                matches!(
                    step.action,
                    StepAction::ReadFile { .. } | StepAction::PatchFile { .. }
                )
            }
            ExecutionFailureClassV1::StepOrderReadBeforeWrite => {
                matches!(step.action, StepAction::ReadFile { .. })
            }
            ExecutionFailureClassV1::ModeBlockedCommandExecution => {
                matches!(step.action, StepAction::RunCommand { .. })
            }
            _ => false,
        }
    }

    /// Format for inspector display
    pub fn format_for_inspector(&self) -> Vec<String> {
        let mut lines = vec![];

        if !self.influenced {
            lines.push("Plan generated (no RLEF influence)".to_string());
            return lines;
        }

        lines.push("Plan generated (RLEF influenced):".to_string());
        lines.push("".to_string());

        for hint_trans in &self.applied_hints {
            lines.push(format!(
                "  Step {}: {}",
                hint_trans.step_index + 1,
                hint_trans.influence_description
            ));
            lines.push(format!("     → {}", hint_trans.hint.guidance));
            lines.push("".to_string());
        }

        if !self.general_guidance.is_empty() {
            lines.push("General guidance:".to_string());
            for guidance in &self.general_guidance {
                lines.push(format!("  • {}", guidance));
            }
        }

        lines
    }
}
