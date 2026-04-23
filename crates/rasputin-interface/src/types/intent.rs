use std::path::PathBuf;

/// Fully-specified intent ready for execution
#[derive(Debug, Clone)]
pub enum IntentSpec {
    /// Direct task with complete context
    Concrete {
        task: String,
        target_files: Vec<PathBuf>,
        constraints: Vec<String>,
        references: Vec<FollowUpReference>,
    },

    /// Needs clarification before execution
    ClarificationNeeded {
        question: String,
        options: Vec<String>,
        ambiguity_type: AmbiguityType,
    },

    /// Failed to resolve reference safely
    ResolutionFailed {
        reference: String,
        reason: ResolutionFailureReason,
    },
}

impl IntentSpec {
    pub fn is_concrete(&self) -> bool {
        matches!(self, Self::Concrete { .. })
    }

    pub fn needs_clarification(&self) -> bool {
        matches!(self, Self::ClarificationNeeded { .. })
    }

    pub fn task_description(&self) -> Option<&str> {
        match self {
            Self::Concrete { task, .. } => Some(task),
            _ => None,
        }
    }
}

/// Reference to prior conversation context
#[derive(Debug, Clone)]
pub enum FollowUpReference {
    TurnRef(u32),
    FileRef(PathBuf),
    ErrorRef { turn_id: u32, error_summary: String },
    IntentRef(u32),
    UncommittedWorkRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbiguityType {
    UnclearTarget,
    MultipleCandidates,
    MissingContext,
    VagueConstraint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionFailureReason {
    NoRecentContext,
    ExpiredReference,
    AmbiguousWithoutClarification,
}
