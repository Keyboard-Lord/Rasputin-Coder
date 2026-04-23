//! Git Grounding - Repository State Awareness
//!
//! Captures Git repository state for safer, more grounded task execution.
//! Per SPRINT: Git Grounding + Approval Checkpoints + Structured Task Intake

use serde::{Deserialize, Serialize};

/// Complete Git repository grounding snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitGrounding {
    /// Whether a Git repo was detected
    pub repo_detected: bool,
    /// Current branch name (if any)
    pub branch_name: Option<String>,
    /// HEAD commit hash (short form)
    pub head_commit: Option<String>,
    /// Whether working tree is dirty
    pub is_dirty: bool,
    /// Modified files (unstaged changes)
    pub modified_files: Vec<GitFileStatus>,
    /// Staged files (added to index)
    pub staged_files: Vec<GitFileStatus>,
    /// Untracked files
    pub untracked_files: Vec<GitFileStatus>,
    /// Recent commits for context (bounded)
    pub recent_commits: Vec<GitCommitSummary>,
}

impl GitGrounding {
    /// Create empty grounding (no repo detected)
    pub fn no_repo() -> Self {
        Self {
            repo_detected: false,
            branch_name: None,
            head_commit: None,
            is_dirty: false,
            modified_files: vec![],
            staged_files: vec![],
            untracked_files: vec![],
            recent_commits: vec![],
        }
    }

    /// Create grounding from detected repo state
    pub fn from_repo(
        branch: Option<String>,
        head: Option<String>,
        dirty: bool,
        modified: Vec<GitFileStatus>,
        staged: Vec<GitFileStatus>,
        untracked: Vec<GitFileStatus>,
        commits: Vec<GitCommitSummary>,
    ) -> Self {
        // Apply bounds
        let modified = modified.into_iter().take(100).collect();
        let staged = staged.into_iter().take(100).collect();
        let untracked = untracked.into_iter().take(100).collect();
        let commits = commits.into_iter().take(10).collect();

        Self {
            repo_detected: true,
            branch_name: branch,
            head_commit: head,
            is_dirty: dirty,
            modified_files: modified,
            staged_files: staged,
            untracked_files: untracked,
            recent_commits: commits,
        }
    }

    /// Check if worktree is clean (no modifications, staged, or untracked)
    pub fn is_clean(&self) -> bool {
        !self.is_dirty
            && self.modified_files.is_empty()
            && self.staged_files.is_empty()
            && self.untracked_files.is_empty()
    }

    /// Get total file changes count
    pub fn total_changes(&self) -> usize {
        self.modified_files.len() + self.staged_files.len() + self.untracked_files.len()
    }

    /// Get a one-line summary of repo state
    pub fn summary(&self) -> String {
        if !self.repo_detected {
            return "No Git repository detected".to_string();
        }

        let branch = self.branch_name.as_deref().unwrap_or("(detached)");
        let dirty_marker = if self.is_dirty { "*" } else { "" };
        let changes = self.total_changes();

        if changes > 0 {
            format!(
                "{}: {}{} ({} files changed)",
                &self.head_commit.as_deref().unwrap_or("unknown")
                    [..7.min(self.head_commit.as_ref().map_or(7, |s| s.len()))],
                branch,
                dirty_marker,
                changes
            )
        } else {
            format!(
                "{}: {}{} (clean)",
                &self.head_commit.as_deref().unwrap_or("unknown")
                    [..7.min(self.head_commit.as_ref().map_or(7, |s| s.len()))],
                branch,
                dirty_marker
            )
        }
    }
}

/// Status of a single file in the Git working tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileStatus {
    /// File path relative to repo root
    pub path: String,
    /// Git status code (M, A, D, ??, etc.)
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

/// Summary of a recent Git commit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitSummary {
    /// Short commit hash (7 chars)
    pub short_hash: String,
    /// Commit subject line
    pub subject: String,
    /// Commit author (optional)
    pub author: Option<String>,
}

impl GitCommitSummary {
    pub fn new(hash: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            short_hash: hash.into(),
            subject: subject.into(),
            author: None,
        }
    }

    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
}

/// Result of task start Git checks
#[derive(Debug, Clone)]
pub struct TaskStartCheckResult {
    /// Whether execution is allowed to proceed
    pub allowed: bool,
    /// Warnings to display to operator
    pub warnings: Vec<TaskStartWarning>,
    /// Whether explicit approval is required
    pub requires_approval: bool,
    /// Human-readable summary
    pub summary: String,
}

