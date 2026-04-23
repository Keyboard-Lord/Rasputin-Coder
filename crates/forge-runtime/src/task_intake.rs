//! Structured Task Intake - Bounded Task Interpretation
//!
//! Converts freeform tasks into explicit, inspectable execution objectives.
//! Per SPRINT: Git Grounding + Approval Checkpoints + Structured Task Intake

use serde::{Deserialize, Serialize};

/// Interpreted task from freeform input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredTaskIntake {
    /// Original request from user
    pub original_request: String,
    /// Interpreted objective
    pub interpreted_objective: String,
    /// Classified task type
    pub task_class: TaskClass,
    /// Risk assessment
    pub risk_level: TaskRiskLevel,
    /// Likely file targets
    pub likely_targets: Vec<String>,
    /// Whether clarification is required
    pub requires_clarification: bool,
    /// Questions to ask user (if clarification needed)
    pub clarification_questions: Vec<String>,
    /// Proposed execution mode
    pub proposed_execution_mode: ExecutionMode,
}

/// Classification of task type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskClass {
    /// Read-only analysis and exploration
    ReadOnlyAnalysis,
    /// Edit a single file
    SingleFileEdit,
    /// Edit multiple files
    MultiFileEdit,
    /// Refactoring across files
    Refactor,
    /// Validation/testing only
    ValidationOnly,
    /// Fix a specific error
    DebugFix,
    /// Cannot determine intent
    Unknown,
}

impl std::fmt::Display for TaskClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnlyAnalysis => write!(f, "read-only analysis"),
            Self::SingleFileEdit => write!(f, "single-file edit"),
            Self::MultiFileEdit => write!(f, "multi-file edit"),
            Self::Refactor => write!(f, "refactoring"),
            Self::ValidationOnly => write!(f, "validation only"),
            Self::DebugFix => write!(f, "debug fix"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Risk level assessment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TaskRiskLevel {
    /// Low risk - read operations only
    Low,
    /// Medium risk - limited edits
    Medium,
    /// High risk - broad changes
    High,
}

impl std::fmt::Display for TaskRiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
        }
    }
}

/// Execution mode recommendation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Analysis mode - no mutations
    Analysis,
    /// Edit mode - mutations allowed
    Edit,
    /// Requires explicit approval
    ApprovalRequired,
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Analysis => write!(f, "analysis"),
            Self::Edit => write!(f, "edit"),
            Self::ApprovalRequired => write!(f, "approval-required"),
        }
    }
}

impl StructuredTaskIntake {
    /// Create a new intake result
    pub fn new(original: impl Into<String>) -> Self {
        Self {
            original_request: original.into(),
            interpreted_objective: String::new(),
            task_class: TaskClass::Unknown,
            risk_level: TaskRiskLevel::Low,
            likely_targets: vec![],
            requires_clarification: false,
            clarification_questions: vec![],
            proposed_execution_mode: ExecutionMode::Analysis,
        }
    }

    /// Set the interpreted objective
    pub fn with_objective(mut self, objective: impl Into<String>) -> Self {
        self.interpreted_objective = objective.into();
        self
    }

    /// Set task class
    pub fn with_class(mut self, class: TaskClass) -> Self {
        self.task_class = class;
        self
    }

    /// Set risk level
    pub fn with_risk(mut self, risk: TaskRiskLevel) -> Self {
        self.risk_level = risk;
        self
    }

    /// Add likely target
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.likely_targets.push(target.into());
        self
    }

    /// Require clarification with question
    pub fn require_clarification(mut self, question: impl Into<String>) -> Self {
        self.requires_clarification = true;
        self.clarification_questions.push(question.into());
        self
    }

    /// Set execution mode
    pub fn with_mode(mut self, mode: ExecutionMode) -> Self {
        self.proposed_execution_mode = mode;
        self
    }

    /// Check if this is a read-only task
    pub fn is_read_only(&self) -> bool {
        matches!(
            self.task_class,
            TaskClass::ReadOnlyAnalysis | TaskClass::ValidationOnly
        )
    }

    /// Check if this task edits files
    pub fn is_edit_task(&self) -> bool {
        matches!(
            self.task_class,
            TaskClass::SingleFileEdit
                | TaskClass::MultiFileEdit
                | TaskClass::Refactor
                | TaskClass::DebugFix
        )
    }

    /// Get a summary string
    pub fn summary(&self) -> String {
        if self.requires_clarification {
            format!(
                "{} task ({} risk) - requires clarification",
                self.task_class, self.risk_level
            )
        } else {
            format!(
                "{} task ({} risk) - {}",
                self.task_class, self.risk_level, self.proposed_execution_mode
            )
        }
    }
}

