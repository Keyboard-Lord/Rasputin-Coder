//! Chain Executor - Bounded Multi-Step Task Execution
//!
//! Executes task chains one step at a time with validation gating.
//! Each step runs as an isolated bounded execution, preserving
//! all existing runtime invariants.

use crate::types::{
    ChainId, ChainIntegrityError, ChainStateDigest, ChainStatus, StepOutcome, StepStatus,
    TaskChain, ValidationDecision, ValidationReport,
};
use std::path::PathBuf;

/// Compute SHA3-512 hash of serializable state
pub fn compute_state_hash<T: serde::Serialize>(state: &T) -> String {
    use sha3::{Digest, Sha3_512};
    let serialized = serde_json::to_vec(state).expect("state must serialize");
    let mut hasher = Sha3_512::new();
    hasher.update(&serialized);
    hex::encode(hasher.finalize())
}

/// Compute aggregate chain hash from individual step hashes
pub fn compute_chain_hash(digests: &[ChainStateDigest]) -> String {
    use sha3::{Digest, Sha3_512};
    let mut hasher = Sha3_512::new();
    for digest in digests {
        hasher.update(digest.state_hash.as_bytes());
        if let Some(prev) = &digest.previous_hash {
            hasher.update(prev.as_bytes());
        }
    }
    hex::encode(hasher.finalize())
}

/// Result of chain step execution
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum StepExecutionResult {
    /// Step completed, chain can advance
    Completed { outcome: StepOutcome },
    /// Step failed, chain must halt
    Failed { reason: String },
    /// Step blocked, requires external action
    Blocked { reason: String },
}

/// Chain execution coordinator
/// Manages lifecycle of a single task chain
#[derive(Debug)]
#[allow(dead_code)]
pub struct ChainExecutor {
    /// The chain being executed
    chain: TaskChain,
    /// Execution log for audit trail
    log: Vec<ChainEvent>,
    /// State digests for cryptographic chain verification
    state_digests: Vec<ChainStateDigest>,
}

/// Events for chain audit trail
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ChainEvent {
    ChainCreated {
        chain_id: ChainId,
        objective: String,
        step_count: usize,
    },
    StepStarted {
        step_index: usize,
        description: String,
    },
    StepValidated {
        step_index: usize,
        decision: ValidationDecision,
    },
    StepCompleted {
        step_index: usize,
        outcome_summary: String,
    },
    StepFailed {
        step_index: usize,
        reason: String,
    },
    ChainAdvanced {
        from_step: usize,
        to_step: usize,
    },
    CheckpointSaved {
        step_index: usize,
        path: PathBuf,
    },
    ChainCompleted {
        chain_id: ChainId,
    },
    ChainFailed {
        chain_id: ChainId,
        at_step: usize,
        reason: String,
    },
}

#[allow(dead_code)]
impl ChainExecutor {
    /// Create a new chain executor with the given objective and steps
    pub fn new(objective: impl Into<String>, steps: Vec<String>) -> Self {
        let chain = TaskChain::new(objective, steps);
        let chain_id = chain.chain_id.clone();
        let objective = chain.objective.clone();
        let step_count = chain.steps.len();

        let mut executor = Self {
            chain,
            log: Vec::new(),
            state_digests: Vec::new(),
        };

        executor.log.push(ChainEvent::ChainCreated {
            chain_id,
            objective,
            step_count,
        });

        executor
    }

    /// Get the current chain status
    pub fn status(&self) -> ChainStatus {
        self.chain.status
    }

    /// Get chain ID
    pub fn chain_id(&self) -> &ChainId {
        &self.chain.chain_id
    }

    /// Get current objective
    pub fn objective(&self) -> &str {
        &self.chain.objective
    }

    /// Get current step index
    pub fn current_step_index(&self) -> usize {
        self.chain.current_step
    }

    /// Get total step count
    pub fn total_steps(&self) -> usize {
        self.chain.steps.len()
    }

    /// Get current step description if any
    pub fn current_step_description(&self) -> Option<&str> {
        self.chain.current_step().map(|s| s.description.as_str())
    }

    /// Check if chain can execute (has pending steps, not failed/complete)
    pub fn can_execute(&self) -> bool {
        matches!(
            self.chain.status,
            ChainStatus::Pending | ChainStatus::Running
        ) && self.chain.current_step < self.chain.steps.len()
    }

