use std::path::PathBuf;

/// Events rendered to user (clean, narrated)
#[derive(Debug, Clone)]
pub enum UserFacingEvent {
    /// Status narration - action beginning
    ActionStarted { description: String },

    /// Status narration - action completed successfully
    ActionCompleted { description: String },

    /// Status narration - action failed
    ActionFailed { description: String, error: String },

    /// Progress indication - validation running
    ValidationRunning { stage: Option<String> },

    /// Validation passed successfully
    ValidationPassed,

    /// Validation failed (possibly with revert)
    ValidationFailed { reason: String, reverted: bool },

    /// User approval required before proceeding
    ApprovalRequested { request: super::ApprovalRequest },

    /// Work completed successfully
    WorkCompleted {
        summary: String,
        files_changed: Vec<PathBuf>,
    },

    /// Work failed
    WorkFailed {
        reason: String,
        suggestion: Option<String>,
    },

    /// System needs clarification
    ClarificationQuestion {
        question: String,
        context: Option<String>,
    },

    /// System message (info, warning, error)
    SystemMessage {
        content: String,
        level: MessageLevel,
    },
}

/// Level for system messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl MessageLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageLevel::Info => "info",
            MessageLevel::Success => "success",
            MessageLevel::Warning => "warning",
            MessageLevel::Error => "error",
        }
    }
}

/// Internal orchestrator events (raw, before mapping)
#[derive(Debug, Clone)]
pub enum OrchestratorEvent {
    // Intent phase
    IntentRefinementStarted {
        user_content: String,
    },
    IntentRefinementCompleted {
        intent_spec: super::IntentSpec,
    },
    ClarificationRequested {
        question: String,
    },

    // Execution phase
    ExecutionStarted {
        execution_id: String,
    },
    PlannerInvoked {
        iteration: u32,
    },
    ToolExecuting {
        name: String,
        arguments: ToolArguments,
    },
    ToolCompleted {
        name: String,
        result: ToolResultSummary,
    },

    // Validation phase
    ValidationStarted {
        mutations_count: u32,
    },
    ValidationStageRunning {
        stage: String,
    },
    ValidationStageCompleted {
        stage: String,
        passed: bool,
    },

    // Approval phase
    ApprovalRequired {
        request: super::ApprovalRequest,
    },

    // Completion
    MutationsCommitted {
        files: Vec<PathBuf>,
    },
    ExecutionCompleted {
        success: bool,
        summary: String,
    },
    ExecutionFailed {
        reason: String,
        recoverable: bool,
    },
}

/// Tool arguments summary for display
#[derive(Debug, Clone, Default)]
pub struct ToolArguments {
    pub path: Option<PathBuf>,
    pub file_path: Option<PathBuf>,
    pub raw: std::collections::HashMap<String, String>,
}

/// Tool execution result summary
#[derive(Debug, Clone)]
pub struct ToolResultSummary {
    pub success: bool,
    pub output_preview: Option<String>,
    pub error_preview: Option<String>,
    pub line_count: Option<usize>,
}