/// Task intake classifier
pub struct TaskIntakeClassifier;

impl TaskIntakeClassifier {
    /// Classify a freeform task request
    pub fn classify(request: &str) -> StructuredTaskIntake {
        let request_lower = request.to_lowercase();
        let mut intake = StructuredTaskIntake::new(request);

        // Detect read-only patterns
        if Self::is_read_only(&request_lower) {
            return intake
                .with_objective("Analyze and explore codebase")
                .with_class(TaskClass::ReadOnlyAnalysis)
                .with_risk(TaskRiskLevel::Low)
                .with_mode(ExecutionMode::Analysis);
        }

        // Detect single file edit patterns (only if not yet classified)
        if intake.task_class == TaskClass::Unknown && Self::is_single_file_edit(&request_lower) {
            intake = intake
                .with_objective("Edit a single file")
                .with_class(TaskClass::SingleFileEdit)
                .with_risk(TaskRiskLevel::Medium)
                .with_mode(ExecutionMode::Edit);
        }

        // Detect multi-file/refactor patterns (only if not yet classified)
        if intake.task_class == TaskClass::Unknown && Self::is_multi_file_edit(&request_lower) {
            intake = intake
                .with_objective("Edit multiple files")
                .with_class(TaskClass::MultiFileEdit)
                .with_risk(TaskRiskLevel::High)
                .with_mode(ExecutionMode::ApprovalRequired);
        }

        // Detect refactor patterns (only if not yet classified)
        if intake.task_class == TaskClass::Unknown && Self::is_refactor(&request_lower) {
            intake = intake
                .with_objective("Refactor code across files")
                .with_class(TaskClass::Refactor)
                .with_risk(TaskRiskLevel::High)
                .with_mode(ExecutionMode::ApprovalRequired);
        }

        // Detect fix/debug patterns (only if not yet classified)
        if intake.task_class == TaskClass::Unknown && Self::is_debug_fix(&request_lower) {
            intake = intake
                .with_objective("Fix a specific error or bug")
                .with_class(TaskClass::DebugFix)
                .with_risk(TaskRiskLevel::Medium)
                .with_mode(ExecutionMode::Edit);
        }

        // Detect validation patterns
        if Self::is_validation(&request_lower) {
            return intake
                .with_objective("Run validation and tests")
                .with_class(TaskClass::ValidationOnly)
                .with_risk(TaskRiskLevel::Low)
                .with_mode(ExecutionMode::Analysis);
        }

        // Ambiguous or unknown - require clarification
        if intake.task_class == TaskClass::Unknown {
            intake = intake
                .require_clarification("Please specify if this is analysis, single-file edit, multi-file edit, or refactoring")
                .with_risk(TaskRiskLevel::High)
                .with_mode(ExecutionMode::ApprovalRequired);
        }

        intake
    }

    fn is_read_only(request: &str) -> bool {
        let patterns = [
            "explain", "what is", "how does", "show me", "find", "search", "analyze", "review",
            "look at", "read", "list", "describe",
        ];
        patterns.iter().any(|p| request.contains(p))
    }

    fn is_single_file_edit(request: &str) -> bool {
        let patterns = [
            "fix the file",
            "update the file",
            "change the file",
            "edit the file",
            "modify the file",
            "in file",
        ];
        patterns.iter().any(|p| request.contains(p))
    }

    fn is_multi_file_edit(request: &str) -> bool {
        let patterns = [
            "files",
            "multiple files",
            "all files",
            "update all",
            "change all",
            "rename across",
            "update across",
        ];
        patterns.iter().any(|p| request.contains(p))
    }

    fn is_refactor(request: &str) -> bool {
        let patterns = [
            "refactor",
            "restructure",
            "reorganize",
            "split into",
            "extract",
            "move to",
            "consolidate",
            "clean up",
        ];
        patterns.iter().any(|p| request.contains(p))
    }

    fn is_debug_fix(request: &str) -> bool {
        let patterns = [
            "fix",
            "bug",
            "error",
            "crash",
            "broken",
            "not working",
            "fails",
            "failure",
            "issue",
            "problem",
        ];
        patterns.iter().any(|p| request.contains(p))
    }