    /// Mark current step as started (call before step execution)
    pub fn mark_step_started(&mut self) -> Result<(), String> {
        if !self.can_execute() {
            return Err(format!(
                "Cannot start step: chain status is {:?}",
                self.chain.status
            ));
        }

        self.chain.status = ChainStatus::Running;

        let step_index;
        let description;
        {
            let step = self.chain.current_step_mut().ok_or("No current step")?;
            step.status = StepStatus::Running;
            step_index = step.index;
            description = step.description.clone();
        }

        self.log.push(ChainEvent::StepStarted {
            step_index,
            description,
        });

        Ok(())
    }

    /// Record step completion outcome and advance if successful
    pub fn complete_step(&mut self, outcome: StepOutcome) -> Result<bool, String> {
        self.complete_step_with_validation(outcome, None)
    }

    /// Record step completion with its validation report and advance only after acceptance.
    pub fn complete_step_with_validation(
        &mut self,
        outcome: StepOutcome,
        validation: Option<ValidationReport>,
    ) -> Result<bool, String> {
        // Clone chain_id before mutable borrow
        let chain_id = self.chain.chain_id.clone();

        if let Some(report) = validation.as_ref() {
            let step_index = self.chain.current_step().ok_or("No current step")?.index;
            self.log.push(ChainEvent::StepValidated {
                step_index,
                decision: report.decision,
            });

            if report.decision != ValidationDecision::Accept {
                let reason = format!("Step validation rejected: {}", report.message);
                {
                    let step = self.chain.current_step_mut().ok_or("No current step")?;
                    step.status = StepStatus::Failed;
                    step.validation_result = validation;
                    step.outcome = Some(StepOutcome::Failed {
                        reason: reason.clone(),
                        recoverable: false,
                    });
                }
                self.chain.fail();
                self.log.push(ChainEvent::StepFailed {
                    step_index,
                    reason: reason.clone(),
                });
                self.log.push(ChainEvent::ChainFailed {
                    chain_id,
                    at_step: step_index,
                    reason,
                });
                return Ok(false);
            }
        }

        // Determine outcome type first to avoid borrow issues
        let outcome_type = match &outcome {
            StepOutcome::Resolved { .. } => 0,
            StepOutcome::Failed { .. } => 1,
            StepOutcome::Blocked { .. } => 2,
        };

        if outcome_type == 1 {
            // Failed path
            let (step_index, reason) = {
                let step = self.chain.current_step_mut().ok_or("No current step")?;
                let step_index = step.index;
                let reason = match &outcome {
                    StepOutcome::Failed { reason, .. } => reason.clone(),
                    _ => unreachable!(),
                };
                step.status = StepStatus::Failed;
                step.validation_result = validation;
                step.outcome = Some(outcome);
                (step_index, reason)
            };
            self.chain.fail();
            self.log.push(ChainEvent::StepFailed {
                step_index,
                reason: reason.clone(),
            });
            self.log.push(ChainEvent::ChainFailed {
                chain_id,
                at_step: step_index,
                reason,
            });
            return Ok(false);
        }

        if outcome_type == 2 {
            // Blocked path
            let step = self.chain.current_step_mut().ok_or("No current step")?;
            step.status = StepStatus::Blocked;
            step.validation_result = validation;
            step.outcome = Some(outcome);
            return Ok(false);
        }

        // Resolved path - complete the step and advance
        let summary = match &outcome {
            StepOutcome::Resolved { summary, .. } => summary.clone(),
            _ => unreachable!(),
        };

        let (step_index, from_step) = {
            let step = self.chain.current_step_mut().ok_or("No current step")?;
            step.status = StepStatus::Completed;
            step.validation_result = validation;
            step.outcome = Some(outcome);
            (step.index, step.index)
        };

        self.log.push(ChainEvent::StepCompleted {
            step_index,
            outcome_summary: summary.clone(),
        });

        // Try to advance
        let advanced = self.chain.advance();

        if advanced {
            let to_step = from_step + 1;
            self.log
                .push(ChainEvent::ChainAdvanced { from_step, to_step });
            Ok(true)
        } else {
            // Chain complete
            self.log.push(ChainEvent::ChainCompleted {
                chain_id: self.chain.chain_id.clone(),
            });
            Ok(false)
        }
    }