impl TaskStartCheckResult {
    /// Allow execution with optional warnings
    pub fn allow(warnings: Vec<TaskStartWarning>, summary: impl Into<String>) -> Self {
        Self {
            allowed: true,
            warnings,
            requires_approval: false,
            summary: summary.into(),
        }
    }

    /// Require approval before proceeding
    pub fn require_approval(warnings: Vec<TaskStartWarning>, summary: impl Into<String>) -> Self {
        Self {
            allowed: false, // Not allowed without explicit approval
            warnings,
            requires_approval: true,
            summary: summary.into(),
        }
    }

    /// Block execution entirely
    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            warnings: vec![TaskStartWarning::error(reason)],
            requires_approval: false,
            summary: "Execution blocked".to_string(),
        }
    }

    /// Check if there are any warnings
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// Single warning for task start
#[derive(Debug, Clone)]
pub struct TaskStartWarning {
    /// Warning class/type
    pub class: String,
    /// Warning message
    pub message: String,
    /// Severity level
    pub severity: WarningSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningSeverity {
    Info,
    Warning,
    Error,
}

impl TaskStartWarning {
    pub fn new(class: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            message: message.into(),
            severity: WarningSeverity::Warning,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            class: "error".to_string(),
            message: message.into(),
            severity: WarningSeverity::Error,
        }
    }

    pub fn info(class: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            message: message.into(),
            severity: WarningSeverity::Info,
        }
    }
}

/// Performs task start checks against Git grounding
pub struct TaskStartChecker;

impl TaskStartChecker {
    /// Check if task can start given current Git state
    pub fn check(grounding: &GitGrounding, policy: &TaskStartPolicy) -> TaskStartCheckResult {
        let mut warnings = Vec::new();
        let mut requires_approval = false;

        if policy.git_policy == GitPolicy::Disabled {
            return TaskStartCheckResult::allow(warnings, "Git checks disabled");
        }

        // Check for no repo
        if !grounding.repo_detected {
            warnings.push(TaskStartWarning::new(
                "no_git_repo",
                "No Git repository detected - changes will not be tracked",
            ));
        }

        // Check for detached HEAD
        if grounding.repo_detected && grounding.branch_name.is_none() {
            warnings.push(TaskStartWarning::new(
                "detached_head",
                "Detached HEAD state - changes may be orphaned",
            ));
            if policy.git_policy == GitPolicy::Strict && policy.require_approval_on_detached_head {
                requires_approval = true;
            }
        }

        // Check for dirty worktree
        if grounding.is_dirty {
            warnings.push(TaskStartWarning::new(
                "dirty_worktree",
                format!(
                    "Dirty worktree with {} modified files",
                    grounding.modified_files.len()
                ),
            ));
            if policy.git_policy == GitPolicy::Strict && policy.require_approval_on_dirty_worktree {
                requires_approval = true;
            }
        }

        // Check for staged changes
        if !grounding.staged_files.is_empty() {
            warnings.push(TaskStartWarning::new(
                "staged_changes",
                format!(
                    "{} staged but uncommitted changes",
                    grounding.staged_files.len()
                ),
            ));
        }

        // Check for untracked files
        if !grounding.untracked_files.is_empty() {
            warnings.push(TaskStartWarning::info(
                "untracked_files",
                format!(
                    "{} untracked files present",
                    grounding.untracked_files.len()
                ),
            ));
        }

        let summary = if warnings.is_empty() {
            "Clean repository state - ready to proceed".to_string()
        } else {
            format!("{} warnings - review before proceeding", warnings.len())
        };

        if requires_approval {
            TaskStartCheckResult::require_approval(warnings, summary)
        } else {
            TaskStartCheckResult::allow(warnings, summary)
        }
    }
}

/// Policy for task start checks
#[derive(Debug, Clone)]
pub struct TaskStartPolicy {
    /// Git safety behavior for task starts
    pub git_policy: GitPolicy,
    /// Require approval if worktree is dirty
    pub require_approval_on_dirty_worktree: bool,
    /// Require approval on detached HEAD
    pub require_approval_on_detached_head: bool,
    /// Warn on staged but uncommitted changes
    pub warn_on_staged_changes: bool,
    /// Warn on untracked files in target paths
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
    /// Strict policy - requires approval on any risk
    pub fn strict() -> Self {
        Self {
            git_policy: GitPolicy::Strict,
            require_approval_on_dirty_worktree: true,
            require_approval_on_detached_head: true,
            warn_on_staged_changes: true,
            warn_on_untracked_targets: true,
        }
    }

