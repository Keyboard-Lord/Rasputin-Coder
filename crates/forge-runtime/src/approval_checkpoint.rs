//! Approval Checkpoints - Bounded Human Review Points
//!
//! Allows optional human approval at defined execution boundaries.
//! Per SPRINT: Git Grounding + Approval Checkpoints + Structured Task Intake

use serde::{Deserialize, Serialize};

/// Checkpoint requiring explicit operator approval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalCheckpoint {
    /// Step index where checkpoint is placed
    pub step_index: u32,
    /// Human-readable reason for checkpoint
    pub reason: String,
    /// Type of checkpoint
    pub checkpoint_type: ApprovalCheckpointType,
    /// Current state
    pub state: ApprovalCheckpointState,
    /// When checkpoint was created (unix millis)
    pub created_at: u64,
    /// When checkpoint was resolved (if approved/denied)
    pub resolved_at: Option<u64>,
    /// Who resolved the checkpoint (if applicable)
    pub resolved_by: Option<String>,
}

/// Type of approval checkpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalCheckpointType {
    /// Before execution begins
    PreExecution,
    /// Before committing mutations
    PreMutationCommit,
    /// After validation, before advancing
    PostValidationPreAdvance,
    /// When replay mismatch detected
    ReplayMismatchReview,
}

impl std::fmt::Display for ApprovalCheckpointType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreExecution => write!(f, "pre-execution"),
            Self::PreMutationCommit => write!(f, "pre-mutation-commit"),
            Self::PostValidationPreAdvance => write!(f, "post-validation"),
            Self::ReplayMismatchReview => write!(f, "replay-mismatch-review"),
        }
    }
}

/// State of an approval checkpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalCheckpointState {
    /// Awaiting operator decision
    Pending,
    /// Operator approved
    Approved,
    /// Operator denied
    Denied,
    /// Checkpoint expired (if applicable)
    Expired,
}

impl ApprovalCheckpoint {
    /// Create a new pending checkpoint
    pub fn new(
        step_index: u32,
        checkpoint_type: ApprovalCheckpointType,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            step_index,
            reason: reason.into(),
            checkpoint_type,
            state: ApprovalCheckpointState::Pending,
            created_at: crate::types::timestamp_now(),
            resolved_at: None,
            resolved_by: None,
        }
    }

    /// Create a pre-execution checkpoint
    pub fn pre_execution(reason: impl Into<String>) -> Self {
        Self::new(0, ApprovalCheckpointType::PreExecution, reason)
    }

    /// Create a pre-mutation checkpoint
    pub fn pre_mutation(step_index: u32, reason: impl Into<String>) -> Self {
        Self::new(
            step_index,
            ApprovalCheckpointType::PreMutationCommit,
            reason,
        )
    }

    /// Create a replay mismatch review checkpoint
    pub fn replay_mismatch(reason: impl Into<String>) -> Self {
        Self::new(0, ApprovalCheckpointType::ReplayMismatchReview, reason)
    }

    /// Approve the checkpoint
    pub fn approve(&mut self, operator: impl Into<String>) {
        self.state = ApprovalCheckpointState::Approved;
        self.resolved_at = Some(crate::types::timestamp_now());
        self.resolved_by = Some(operator.into());
    }

    /// Deny the checkpoint
    pub fn deny(&mut self, operator: impl Into<String>) {
        self.state = ApprovalCheckpointState::Denied;
        self.resolved_at = Some(crate::types::timestamp_now());
        self.resolved_by = Some(operator.into());
    }

    /// Check if checkpoint is pending
    pub fn is_pending(&self) -> bool {
        self.state == ApprovalCheckpointState::Pending
    }

    /// Check if checkpoint is approved
    pub fn is_approved(&self) -> bool {
        self.state == ApprovalCheckpointState::Approved
    }

    /// Check if checkpoint was denied
    pub fn is_denied(&self) -> bool {
        self.state == ApprovalCheckpointState::Denied
    }

    /// Get resolution time if resolved
    pub fn resolution_time_ms(&self) -> Option<u64> {
        match (self.resolved_at, self.created_at) {
            (Some(resolved), created) => Some(resolved - created),
            _ => None,
        }
    }
}

/// Manager for multiple checkpoints in a run
#[derive(Debug, Clone, Default)]
pub struct CheckpointManager {
    checkpoints: Vec<ApprovalCheckpoint>,
    pending_index: Option<usize>,
}

impl CheckpointManager {
    /// Create new empty manager
    pub fn new() -> Self {
        Self {
            checkpoints: vec![],
            pending_index: None,
        }
    }

    /// Add a checkpoint
    pub fn add(&mut self, checkpoint: ApprovalCheckpoint) {
        if checkpoint.is_pending() && self.pending_index.is_none() {
            self.pending_index = Some(self.checkpoints.len());
        }
        self.checkpoints.push(checkpoint);
    }

    /// Get the currently pending checkpoint (if any)
    pub fn pending(&self) -> Option<&ApprovalCheckpoint> {
        self.pending_index.and_then(|i| self.checkpoints.get(i))
    }

    /// Get mutable reference to pending checkpoint
    pub fn pending_mut(&mut self) -> Option<&mut ApprovalCheckpoint> {
        self.pending_index.and_then(|i| self.checkpoints.get_mut(i))
    }

    /// Approve the current pending checkpoint
    pub fn approve_pending(&mut self, operator: impl Into<String>) -> Option<&ApprovalCheckpoint> {
        if let Some(idx) = self.pending_index {
            // First approve the checkpoint
            if let Some(cp) = self.checkpoints.get_mut(idx) {
                cp.approve(operator);
            } else {
                return None;
            }
            // Then find next pending (separate mutable borrow)
            self.pending_index = self.find_next_pending();
            // Return reference to approved checkpoint
            return self.checkpoints.get(idx);
        }
        None
    }

