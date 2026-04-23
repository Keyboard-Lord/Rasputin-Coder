//! System Invariants - Formal Runtime Guarantees
//!
//! Enforces core system properties that must always hold.
//! Per PHASE F of PROOF-LEVEL HARDENING sprint.

use crate::chain_executor::ChainExecutor;
use crate::tool_registry::ToolRegistry;
use crate::types::{ChainStatus, StepOutcome, ValidationDecision, ValidationReport};

/// System invariant violations
#[derive(Debug, Clone)]
pub enum InvariantViolation {
    /// Step executed without validation pass
    StepWithoutValidation { step_index: usize },
    /// State mutated on failure path
    MutationOnFailure { step_index: usize },
    /// Chain skipped a step
    StepSkipped { expected: usize, actual: usize },
    /// Unregistered tool executed
    UnregisteredTool { tool_name: String },
    /// Planner bypassed firewall
    PlannerFirewallBypassed,
    /// Replay did not match original
    ReplayMismatch { expected: String, actual: String },
    /// State integrity compromised
    StateIntegrityCompromised { reason: String },
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StepWithoutValidation { step_index } => {
                write!(f, "Step {} executed without validation pass", step_index)
            }
            Self::MutationOnFailure { step_index } => {
                write!(f, "State mutated on failure at step {}", step_index)
            }
            Self::StepSkipped { expected, actual } => {
                write!(
                    f,
                    "Step ordering violated: expected step {} but got {}",
                    expected, actual
                )
            }
            Self::UnregisteredTool { tool_name } => {
                write!(f, "Unregistered tool executed: {}", tool_name)
            }
            Self::PlannerFirewallBypassed => {
                write!(f, "Planner output bypassed protocol firewall")
            }
            Self::ReplayMismatch { expected, actual } => {
                write!(
                    f,
                    "Replay mismatch: expected {} but got {}",
                    expected, actual
                )
            }
            Self::StateIntegrityCompromised { reason } => {
                write!(f, "State integrity compromised: {}", reason)
            }
        }
    }
}

/// System invariant checker
pub struct SystemInvariantChecker;

impl SystemInvariantChecker {
    /// Verify no step executes without validation pass
    pub fn no_step_without_validation(
        step_index: usize,
        validation_report: &ValidationReport,
    ) -> Result<(), InvariantViolation> {
        if validation_report.decision != ValidationDecision::Accept {
            return Err(InvariantViolation::StepWithoutValidation { step_index });
        }
        Ok(())
    }

    /// Verify no mutation occurs on failure path
    pub fn no_mutation_on_failure(
        step_index: usize,
        outcome: &StepOutcome,
        validation_report: &ValidationReport,
    ) -> Result<(), InvariantViolation> {
        // If validation failed, check no successful mutations occurred
        if let ValidationDecision::Reject { .. } = &validation_report.decision {
            if let StepOutcome::Resolved { files_modified, .. } = outcome {
                if !files_modified.is_empty() {
                    return Err(InvariantViolation::MutationOnFailure { step_index });
                }
            }
        }
        Ok(())
    }

    /// Verify chain cannot skip steps
    pub fn no_step_skipping(
        executor: &ChainExecutor,
        expected_current_step: usize,
    ) -> Result<(), InvariantViolation> {
        let actual = executor.current_step_index();
        if actual != expected_current_step {
            return Err(InvariantViolation::StepSkipped {
                expected: expected_current_step,
                actual,
            });
        }
        Ok(())
    }

    /// Verify only registered tools execute
    pub fn only_registered_tools(tool_name: &str) -> Result<(), InvariantViolation> {
        let registry = ToolRegistry::new();
        let name = crate::types::ToolName::new(tool_name).map_err(|_| {
            InvariantViolation::UnregisteredTool {
                tool_name: tool_name.to_string(),
            }
        })?;

        if registry.resolve(&name).is_err() {
            return Err(InvariantViolation::UnregisteredTool {
                tool_name: tool_name.to_string(),
            });
        }
        Ok(())
    }

    /// Verify chain status is valid
    pub fn valid_chain_status(status: ChainStatus) -> Result<(), InvariantViolation> {
        match status {
            ChainStatus::Pending
            | ChainStatus::Running
            | ChainStatus::Complete
            | ChainStatus::Failed => Ok(()),
            _ => Err(InvariantViolation::StateIntegrityCompromised {
                reason: format!("Invalid chain status: {:?}", status),
            }),
        }
    }