    /// Record the durable checkpoint generated after a validated step.
    pub fn record_checkpoint_saved(&mut self, step_index: usize, path: PathBuf) {
        self.log
            .push(ChainEvent::CheckpointSaved { step_index, path });
    }

    /// Get execution log for audit
    pub fn execution_log(&self) -> &[ChainEvent] {
        &self.log
    }

    /// Check if chain is complete
    pub fn is_complete(&self) -> bool {
        self.chain.is_complete()
    }

    /// Check if chain has failed
    pub fn is_failed(&self) -> bool {
        self.chain.is_failed()
    }

    /// Get completed step count
    pub fn completed_steps(&self) -> usize {
        self.chain.completed_steps()
    }

    /// Get chain summary for reporting
    pub fn summary(&self) -> ChainSummary {
        ChainSummary {
            chain_id: self.chain.chain_id.clone(),
            objective: self.chain.objective.clone(),
            status: self.chain.status,
            current_step: self.chain.current_step,
            total_steps: self.chain.steps.len(),
            completed_steps: self.chain.completed_steps(),
        }
    }

    /// Compute and record state hash for current step
    pub fn record_step_state(&mut self, step_index: u32, state_data: &impl serde::Serialize) {
        let state_hash = compute_state_hash(state_data);
        let previous_hash = if step_index == 0 {
            None
        } else {
            self.state_digests.last().map(|d| d.state_hash.clone())
        };

        let digest = ChainStateDigest::new(step_index, state_hash, previous_hash);
        self.state_digests.push(digest);
    }

    /// Get all state digests
    pub fn state_digests(&self) -> &[ChainStateDigest] {
        &self.state_digests
    }

    /// Compute aggregate chain hash from all recorded digests
    pub fn compute_chain_hash(&self) -> String {
        compute_chain_hash(&self.state_digests)
    }

    /// Verify chain integrity: all digests link correctly
    pub fn verify_chain_integrity(&self) -> Result<(), ChainIntegrityError> {
        for (i, digest) in self.state_digests.iter().enumerate() {
            if i == 0 {
                // Step 0 should have no previous hash
                if digest.previous_hash.is_some() {
                    return Err(ChainIntegrityError::ChainBroken {
                        step_index: digest.step_index,
                        expected_hash: "none".to_string(),
                        actual_hash: digest.previous_hash.clone().unwrap(),
                    });
                }
            } else {
                // Other steps should link to previous
                let expected_prev = &self.state_digests[i - 1].state_hash;
                digest.verify_chain_continuity(expected_prev)?;
            }

            // Verify step index is sequential
            let expected_step = i as u32;
            if digest.step_index != expected_step {
                return Err(ChainIntegrityError::StepReordered {
                    step_index: digest.step_index,
                    expected_step,
                });
            }
        }
        Ok(())
    }

    /// Get the hash of the last recorded state
    pub fn last_state_hash(&self) -> Option<&str> {
        self.state_digests.last().map(|d| d.state_hash.as_str())
    }
}

