use super::{ApprovalRequest, ConversationTurn, IntentSpec};
use chrono::{DateTime, Local};
use std::path::PathBuf;

/// Complete context for the current session
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: String,
    pub conversation: Vec<ConversationTurn>,
    pub recent_files: Vec<PathBuf>,
    pub recent_errors: Vec<ErrorRecord>,
    pub uncommitted_work: Option<UncommittedWork>,
    pub interaction_mode: super::InteractionMode,
    pub auto_approve: AutoApprovePolicy,
}

impl Default for SessionContext {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionContext {
    pub fn new() -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            conversation: Vec::new(),
            recent_files: Vec::new(),
            recent_errors: Vec::new(),
            uncommitted_work: None,
            interaction_mode: super::InteractionMode::Chat,
            auto_approve: AutoApprovePolicy::SafeOnly,
        }
    }

    pub fn add_turn(&mut self, turn: ConversationTurn) {
        self.conversation.push(turn);
        // Keep only last 50 turns
        if self.conversation.len() > 50 {
            self.conversation.remove(0);
        }
    }

    pub fn last_turn(&self) -> Option<&ConversationTurn> {
        self.conversation.last()
    }

    pub fn last_executing_turn(&self) -> Option<&ConversationTurn> {
        self.conversation
            .iter()
            .rev()
            .find(|t| t.execution_id.is_some())
    }

    /// Resolve "that" to a specific referent
    pub fn resolve_that(&self) -> ResolutionResult {
        // Priority: most recent error > most recent file modified > most recent action
        if let Some(error) = self.recent_errors.first() {
            return ResolutionResult::Error(error.clone());
        }
        if let Some(file) = self.recent_files.first() {
            return ResolutionResult::File(file.clone());
        }
        if let Some(turn) = self.conversation.last() {
            return ResolutionResult::Turn(turn.turn_id);
        }
        ResolutionResult::Ambiguous
    }

    pub fn suggest_recent_files(&self) -> Vec<String> {
        self.recent_files
            .iter()
            .take(5)
            .map(|p| p.display().to_string())
            .collect()
    }
}

/// Result of resolving a reference
#[derive(Debug, Clone)]
pub enum ResolutionResult {
    Turn(u32),
    File(PathBuf),
    Error(ErrorRecord),
    Ambiguous,
}

/// Record of an error encountered
#[derive(Debug, Clone)]
pub struct ErrorRecord {
    pub turn_id: u32,
    pub timestamp: DateTime<Local>,
    pub summary: String,
    pub file: Option<PathBuf>,
}

/// Captured state of paused execution
#[derive(Debug, Clone)]
pub struct UncommittedWork {
    pub paused_at: DateTime<Local>,
    pub intent_spec: IntentSpec,
    pub runtime_state: serde_json::Value,
    pub pending_approvals: Vec<ApprovalRequest>,
}

/// Policy for when to ask permission
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoApprovePolicy {
    Never,
    SafeOnly,
    WithinSession,
    Always,
}

impl AutoApprovePolicy {
    pub fn allows(&self, action_type: &super::ActionType) -> bool {
        match self {
            AutoApprovePolicy::Never => false,
            AutoApprovePolicy::Always => true,
            AutoApprovePolicy::SafeOnly => {
                matches!(action_type, super::ActionType::FileRead)
            }
            AutoApprovePolicy::WithinSession => {
                // Allows reads and idempotent operations
                !matches!(action_type, super::ActionType::FileDelete)
            }
        }
    }
}

/// Internal orchestrator state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestratorState {
    Idle,
    RefiningIntent,
    AwaitingClarification,
    Executing,
    AwaitingApproval,
    Validating,
    Committing,
    Recovering,
    Completed,
    Failed,
}

impl OrchestratorState {
    pub fn accepts_input(&self) -> bool {
        matches!(
            self,
            Self::Idle | Self::AwaitingClarification | Self::AwaitingApproval
        )
    }

    pub fn interruptible(&self) -> bool {
        matches!(self, Self::Executing | Self::Validating)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::RefiningIntent => "refining",
            Self::AwaitingClarification => "awaiting_clarification",
            Self::Executing => "executing",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Validating => "validating",
            Self::Committing => "committing",
            Self::Recovering => "recovering",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}