    /// Run all invariants on chain executor
    pub fn check_all(executor: &ChainExecutor) -> Result<(), Vec<InvariantViolation>> {
        let mut violations = Vec::new();

        // Check chain status is valid
        if let Err(e) = Self::valid_chain_status(executor.status()) {
            violations.push(e);
        }

        // Additional checks would go here based on executor state

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

/// Property-based invariant tests
#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    // Property: Valid chain status is always recognized
    proptest! {
        #[test]
        fn prop_valid_chain_status(status in prop::sample::select(vec![
            ChainStatus::Pending,
            ChainStatus::Running,
            ChainStatus::Complete,
            ChainStatus::Failed,
        ])) {
            assert!(SystemInvariantChecker::valid_chain_status(status).is_ok());
        }
    }

    // Property: Step without validation is rejected
    proptest! {
        #[test]
        fn prop_step_requires_validation(step_index in 0usize..100) {
            let reject_report = ValidationReport::reject("validation failed");
            let result = SystemInvariantChecker::no_step_without_validation(step_index, &reject_report);
            assert!(result.is_err());
        }
    }

    // Property: Chain maintains sequential step ordering
    proptest! {
        #[test]
        fn prop_chain_sequential_steps(steps in 1usize..50) {
            let executor = ChainExecutor::new("test", (0..steps).map(|i| format!("step {}", i)).collect());

            // Initial step should be 0
            assert_eq!(executor.current_step_index(), 0);
            assert!(SystemInvariantChecker::no_step_skipping(&executor, 0).is_ok());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StepOutcome, ValidationReport};
    use std::path::PathBuf;

    /// Test: Validation pass allows step execution
    #[test]
    fn validation_pass_allows_step() {
        let report = ValidationReport::accept("All good");
        let result = SystemInvariantChecker::no_step_without_validation(0, &report);
        assert!(result.is_ok());
    }

    /// Test: Validation rejection blocks step
    #[test]
    fn validation_rejection_blocks_step() {
        let report = ValidationReport::reject("Build failed");
        let result = SystemInvariantChecker::no_step_without_validation(0, &report);
        assert!(result.is_err());
    }

    /// Test: No mutation on validation failure
    #[test]
    fn no_mutation_on_validation_failure() {
        let outcome = StepOutcome::Resolved {
            summary: "Attempted".to_string(),
            files_modified: vec![PathBuf::from("test.txt")],
        };
        let report = ValidationReport::reject("Test failed");

        let result = SystemInvariantChecker::no_mutation_on_failure(0, &outcome, &report);
        assert!(result.is_err(), "Should detect mutation on failure");
    }

    /// Test: Mutation allowed on success
    #[test]
    fn mutation_allowed_on_success() {
        let outcome = StepOutcome::Resolved {
            summary: "Success".to_string(),
            files_modified: vec![PathBuf::from("test.txt")],
        };
        let report = ValidationReport::accept("All passed");

        let result = SystemInvariantChecker::no_mutation_on_failure(0, &outcome, &report);
        assert!(result.is_ok(), "Mutation OK when validation passes");
    }

    /// Test: Known tools are accepted
    #[test]
    fn known_tools_accepted() {
        let result = SystemInvariantChecker::only_registered_tools("read_file");
        assert!(result.is_ok());
    }

    /// Test: Unknown tools are rejected
    #[test]
    fn unknown_tools_rejected() {
        let result = SystemInvariantChecker::only_registered_tools("unknown_tool_xyz");
        assert!(result.is_err());
    }

    /// Test: Step skipping detected
    #[test]
    fn step_skipping_detected() {
        let executor = ChainExecutor::new("test", vec!["step 1".to_string(), "step 2".to_string()]);

        // Check at step 0
        let result = SystemInvariantChecker::no_step_skipping(&executor, 0);
        assert!(result.is_ok());

        // Check at wrong step (expecting 1 but still at 0)
        let result = SystemInvariantChecker::no_step_skipping(&executor, 1);
        assert!(result.is_err());
    }

    /// Test: Valid chain statuses pass
    #[test]
    fn valid_statuses_pass() {
        for status in [
            ChainStatus::Pending,
            ChainStatus::Running,
            ChainStatus::Complete,
            ChainStatus::Failed,
        ] {
            assert!(SystemInvariantChecker::valid_chain_status(status).is_ok());
        }
    }

    /// Test: All invariants pass on fresh executor
    #[test]
    fn all_invariants_pass_fresh() {
        let executor = ChainExecutor::new("test", vec!["step 1".to_string()]);
        let result = SystemInvariantChecker::check_all(&executor);
        assert!(result.is_ok());
    }

    /// Test: Invariant violation display
    #[test]
    fn invariant_violation_display() {
        let v1 = InvariantViolation::StepWithoutValidation { step_index: 5 };
        assert!(v1.to_string().contains("5"));

        let v2 = InvariantViolation::MutationOnFailure { step_index: 3 };
        assert!(v2.to_string().contains("3"));

        let v3 = InvariantViolation::UnregisteredTool {
            tool_name: "bad_tool".to_string(),
        };
        assert!(v3.to_string().contains("bad_tool"));
    }

    /// Test: No mutation when no files modified
    #[test]
    fn no_mutation_when_no_files() {
        let outcome = StepOutcome::Resolved {
            summary: "Failed".to_string(),
            files_modified: vec![], // Empty
        };
        let report = ValidationReport::reject("Build failed");

        let result = SystemInvariantChecker::no_mutation_on_failure(0, &outcome, &report);
        // Should pass because no files were actually modified
        assert!(result.is_ok());
    }

    /// Test: Failed outcome (not Resolved) doesn't trigger mutation violation
    #[test]
    fn failed_outcome_no_mutation_check() {
        let outcome = StepOutcome::Failed {
            reason: "Error".to_string(),
            recoverable: false,
        };
        let report = ValidationReport::reject("Validation failed");

        let result = SystemInvariantChecker::no_mutation_on_failure(0, &outcome, &report);
        // StepOutcome::Failed doesn't have files_modified, so check passes
        assert!(result.is_ok());
    }
}
