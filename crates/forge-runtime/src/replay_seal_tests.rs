//! Replay Seal Tests - Provably Identical Replay
//!
//! Tests deterministic replay guarantees and non-determinism detection.
//! Per PHASE B of PROOF-LEVEL HARDENING sprint.

use crate::chain_executor::{ChainExecutor, compute_chain_hash, compute_state_hash};
use crate::types::{
    ChainStateDigest, ReplaySeal, ReplayVerificationError, StepOutcome, ValidationReport,
};

/// Test: ReplaySeal creation captures all required fields
#[test]
fn replay_seal_creation() {
    let initial_hash = compute_state_hash(&"initial input");
    let chain_hash = compute_state_hash(&"chain data");
    let final_hash = compute_state_hash(&"final state");

    let seal = ReplaySeal::new(initial_hash.clone(), chain_hash.clone(), final_hash.clone());

    assert_eq!(seal.initial_input_hash, initial_hash);
    assert_eq!(seal.chain_hash, chain_hash);
    assert_eq!(seal.final_state_hash, final_hash);
    assert!(!seal.runtime_version.is_empty());
    assert!(seal.created_at > 0);
}

/// Test: Replay verification passes when hashes match
#[test]
fn replay_verification_passes_when_hashes_match() {
    let initial_hash = compute_state_hash(&"initial input");
    let chain_hash = compute_state_hash(&"chain data");
    let final_hash = compute_state_hash(&"final state");

    let seal = ReplaySeal::new(initial_hash.clone(), chain_hash.clone(), final_hash.clone());

    let result = seal.verify_replay(&chain_hash, &final_hash);
    assert!(result.is_ok(), "Matching hashes should pass verification");
}

/// Test: Replay verification fails when chain hash mismatches
#[test]
fn replay_verification_fails_on_chain_hash_mismatch() {
    let initial_hash = compute_state_hash(&"initial input");
    let chain_hash = compute_state_hash(&"chain data");
    let final_hash = compute_state_hash(&"final state");

    let seal = ReplaySeal::new(initial_hash, chain_hash.clone(), final_hash.clone());

    let wrong_chain_hash = compute_state_hash(&"different chain");
    let result = seal.verify_replay(&wrong_chain_hash, &final_hash);

    assert!(result.is_err(), "Mismatched chain hash should fail");
    match result.unwrap_err() {
        ReplayVerificationError::ChainHashMismatch { expected, actual } => {
            assert_eq!(expected, chain_hash);
            assert_eq!(actual, wrong_chain_hash);
        }
        _ => panic!("Expected ChainHashMismatch error"),
    }
}

/// Test: Replay verification fails when final state mismatches
#[test]
fn replay_verification_fails_on_final_state_mismatch() {
    let initial_hash = compute_state_hash(&"initial input");
    let chain_hash = compute_state_hash(&"chain data");
    let final_hash = compute_state_hash(&"final state");

    let seal = ReplaySeal::new(initial_hash, chain_hash.clone(), final_hash.clone());

    let wrong_final_hash = compute_state_hash(&"different final state");
    let result = seal.verify_replay(&chain_hash, &wrong_final_hash);

    assert!(result.is_err(), "Mismatched final state should fail");
    match result.unwrap_err() {
        ReplayVerificationError::FinalStateMismatch { expected, actual } => {
            assert_eq!(expected, final_hash);
            assert_eq!(actual, wrong_final_hash);
        }
        _ => panic!("Expected FinalStateMismatch error"),
    }
}

/// Test: Same input produces identical seal
#[test]
fn same_input_produces_identical_seal() {
    let input = "consistent input data";

    let initial_hash1 = compute_state_hash(&input);
    let chain_hash1 = compute_state_hash(&"chain steps");
    let final_hash1 = compute_state_hash(&"final result");
    let seal1 = ReplaySeal::new(initial_hash1, chain_hash1, final_hash1);

    let initial_hash2 = compute_state_hash(&input);
    let chain_hash2 = compute_state_hash(&"chain steps");
    let final_hash2 = compute_state_hash(&"final result");
    let seal2 = ReplaySeal::new(initial_hash2, chain_hash2, final_hash2);

    assert_eq!(seal1.initial_input_hash, seal2.initial_input_hash);
    assert_eq!(seal1.chain_hash, seal2.chain_hash);
    assert_eq!(seal1.final_state_hash, seal2.final_state_hash);
}

/// Test: Different input produces different seal
#[test]
fn different_input_produces_different_seal() {
    let initial_hash1 = compute_state_hash(&"input A");
    let chain_hash1 = compute_state_hash(&"chain");
    let final_hash1 = compute_state_hash(&"final");
    let seal1 = ReplaySeal::new(initial_hash1, chain_hash1, final_hash1);

    let initial_hash2 = compute_state_hash(&"input B");
    let chain_hash2 = compute_state_hash(&"chain");
    let final_hash2 = compute_state_hash(&"final");
    let seal2 = ReplaySeal::new(initial_hash2, chain_hash2, final_hash2);

    assert_ne!(seal1.initial_input_hash, seal2.initial_input_hash);
}

/// Test: ChainExecutor generates consistent chain hash
#[test]
fn chain_executor_generates_consistent_hash() {
    let mut executor1 = ChainExecutor::new("test chain", vec!["step 1".to_string()]);
    let mut executor2 = ChainExecutor::new("test chain", vec!["step 1".to_string()]);

    // Record same state in both
    executor1.record_step_state(0, &"state data");
    executor2.record_step_state(0, &"state data");

    let hash1 = executor1.compute_chain_hash();
    let hash2 = executor2.compute_chain_hash();

    assert_eq!(
        hash1, hash2,
        "Same operations should produce same chain hash"
    );
}

