//! Determinism Guard - Non-Determinism Detection and Enforcement
//!
//! Detects and rejects time-based, random, or external IO operations.
//! Per PHASE B of PROOF-LEVEL HARDENING sprint.

use crate::types::DeterminismViolation;

/// Guard for detecting non-deterministic operations
pub struct DeterminismGuard {
    violations: Vec<DeterminismViolation>,
    strict_mode: bool,
}

impl DeterminismGuard {
    /// Create a new determinism guard in strict mode
    pub fn new() -> Self {
        Self {
            violations: Vec::new(),
            strict_mode: true,
        }
    }

    /// Create with configurable strictness
    pub fn with_strictness(strict: bool) -> Self {
        Self {
            violations: Vec::new(),
            strict_mode: strict,
        }
    }

    /// Record a time usage violation
    pub fn report_time_usage(&mut self, source: &str) {
        self.violations
            .push(DeterminismViolation::TimeUsageDetected {
                source: source.to_string(),
            });
    }

    /// Record a randomness violation
    pub fn report_randomness(&mut self, source: &str) {
        self.violations
            .push(DeterminismViolation::RandomnessDetected {
                source: source.to_string(),
            });
    }

    /// Record an external IO violation
    pub fn report_external_io(&mut self, operation: &str) {
        self.violations
            .push(DeterminismViolation::ExternalIoDetected {
                operation: operation.to_string(),
            });
    }

    /// Record state mutation after checkpoint
    pub fn report_state_mutation(&mut self, step_index: u32) {
        self.violations
            .push(DeterminismViolation::StateMutationAfterCheckpoint { step_index });
    }

    /// Check if any violations have been recorded
    pub fn has_violations(&self) -> bool {
        !self.violations.is_empty()
    }

    /// Get all recorded violations
    pub fn violations(&self) -> &[DeterminismViolation] {
        &self.violations
    }

    /// Verify determinism - returns error if in strict mode and violations exist
    pub fn verify(&self) -> Result<(), Vec<DeterminismViolation>> {
        if self.strict_mode && !self.violations.is_empty() {
            Err(self.violations.clone())
        } else {
            Ok(())
        }
    }

    /// Check if strictly deterministic
    pub fn is_strict(&self) -> bool {
        self.strict_mode
    }

    /// Clear all violations (use with caution)
    pub fn clear(&mut self) {
        self.violations.clear();
    }
}

impl Default for DeterminismGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime error for determinism violations
#[derive(Debug, Clone)]
pub enum RuntimeError {
    DeterminismViolation(Vec<DeterminismViolation>),
    #[allow(dead_code)]
    StateIntegrityError(String),
    #[allow(dead_code)]
    ReplayMismatch(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeterminismViolation(violations) => {
                write!(
                    f,
                    "Determinism violations detected ({} total):",
                    violations.len()
                )?;
                for v in violations {
                    write!(f, "; {}", v)?;
                }
                Ok(())
            }
            Self::StateIntegrityError(msg) => write!(f, "State integrity error: {}", msg),
            Self::ReplayMismatch(msg) => write!(f, "Replay mismatch: {}", msg),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test: Guard detects time usage
    #[test]
    fn guard_detects_time_usage() {
        let mut guard = DeterminismGuard::new();
        assert!(!guard.has_violations());

        guard.report_time_usage("std::time::Instant::now()");

        assert!(guard.has_violations());
        assert_eq!(guard.violations().len(), 1);
    }

    /// Test: Guard detects randomness
    #[test]
    fn guard_detects_randomness() {
        let mut guard = DeterminismGuard::new();

        guard.report_randomness("rand::random()");

        assert!(guard.has_violations());
        match &guard.violations()[0] {
            DeterminismViolation::RandomnessDetected { source } => {
                assert_eq!(source, "rand::random()");
            }
            _ => panic!("Expected RandomnessDetected"),
        }
    }

    /// Test: Guard detects external IO
    #[test]
    fn guard_detects_external_io() {
        let mut guard = DeterminismGuard::new();

        guard.report_external_io("network request");

        assert!(guard.has_violations());
        match &guard.violations()[0] {
            DeterminismViolation::ExternalIoDetected { operation } => {
                assert_eq!(operation, "network request");
            }
            _ => panic!("Expected ExternalIoDetected"),
        }
    }

    /// Test: Guard detects state mutation after checkpoint
    #[test]
    fn guard_detects_state_mutation_after_checkpoint() {
        let mut guard = DeterminismGuard::new();

        guard.report_state_mutation(5);

        assert!(guard.has_violations());
        match &guard.violations()[0] {
            DeterminismViolation::StateMutationAfterCheckpoint { step_index } => {
                assert_eq!(*step_index, 5);
            }
            _ => panic!("Expected StateMutationAfterCheckpoint"),
        }
    }

    /// Test: Verify passes when no violations in strict mode
    #[test]
    fn verify_passes_when_no_violations() {
        let guard = DeterminismGuard::new();
        let result = guard.verify();
        assert!(result.is_ok());
    }

    /// Test: Verify fails when violations in strict mode
    #[test]
    fn verify_fails_when_violations_in_strict_mode() {
        let mut guard = DeterminismGuard::new();
        guard.report_randomness("test");

        let result = guard.verify();
        assert!(result.is_err());
    }

    /// Test: Verify passes in non-strict mode even with violations
    #[test]
    fn verify_passes_in_non_strict_mode() {
        let mut guard = DeterminismGuard::with_strictness(false);
        guard.report_randomness("test");

        let result = guard.verify();
        assert!(result.is_ok());
    }

    /// Test: Multiple violations accumulated
    #[test]
    fn multiple_violations_accumulated() {
        let mut guard = DeterminismGuard::new();

        guard.report_time_usage("time1");
        guard.report_randomness("rand1");
        guard.report_external_io("io1");

        assert_eq!(guard.violations().len(), 3);
    }

    /// Test: Clear removes all violations
    #[test]
    fn clear_removes_all_violations() {
        let mut guard = DeterminismGuard::new();
        guard.report_randomness("test");
        assert!(guard.has_violations());

        guard.clear();
        assert!(!guard.has_violations());
    }

    /// Test: RuntimeError displays correctly
    #[test]
    fn runtime_error_display() {
        let mut guard = DeterminismGuard::new();
        guard.report_randomness("source");
        let err = RuntimeError::DeterminismViolation(guard.violations().to_vec());

        let msg = err.to_string();
        assert!(msg.contains("Determinism violations"));
        assert!(msg.contains("randomness"));
    }

    /// Test: DeterminismViolation display formats correctly
    #[test]
    fn determinism_violation_display() {
        let v1 = DeterminismViolation::TimeUsageDetected {
            source: "timer".to_string(),
        };
        assert!(v1.to_string().contains("timer"));

        let v2 = DeterminismViolation::RandomnessDetected {
            source: "rng".to_string(),
        };
        assert!(v2.to_string().contains("rng"));

        let v3 = DeterminismViolation::ExternalIoDetected {
            operation: "file read".to_string(),
        };
        assert!(v3.to_string().contains("file read"));

        let v4 = DeterminismViolation::StateMutationAfterCheckpoint { step_index: 3 };
        assert!(v4.to_string().contains("3"));
    }

    /// Test: Strict mode is default
    #[test]
    fn strict_mode_is_default() {
        let guard = DeterminismGuard::new();
        assert!(guard.is_strict());
    }

    /// Test: Non-strict mode can be configured
    #[test]
    fn non_strict_mode_configurable() {
        let guard = DeterminismGuard::with_strictness(false);
        assert!(!guard.is_strict());
    }
}