    /// Permissive policy - warns only
    pub fn permissive() -> Self {
        Self {
            git_policy: GitPolicy::Advisory,
            require_approval_on_dirty_worktree: false,
            require_approval_on_detached_head: false,
            warn_on_staged_changes: true,
            warn_on_untracked_targets: false,
        }
    }

    /// Advisory policy - warns only
    pub fn advisory() -> Self {
        Self::default()
    }

    /// Disabled policy - skips Git state checks
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_grounding_no_repo() {
        let grounding = GitGrounding::no_repo();
        assert!(!grounding.repo_detected);
        assert!(grounding.branch_name.is_none());
        assert!(grounding.is_clean());
    }

    #[test]
    fn git_grounding_clean_repo() {
        let grounding = GitGrounding::from_repo(
            Some("main".to_string()),
            Some("abc1234".to_string()),
            false,
            vec![],
            vec![],
            vec![],
            vec![GitCommitSummary::new("abc1234", "Initial commit")],
        );
        assert!(grounding.repo_detected);
        assert!(grounding.is_clean());
        assert_eq!(grounding.branch_name, Some("main".to_string()));
    }

    #[test]
    fn git_grounding_bounds_applied() {
        let modified: Vec<_> = (0..150)
            .map(|i| GitFileStatus::new(format!("file{}.rs", i), "M"))
            .collect();
        let grounding = GitGrounding::from_repo(
            Some("main".to_string()),
            Some("abc1234".to_string()),
            true,
            modified,
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(grounding.modified_files.len(), 100); // Bounded to 100
    }

    #[test]
    fn git_grounding_summary() {
        let grounding = GitGrounding::from_repo(
            Some("main".to_string()),
            Some("abc1234".to_string()),
            false,
            vec![],
            vec![],
            vec![],
            vec![],
        );
        assert!(grounding.summary().contains("abc1234"));
        assert!(grounding.summary().contains("main"));
        assert!(grounding.summary().contains("clean"));
    }

    #[test]
    fn task_start_check_clean_repo() {
        let grounding = GitGrounding::from_repo(
            Some("main".to_string()),
            Some("abc1234".to_string()),
            false,
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let policy = TaskStartPolicy::default();
        let result = TaskStartChecker::check(&grounding, &policy);

        assert!(result.allowed);
        assert!(!result.requires_approval);
        assert!(!result.has_warnings());
    }

    #[test]
    fn task_start_check_no_repo_is_advisory_by_default() {
        let grounding = GitGrounding::no_repo();
        let policy = TaskStartPolicy::default();
        let result = TaskStartChecker::check(&grounding, &policy);

        assert!(result.allowed);
        assert!(!result.requires_approval);
        assert!(result.has_warnings());
        assert_eq!(result.warnings[0].class, "no_git_repo");
    }

    #[test]
    fn task_start_check_disabled_git_policy_suppresses_no_repo_warning() {
        let grounding = GitGrounding::no_repo();
        let policy = TaskStartPolicy::disabled();
        let result = TaskStartChecker::check(&grounding, &policy);

        assert!(result.allowed);
        assert!(!result.requires_approval);
        assert!(!result.has_warnings());
    }

    #[test]
    fn task_start_check_dirty_worktree() {
        let grounding = GitGrounding::from_repo(
            Some("main".to_string()),
            Some("abc1234".to_string()),
            true,
            vec![GitFileStatus::new("src/main.rs", "M")],
            vec![],
            vec![],
            vec![],
        );
        let policy = TaskStartPolicy::strict();
        let result = TaskStartChecker::check(&grounding, &policy);

        assert!(!result.allowed);
        assert!(result.requires_approval);
        assert!(result.has_warnings());
    }

    #[test]
    fn task_start_check_detached_head() {
        let grounding = GitGrounding::from_repo(
            None, // detached
            Some("abc1234".to_string()),
            false,
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let policy = TaskStartPolicy::strict();
        let result = TaskStartChecker::check(&grounding, &policy);

        assert!(!result.allowed);
        assert!(result.requires_approval);
    }

    #[test]
    fn task_start_check_permissive() {
        let grounding = GitGrounding::from_repo(
            Some("main".to_string()),
            Some("abc1234".to_string()),
            true,
            vec![GitFileStatus::new("src/main.rs", "M")],
            vec![],
            vec![],
            vec![],
        );
        let policy = TaskStartPolicy::permissive();
        let result = TaskStartChecker::check(&grounding, &policy);

        assert!(result.allowed);
        assert!(!result.requires_approval);
        assert!(result.has_warnings()); // Still warns
    }
}