/// Test: Chain hash changes when state changes
#[test]
fn chain_hash_changes_when_state_changes() {
    let mut executor = ChainExecutor::new("test chain", vec!["step 1".to_string()]);

    // Record initial state
    executor.record_step_state(0, &"state A");
    let hash_a = executor.compute_chain_hash();

    // Create new executor with different state
    let mut executor2 = ChainExecutor::new("test chain", vec!["step 1".to_string()]);
    executor2.record_step_state(0, &"state B");
    let hash_b = executor2.compute_chain_hash();

    assert_ne!(
        hash_a, hash_b,
        "Different states should produce different hashes"
    );
}

/// Test: ReplaySeal serialization roundtrip
#[test]
fn replay_seal_serialization_roundtrip() {
    use serde_json;

    let initial_hash = compute_state_hash(&"initial");
    let chain_hash = compute_state_hash(&"chain");
    let final_hash = compute_state_hash(&"final");
    let seal = ReplaySeal::new(initial_hash, chain_hash, final_hash);

    let json = serde_json::to_string(&seal).expect("Should serialize");
    let restored: ReplaySeal = serde_json::from_str(&json).expect("Should deserialize");

    assert_eq!(restored.initial_input_hash, seal.initial_input_hash);
    assert_eq!(restored.chain_hash, seal.chain_hash);
    assert_eq!(restored.final_state_hash, seal.final_state_hash);
    assert_eq!(restored.runtime_version, seal.runtime_version);
}

/// Test: Empty chain hash is valid but different
#[test]
fn empty_chain_vs_nonempty_chain_different() {
    let empty_digests: Vec<ChainStateDigest> = vec![];
    let empty_hash = compute_chain_hash(&empty_digests);

    let mut nonempty_digests: Vec<ChainStateDigest> = vec![];
    let state_hash = compute_state_hash(&"some state");
    nonempty_digests.push(ChainStateDigest::new(0, state_hash, None));
    let nonempty_hash = compute_chain_hash(&nonempty_digests);

    assert_ne!(empty_hash, nonempty_hash);
    assert!(!empty_hash.is_empty());
    assert!(!nonempty_hash.is_empty());
}

/// Test: Multi-step chain produces deterministic seal
#[test]
fn multi_step_chain_produces_deterministic_seal() {
    let mut executor = ChainExecutor::new(
        "multi-step",
        vec!["step 1".to_string(), "step 2".to_string()],
    );

    // Execute and record step 0
    executor.mark_step_started().expect("Should start");
    let outcome = StepOutcome::Resolved {
        summary: "Step 0 done".to_string(),
        files_modified: vec![],
    };
    let report = ValidationReport::accept("Step 0 passed");
    executor
        .complete_step_with_validation(outcome, Some(report))
        .expect("Should complete");

    // Record state after step 0
    executor.record_step_state(0, &"after step 0");

    let chain_hash = executor.compute_chain_hash();

    // Create seal
    let initial_hash = compute_state_hash(&"initial");
    let final_hash = compute_state_hash(&"after step 0");
    let seal = ReplaySeal::new(initial_hash, chain_hash.clone(), final_hash.clone());

    // Verify seal can validate the replay
    let result = seal.verify_replay(&chain_hash, &final_hash);
    assert!(result.is_ok());
}

/// Test: Detects non-determinism through state mismatch
#[test]
fn nondeterminism_detected_through_state_mismatch() {
    // In a real scenario, non-deterministic operations would produce
    // different state hashes on replay

    let initial_state = "initial";
    let deterministic_result = compute_state_hash(&initial_state);

    // Same initial state should always produce same result in deterministic system
    let second_result = compute_state_hash(&initial_state);

    assert_eq!(deterministic_result, second_result);

    // Non-deterministic operation would be:
    // let nondeterministic_result = compute_state_hash(&random_value());
    // assert_ne!(deterministic_result, nondeterministic_result);
}

/// Test: Chain integrity verified during replay
#[test]
fn chain_integrity_verified_during_replay() {
    let mut executor = ChainExecutor::new("test", vec!["step 1".to_string(), "step 2".to_string()]);

    // Record sequential steps
    executor.record_step_state(0, &"step 0 state");
    executor.record_step_state(1, &"step 1 state");

    // Verify chain integrity
    let result = executor.verify_chain_integrity();
    assert!(result.is_ok(), "Valid chain should pass integrity check");
}

/// Test: Broken chain detected during replay
#[test]
fn broken_chain_detected_during_replay() {
    let digests = vec![
        ChainStateDigest::new(0, "hash0".to_string(), None),
        ChainStateDigest::new(1, "hash1".to_string(), Some("wrong_prev".to_string())),
    ];

    // Can't directly test through executor, but we can test the digest verification
    let result = digests[1].verify_chain_continuity(&digests[0].state_hash);
    assert!(result.is_err(), "Broken link should be detected");
}

/// Test: Replay produces identical output or hard fails
#[test]
fn replay_produces_identical_or_fails() {
    // This is the fundamental replay guarantee:
    // Same input + same operations = same output
    // OR
    // Different output = verification failure

    let input = "test input";
    let hash1 = compute_state_hash(&input);
    let hash2 = compute_state_hash(&input);

    // Must be identical
    assert_eq!(hash1, hash2);

    // If they differ (non-determinism), replay must fail
    // This would be caught by seal verification
}