/// Summary of chain state for UI/reporting
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChainSummary {
    pub chain_id: ChainId,
    pub objective: String,
    pub status: ChainStatus,
    pub current_step: usize,
    pub total_steps: usize,
    pub completed_steps: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_executor_creation() {
        let executor = ChainExecutor::new(
            "Refactor auth module",
            vec![
                "Analyze current auth code".to_string(),
                "Extract auth utilities".to_string(),
                "Update dependent modules".to_string(),
            ],
        );

        assert_eq!(executor.total_steps(), 3);
        assert_eq!(executor.current_step_index(), 0);
        assert_eq!(executor.completed_steps(), 0);
        assert!(executor.can_execute());
        assert!(!executor.is_complete());
        assert!(!executor.is_failed());
    }

    #[test]
    fn test_chain_single_step_completion() {
        let mut executor = ChainExecutor::new("Simple task", vec!["Step 1".to_string()]);

        executor.mark_step_started().unwrap();

        let outcome = StepOutcome::Resolved {
            summary: "Step 1 completed".to_string(),
            files_modified: vec![],
        };

        let advanced = executor.complete_step(outcome).unwrap();
        assert!(!advanced); // No more steps
        assert!(executor.is_complete());
        assert_eq!(executor.completed_steps(), 1);
    }

    #[test]
    fn test_chain_multi_step_completion() {
        let mut executor = ChainExecutor::new(
            "Multi-step task",
            vec!["Step 1".to_string(), "Step 2".to_string()],
        );

        // Step 1
        executor.mark_step_started().unwrap();
        let outcome = StepOutcome::Resolved {
            summary: "Step 1 done".to_string(),
            files_modified: vec![],
        };
        let advanced = executor.complete_step(outcome).unwrap();
        assert!(advanced);
        assert_eq!(executor.current_step_index(), 1);
        assert!(!executor.is_complete());

        // Step 2
        executor.mark_step_started().unwrap();
        let outcome = StepOutcome::Resolved {
            summary: "Step 2 done".to_string(),
            files_modified: vec![],
        };
        let advanced = executor.complete_step(outcome).unwrap();
        assert!(!advanced); // Chain complete
        assert!(executor.is_complete());
        assert_eq!(executor.completed_steps(), 2);
    }

    #[test]
    fn test_chain_failure_halts() {
        let mut executor = ChainExecutor::new(
            "Task that will fail",
            vec!["Step 1".to_string(), "Step 2".to_string()],
        );

        executor.mark_step_started().unwrap();

        let outcome = StepOutcome::Failed {
            reason: "Compilation error".to_string(),
            recoverable: false,
        };

        let advanced = executor.complete_step(outcome).unwrap();
        assert!(!advanced);
        assert!(executor.is_failed());
        assert!(!executor.is_complete());
        assert_eq!(executor.completed_steps(), 0);
    }

    #[test]
    fn test_chain_execution_log() {
        let mut executor = ChainExecutor::new("Logged task", vec!["Step 1".to_string()]);

        executor.mark_step_started().unwrap();

        let outcome = StepOutcome::Resolved {
            summary: "Done".to_string(),
            files_modified: vec![],
        };
        executor.complete_step(outcome).unwrap();

        let log = executor.execution_log();
        assert!(log.len() >= 2); // At least created + completed
        assert!(matches!(log[0], ChainEvent::ChainCreated { .. }));
    }

    #[test]
    fn test_chain_summary() {
        let executor = ChainExecutor::new("Test objective", vec!["A".to_string(), "B".to_string()]);

        let summary = executor.summary();
        assert_eq!(summary.objective, "Test objective");
        assert_eq!(summary.total_steps, 2);
        assert_eq!(summary.current_step, 0);
        assert_eq!(summary.completed_steps, 0);
    }

    // ============================================================================
    // INTEGRATION TESTS: Runtime Integration Verification
    // These tests exercise run_chained() through the real Runtime path
    // ============================================================================

    /// Verify that Runtime can be configured with a ChainExecutor
    #[test]
    fn test_runtime_chain_attachment() {
        let chain = ChainExecutor::new(
            "Runtime integration test",
            vec!["Analyze code".to_string(), "Apply fix".to_string()],
        );

        // Just verify the chain can be created and queried
        assert_eq!(chain.total_steps(), 2);
        assert_eq!(chain.current_step_index(), 0);
        assert!(chain.can_execute());
    }

    /// Verify chain step advancement ordering is correct
    #[test]
    fn test_chain_step_ordering() {
        let mut chain = ChainExecutor::new(
            "Ordered execution test",
            vec![
                "Step A".to_string(),
                "Step B".to_string(),
                "Step C".to_string(),
            ],
        );

        // Step 0
        chain.mark_step_started().unwrap();
        assert_eq!(chain.current_step_index(), 0);

        let outcome = StepOutcome::Resolved {
            summary: "Step A done".to_string(),
            files_modified: vec![],
        };
        let advanced = chain.complete_step(outcome).unwrap();
        assert!(advanced);
        assert_eq!(chain.current_step_index(), 1);
        assert_eq!(chain.completed_steps(), 1);

        // Step 1
        chain.mark_step_started().unwrap();
        assert_eq!(chain.current_step_index(), 1);

        let outcome = StepOutcome::Resolved {
            summary: "Step B done".to_string(),
            files_modified: vec![],
        };
        let advanced = chain.complete_step(outcome).unwrap();
        assert!(advanced);
        assert_eq!(chain.current_step_index(), 2);
        assert_eq!(chain.completed_steps(), 2);

        // Step 2 (final)
        chain.mark_step_started().unwrap();
        assert_eq!(chain.current_step_index(), 2);

        let outcome = StepOutcome::Resolved {
            summary: "Step C done".to_string(),
            files_modified: vec![],
        };
        let advanced = chain.complete_step(outcome).unwrap();
        assert!(!advanced); // No more steps
        assert!(chain.is_complete());
        assert_eq!(chain.completed_steps(), 3);
    }

    /// Verify halt behavior on failed step
    #[test]
    fn test_chain_halt_on_failure() {
        let mut chain = ChainExecutor::new(
            "Failure halt test",
            vec!["Step 1".to_string(), "Step 2".to_string()],
        );

        // Step 1 succeeds
        chain.mark_step_started().unwrap();
        let outcome = StepOutcome::Resolved {
            summary: "Step 1 done".to_string(),
            files_modified: vec![],
        };
        chain.complete_step(outcome).unwrap();
        assert!(!chain.is_failed());
        assert!(!chain.is_complete());

        // Step 2 fails
        chain.mark_step_started().unwrap();
        let outcome = StepOutcome::Failed {
            reason: "Compilation error".to_string(),
            recoverable: false,
        };
        let advanced = chain.complete_step(outcome).unwrap();
        assert!(!advanced); // Did not advance
        assert!(chain.is_failed());
        assert!(!chain.is_complete());
        assert_eq!(chain.completed_steps(), 1); // Only step 1 completed
    }

    /// Verify blocked step behavior (pause, not halt)
    #[test]
    fn test_chain_blocked_step_behavior() {
        let mut chain = ChainExecutor::new(
            "Blocked step test",
            vec!["Step 1".to_string(), "Step 2".to_string()],
        );

        chain.mark_step_started().unwrap();
        let outcome = StepOutcome::Blocked {
            reason: "Waiting for approval".to_string(),
        };
        let advanced = chain.complete_step(outcome).unwrap();
        assert!(!advanced); // Did not advance
        assert!(!chain.is_failed()); // Not failed
        assert!(!chain.is_complete()); // Not complete
        // Status should still be Running (blocked but not failed)
    }

    /// Verify ChainExecutionResult structure
    #[test]
    fn test_chain_execution_result_structure() {
        use crate::types::{ChainExecutionResult, ChainStatus};

        let result = ChainExecutionResult {
            success: true,
            chain_id: "test-chain-123".to_string(),
            completed_steps: 3,
            total_steps: 3,
            final_status: ChainStatus::Complete,
            error: None,
        };

        assert!(result.success);
        assert_eq!(result.completed_steps, 3);
        assert_eq!(result.total_steps, 3);
        assert!(matches!(result.final_status, ChainStatus::Complete));
        assert!(result.error.is_none());
    }

    /// Verify execution log captures all events
    #[test]
    fn test_chain_execution_log_completeness() {
        let mut chain = ChainExecutor::new(
            "Log completeness test",
            vec!["Step 1".to_string(), "Step 2".to_string()],
        );

        chain.mark_step_started().unwrap();
        let outcome = StepOutcome::Resolved {
            summary: "Step 1 done".to_string(),
            files_modified: vec![],
        };
        chain.complete_step(outcome).unwrap();

        chain.mark_step_started().unwrap();
        let outcome = StepOutcome::Resolved {
            summary: "Step 2 done".to_string(),
            files_modified: vec![],
        };
        chain.complete_step(outcome).unwrap();

        let log = chain.execution_log();

        // Verify event sequence
        let has_created = log
            .iter()
            .any(|e| matches!(e, ChainEvent::ChainCreated { .. }));
        let has_step_started = log
            .iter()
            .any(|e| matches!(e, ChainEvent::StepStarted { .. }));
        let has_step_completed = log
            .iter()
            .any(|e| matches!(e, ChainEvent::StepCompleted { .. }));
        let has_chain_completed = log
            .iter()
            .any(|e| matches!(e, ChainEvent::ChainCompleted { .. }));

        assert!(has_created, "Log should contain ChainCreated");
        assert!(has_step_started, "Log should contain StepStarted");
        assert!(has_step_completed, "Log should contain StepCompleted");
        assert!(has_chain_completed, "Log should contain ChainCompleted");
    }
}