    /// Deny the current pending checkpoint
    pub fn deny_pending(&mut self, operator: impl Into<String>) -> Option<&ApprovalCheckpoint> {
        if let Some(idx) = self.pending_index {
            if let Some(cp) = self.checkpoints.get_mut(idx) {
                cp.deny(operator);
                self.pending_index = None; // Denial stops execution
                return Some(cp);
            }
        }
        None
    }

    /// Check if any checkpoint is currently pending
    pub fn has_pending(&self) -> bool {
        self.pending_index.is_some()
    }

    /// Check if execution was denied (fail-closed)
    pub fn was_denied(&self) -> bool {
        self.checkpoints.iter().any(|cp| cp.is_denied())
    }

    /// Get all checkpoints
    pub fn all(&self) -> &[ApprovalCheckpoint] {
        &self.checkpoints
    }

    /// Count checkpoints by state
    pub fn count_by_state(&self, state: ApprovalCheckpointState) -> usize {
        self.checkpoints
            .iter()
            .filter(|cp| cp.state == state)
            .count()
    }

    /// Find next pending checkpoint
    fn find_next_pending(&self) -> Option<usize> {
        self.checkpoints
            .iter()
            .enumerate()
            .find(|(_, cp)| cp.is_pending())
            .map(|(i, _)| i)
    }
}

/// Actions available at a checkpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointAction {
    Approve,
    Deny,
    Status,
}

/// Command result for checkpoint operations
#[derive(Debug, Clone)]
pub struct CheckpointCommandResult {
    pub success: bool,
    pub message: String,
    pub checkpoint: Option<ApprovalCheckpoint>,
}

impl CheckpointCommandResult {
    pub fn success(message: impl Into<String>, checkpoint: Option<ApprovalCheckpoint>) -> Self {
        Self {
            success: true,
            message: message.into(),
            checkpoint,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            checkpoint: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_creation() {
        let cp = ApprovalCheckpoint::pre_execution("High-risk task requires approval");
        assert_eq!(cp.step_index, 0);
        assert_eq!(cp.checkpoint_type, ApprovalCheckpointType::PreExecution);
        assert!(cp.is_pending());
        assert!(!cp.is_approved());
        assert!(!cp.is_denied());
    }

    #[test]
    fn checkpoint_approve() {
        let mut cp = ApprovalCheckpoint::pre_mutation(2, "Multi-file edit");
        cp.approve("operator");
        assert!(cp.is_approved());
        assert!(!cp.is_pending());
        assert_eq!(cp.resolved_by, Some("operator".to_string()));
        assert!(cp.resolved_at.is_some());
    }

    #[test]
    fn checkpoint_deny() {
        let mut cp = ApprovalCheckpoint::replay_mismatch("Replay diverged");
        cp.deny("operator");
        assert!(cp.is_denied());
        assert!(!cp.is_pending());
        assert!(!cp.is_approved());
    }

    #[test]
    fn checkpoint_manager_add_and_pending() {
        let mut manager = CheckpointManager::new();
        assert!(!manager.has_pending());

        manager.add(ApprovalCheckpoint::pre_execution("Start"));
        assert!(manager.has_pending());
        assert!(manager.pending().is_some());
    }

    #[test]
    fn checkpoint_manager_approve() {
        let mut manager = CheckpointManager::new();
        manager.add(ApprovalCheckpoint::pre_execution("Start"));

        let result = manager.approve_pending("operator");
        assert!(result.is_some());
        assert!(result.unwrap().is_approved());
        assert!(!manager.has_pending());
    }

    #[test]
    fn checkpoint_manager_deny() {
        let mut manager = CheckpointManager::new();
        manager.add(ApprovalCheckpoint::pre_execution("Start"));

        let result = manager.deny_pending("operator");
        assert!(result.is_some());
        assert!(manager.was_denied());
        assert!(!manager.has_pending());
    }

    #[test]
    fn checkpoint_manager_multiple_checkpoints() {
        let mut manager = CheckpointManager::new();
        manager.add(ApprovalCheckpoint::pre_execution("Start"));
        manager.add(ApprovalCheckpoint::pre_mutation(1, "Step 1"));
        manager.add(ApprovalCheckpoint::pre_mutation(2, "Step 2"));

        // First checkpoint is pending
        assert!(manager.pending().is_some());
        assert_eq!(
            manager.pending().unwrap().checkpoint_type,
            ApprovalCheckpointType::PreExecution
        );

        // Approve first, next becomes pending
        manager.approve_pending("operator");
        assert!(manager.has_pending());
        assert_eq!(
            manager.pending().unwrap().checkpoint_type,
            ApprovalCheckpointType::PreMutationCommit
        );
        assert_eq!(manager.pending().unwrap().step_index, 1);
    }

    #[test]
    fn checkpoint_counts() {
        let mut manager = CheckpointManager::new();
        manager.add(ApprovalCheckpoint::pre_execution("Start"));
        manager.add(ApprovalCheckpoint::pre_mutation(1, "Step 1"));
        manager.add(ApprovalCheckpoint::pre_mutation(2, "Step 2"));

        assert_eq!(manager.count_by_state(ApprovalCheckpointState::Pending), 3);

        manager.approve_pending("op");
        assert_eq!(manager.count_by_state(ApprovalCheckpointState::Approved), 1);
        assert_eq!(manager.count_by_state(ApprovalCheckpointState::Pending), 2);
    }
}