    fn is_validation(request: &str) -> bool {
        let patterns = [
            "test",
            "validate",
            "check",
            "verify",
            "lint",
            "run tests",
            "check build",
            "validate code",
        ];
        patterns.iter().any(|p| request.contains(p))
    }
}

/// Execution risk policy
#[derive(Debug, Clone)]
pub struct ExecutionRiskPolicy {
    /// Require approval on dirty worktree
    pub require_approval_on_dirty_worktree: bool,
    /// Require approval on high-risk task
    pub require_approval_on_high_risk_task: bool,
    /// Require approval on multi-file edits
    pub require_approval_on_multi_file_edits: bool,
    /// Require approval on replay mismatch
    pub require_approval_on_replay_mismatch: bool,
    /// Warn on untracked target files
    pub warn_on_untracked_targets: bool,
}

impl Default for ExecutionRiskPolicy {
    fn default() -> Self {
        Self {
            require_approval_on_dirty_worktree: false,
            require_approval_on_high_risk_task: true,
            require_approval_on_multi_file_edits: true,
            require_approval_on_replay_mismatch: true,
            warn_on_untracked_targets: true,
        }
    }
}

impl ExecutionRiskPolicy {
    /// Check if approval is required for this intake
    pub fn requires_approval_for_intake(&self, intake: &StructuredTaskIntake) -> bool {
        if self.require_approval_on_high_risk_task && intake.risk_level == TaskRiskLevel::High {
            return true;
        }
        if self.require_approval_on_multi_file_edits {
            match intake.task_class {
                TaskClass::MultiFileEdit | TaskClass::Refactor => return true,
                _ => {}
            }
        }
        intake.requires_clarification
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_read_only() {
        let intake = TaskIntakeClassifier::classify("explain how the code works");
        assert!(matches!(intake.task_class, TaskClass::ReadOnlyAnalysis));
        assert!(matches!(intake.risk_level, TaskRiskLevel::Low));
        assert!(intake.is_read_only());
    }

    #[test]
    fn classify_single_file_edit() {
        let intake = TaskIntakeClassifier::classify("fix the bug in src/main.rs");
        assert!(matches!(
            intake.task_class,
            TaskClass::SingleFileEdit | TaskClass::DebugFix
        ));
    }

    #[test]
    fn classify_multi_file() {
        let intake = TaskIntakeClassifier::classify("update all files to use new api");
        assert!(matches!(intake.task_class, TaskClass::MultiFileEdit));
        assert!(matches!(intake.risk_level, TaskRiskLevel::High));
        assert!(matches!(
            intake.proposed_execution_mode,
            ExecutionMode::ApprovalRequired
        ));
    }

    #[test]
    fn classify_refactor() {
        let intake = TaskIntakeClassifier::classify("refactor the error handling");
        assert!(matches!(intake.task_class, TaskClass::Refactor));
        assert!(matches!(intake.risk_level, TaskRiskLevel::High));
    }

    #[test]
    fn classify_validation() {
        let intake = TaskIntakeClassifier::classify("run tests and validate");
        assert!(matches!(intake.task_class, TaskClass::ValidationOnly));
        assert!(intake.is_read_only());
    }

    #[test]
    fn classify_ambiguous() {
        let intake = TaskIntakeClassifier::classify("do something");
        assert!(intake.requires_clarification);
        assert!(!intake.clarification_questions.is_empty());
    }

    #[test]
    fn risk_policy_defaults() {
        let policy = ExecutionRiskPolicy::default();
        assert!(policy.require_approval_on_high_risk_task);
        assert!(policy.require_approval_on_multi_file_edits);
        assert!(policy.require_approval_on_replay_mismatch);
    }

    #[test]
    fn risk_policy_high_risk_trigger() {
        let policy = ExecutionRiskPolicy::default();
        let high_risk = StructuredTaskIntake::new("test")
            .with_class(TaskClass::MultiFileEdit)
            .with_risk(TaskRiskLevel::High);

        assert!(policy.requires_approval_for_intake(&high_risk));
    }

    #[test]
    fn intake_is_edit_task() {
        let edit = StructuredTaskIntake::new("test").with_class(TaskClass::SingleFileEdit);
        assert!(edit.is_edit_task());

        let read = StructuredTaskIntake::new("test").with_class(TaskClass::ReadOnlyAnalysis);
        assert!(!read.is_edit_task());
    }
}
