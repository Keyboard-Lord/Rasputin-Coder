use chrono::{DateTime, Local};
use std::path::PathBuf;

/// Request for user approval before proceeding
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub action_type: ActionType,
    pub description: String,
    pub rationale: String,
    pub affected_paths: Vec<PathBuf>,
    pub can_preview: bool,
    pub expires_at: Option<DateTime<Local>>,
}

impl ApprovalRequest {
    pub fn new(action_type: ActionType, description: impl Into<String>) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            action_type,
            description: description.into(),
            rationale: String::new(),
            affected_paths: Vec::new(),
            can_preview: false,
            expires_at: None,
        }
    }

    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = rationale.into();
        self
    }

    pub fn with_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.affected_paths = paths;
        self
    }

    pub fn with_preview(mut self, can_preview: bool) -> Self {
        self.can_preview = can_preview;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionType {
    FileRead,
    FileWrite,
    FilePatch,
    FileDelete,
    CommandExecute,
    BatchMutation,
    DestructiveTool,
}

impl ActionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionType::FileRead => "read",
            ActionType::FileWrite => "write",
            ActionType::FilePatch => "patch",
            ActionType::FileDelete => "delete",
            ActionType::CommandExecute => "command",
            ActionType::BatchMutation => "batch",
            ActionType::DestructiveTool => "destructive",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ActionType::FileRead => "Read file",
            ActionType::FileWrite => "Write file",
            ActionType::FilePatch => "Apply changes",
            ActionType::FileDelete => "Delete file",
            ActionType::CommandExecute => "Execute command",
            ActionType::BatchMutation => "Multiple file changes",
            ActionType::DestructiveTool => "Destructive operation",
        }
    }
}

/// User's response to approval request
#[derive(Debug, Clone)]
pub enum ApprovalResponse {
    Grant {
        request_id: String,
        grant_duration: GrantDuration,
    },
    Deny {
        request_id: String,
        reason: Option<String>,
    },
    Preview {
        request_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantDuration {
    Once,
    Turn,
    Session,
}

impl GrantDuration {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrantDuration::Once => "once",
            GrantDuration::Turn => "turn",
            GrantDuration::Session => "session",
        }
    }
}
