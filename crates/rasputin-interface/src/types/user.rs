use super::events::UserFacingEvent;
use super::IntentSpec;
use chrono::{DateTime, Local};

/// A message from the user
#[derive(Debug, Clone)]
pub struct UserMessage {
    pub id: String,
    pub content: String,
    pub timestamp: DateTime<Local>,
    pub mode: InteractionMode,
}

impl UserMessage {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.into(),
            timestamp: Local::now(),
            mode: InteractionMode::Chat,
        }
    }

    pub fn with_mode(mut self, mode: InteractionMode) -> Self {
        self.mode = mode;
        self
    }
}

/// One turn in the conversation
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub turn_id: u32,
    pub user_message: UserMessage,
    pub intent_spec: Option<IntentSpec>,
    pub execution_id: Option<String>,
    pub response: Vec<UserFacingEvent>,
    pub completed_at: Option<DateTime<Local>>,
}

impl ConversationTurn {
    pub fn new(turn_id: u32, user_message: UserMessage) -> Self {
        Self {
            turn_id,
            user_message,
            intent_spec: None,
            execution_id: None,
            response: Vec::new(),
            completed_at: None,
        }
    }

    pub fn add_event(&mut self, event: UserFacingEvent) {
        self.response.push(event);
    }

    pub fn mark_completed(&mut self) {
        self.completed_at = Some(Local::now());
    }
}

/// How the user is interacting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    Chat,
    Command,
    ApprovalGrant,
    ApprovalDeny,
    Clarification,
}

impl InteractionMode {
    pub fn is_approval_response(&self) -> bool {
        matches!(self, Self::ApprovalGrant | Self::ApprovalDeny)
    }
}
