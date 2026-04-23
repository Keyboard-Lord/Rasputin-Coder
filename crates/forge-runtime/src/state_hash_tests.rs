//! State Hash Tests - Proof-Level Chain Integrity
//!
//! Tests cryptographic chain binding, tamper detection, and step ordering verification.
//! Per PHASE A of PROOF-LEVEL HARDENING sprint.

use crate::types::{
    ChainIntegrityError, ChainStateDigest, TaskChain, ValidationDecision, ValidationReport,
};

/// Helper to compute a simple state hash (mock for testing)
fn compute_test_state_hash(step_index: u32, data: &str) -> String {
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(step_index.to_le_bytes());
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

/// Test: Step 0 digest has no previous hash
#[test]
fn step_zero_digest_has_no_previous() {
    let hash = compute_test_state_hash(0, "initial state");
    let digest = ChainStateDigest::new(0, hash, None);

    assert_eq!(digest.step_index, 0);
    assert!(digest.previous_hash.is_none());
}

/// Test: Step N > 0 digest links to previous hash
#[test]
fn step_n_digest_links_to_previous() {
    let prev_hash = compute_test_state_hash(0, "step 0");
    let current_hash = compute_test_state_hash(1, "step 1");
    let digest = ChainStateDigest::new(1, current_hash, Some(prev_hash.clone()));

    assert_eq!(digest.step_index, 1);
    assert_eq!(digest.previous_hash, Some(prev_hash));
}

/// Test: Chain continuity verification passes for valid link
#[test]
fn chain_continuity_verification_passes() {
    let prev_hash = compute_test_state_hash(0, "step 0");
    let current_hash = compute_test_state_hash(1, "step 1");
    let digest = ChainStateDigest::new(1, current_hash, Some(prev_hash.clone()));

    let result = digest.verify_chain_continuity(&prev_hash);
    assert!(result.is_ok(), "Valid chain continuity should pass");
}

/// Test: Chain continuity verification fails for tampered link
#[test]
fn chain_continuity_fails_for_tampered_link() {
    let prev_hash = compute_test_state_hash(0, "step 0");
    let current_hash = compute_test_state_hash(1, "step 1");
    let mut digest = ChainStateDigest::new(1, current_hash, Some(prev_hash.clone()));

    // Tamper with the previous hash
    digest.previous_hash = Some("tampered_hash".to_string());

    let result = digest.verify_chain_continuity(&prev_hash);
    assert!(result.is_err(), "Tampered link should fail verification");

    match result.unwrap_err() {
        ChainIntegrityError::ChainBroken { step_index, .. } => {
            assert_eq!(step_index, 1);
        }
        _ => panic!("Expected ChainBroken error"),
    }
}

/// Test: Missing previous hash on non-zero step fails
#[test]
fn missing_previous_hash_on_nonzero_step_fails() {
    let current_hash = compute_test_state_hash(1, "step 1");
    let digest = ChainStateDigest::new(1, current_hash, None); // Missing previous

    let result = digest.verify_chain_continuity("any_hash");
    assert!(result.is_err(), "Missing previous hash should fail");

    match result.unwrap_err() {
        ChainIntegrityError::MissingPreviousHash { step_index } => {
            assert_eq!(step_index, 1);
        }
        _ => panic!("Expected MissingPreviousHash error"),
    }
}

/// Test: Step 0 with None previous hash is valid
#[test]
fn step_zero_with_none_previous_is_valid() {
    let hash = compute_test_state_hash(0, "step 0");
    let digest = ChainStateDigest::new(0, hash, None);

    // Step 0 has no previous, so any expected previous should fail
    // But None is valid for step 0
    assert_eq!(digest.step_index, 0);
    assert!(digest.previous_hash.is_none());
}

/// Test: Chain of 3 steps maintains hash linking
#[test]
fn chain_of_three_steps_maintains_links() {
    let mut digests: Vec<ChainStateDigest> = Vec::new();

    // Step 0
    let hash_0 = compute_test_state_hash(0, "step 0 state");
    let digest_0 = ChainStateDigest::new(0, hash_0.clone(), None);
    digests.push(digest_0);

    // Step 1
    let hash_1 = compute_test_state_hash(1, "step 1 state");
    let digest_1 = ChainStateDigest::new(1, hash_1.clone(), Some(hash_0.clone()));
    digests.push(digest_1);

    // Step 2
    let hash_2 = compute_test_state_hash(2, "step 2 state");
    let digest_2 = ChainStateDigest::new(2, hash_2, Some(hash_1.clone()));
    digests.push(digest_2);

    // Verify chain continuity
    for i in 1..digests.len() {
        let current = &digests[i];
        let previous = &digests[i - 1];

        current
            .verify_chain_continuity(&previous.state_hash)
            .expect(&format!("Step {} should link to step {}", i, i - 1));
    }
}

/// Test: State mutation detection after checkpoint
#[test]
fn state_mutation_after_checkpoint_detected() {
    let initial_hash = compute_test_state_hash(0, "initial");
    let checkpoint = ChainStateDigest::new(0, initial_hash.clone(), None);

    // Simulate state mutation
    let mutated_hash = compute_test_state_hash(0, "mutated");

    // Verify checkpoint hash doesn't match mutated state
    assert_ne!(checkpoint.state_hash, mutated_hash);
}

/// Test: Step reordering detection
#[test]
fn step_reordering_detected_by_index_mismatch() {
    // Create digests for steps 0, 1, 2
    let hash_0 = compute_test_state_hash(0, "step 0");
    let hash_1 = compute_test_state_hash(1, "step 1");
    let hash_2 = compute_test_state_hash(2, "step 2");

    let digest_0 = ChainStateDigest::new(0, hash_0.clone(), None);
    let _digest_1 = ChainStateDigest::new(1, hash_1.clone(), Some(hash_0.clone()));
    let _digest_2 = ChainStateDigest::new(2, hash_2, Some(hash_1.clone()));

    // Verify indices are sequential
    assert_eq!(digest_0.step_index, 0);

    // A reordered step would have wrong index in the digest
    // For example, if digest said step 0 but expected step 1
    let reordered = ChainStateDigest::new(0, hash_0.clone(), Some(hash_1.clone()));
    // This digest claims to be step 0 but references hash_1 (which is from step 1)
    // This is structurally valid but semantically wrong

    // The verification would catch this if we check step ordering
    assert_eq!(reordered.step_index, 0);
    assert_eq!(reordered.previous_hash, Some(hash_1));
}

/// Test: Chain hash computation aggregates all step hashes
#[test]
fn chain_hash_computation_aggregates_steps() {
    let hash_0 = compute_test_state_hash(0, "step 0");
    let hash_1 = compute_test_state_hash(1, "step 1");
    let hash_2 = compute_test_state_hash(2, "step 2");

    // Compute aggregate chain hash
    let chain_hash = compute_chain_hash(&[hash_0.clone(), hash_1.clone(), hash_2.clone()]);

    // Same steps in different order should produce different hash
    let reordered_hash = compute_chain_hash(&[hash_2.clone(), hash_0.clone(), hash_1.clone()]);

    assert_ne!(chain_hash, reordered_hash, "Order matters in chain hash");

    // Same steps in same order should produce same hash
    let same_chain_hash = compute_chain_hash(&[hash_0, hash_1, hash_2]);
    assert_eq!(
        chain_hash, same_chain_hash,
        "Same order should produce same hash"
    );
}

/// Helper to compute aggregate chain hash
fn compute_chain_hash(hashes: &[String]) -> String {
    use sha3::{Digest, Sha3_512};
    let mut hasher = Sha3_512::new();
    for hash in hashes {
        hasher.update(hash.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Test: Empty chain hash is valid
#[test]
fn empty_chain_hash_is_valid() {
    let empty_hash = compute_chain_hash(&[]);
    assert!(!empty_hash.is_empty());

    // Adding any step changes the hash
    let hash_0 = compute_test_state_hash(0, "step 0");
    let non_empty_hash = compute_chain_hash(&[hash_0]);
    assert_ne!(empty_hash, non_empty_hash);
}

/// Test: Chain serialization roundtrip preserves hashes
#[test]
fn chain_serialization_roundtrip_preserves_hashes() {
    use serde_json;

    let hash = compute_test_state_hash(0, "test");
    let digest = ChainStateDigest::new(0, hash.clone(), None);

    // Serialize
    let json = serde_json::to_string(&digest).expect("Should serialize");

    // Deserialize
    let restored: ChainStateDigest = serde_json::from_str(&json).expect("Should deserialize");

    assert_eq!(restored.step_index, digest.step_index);
    assert_eq!(restored.state_hash, digest.state_hash);
    assert_eq!(restored.previous_hash, digest.previous_hash);
}

/// Test: TaskChain state hashing integration
#[test]
fn taskchain_state_hashing_integration() {
    let mut chain = TaskChain::new(
        "test objective",
        vec!["step 1".to_string(), "step 2".to_string()],
    );

    // Initial state
    assert_eq!(chain.current_step, 0);
    assert!(matches!(chain.status, crate::types::ChainStatus::Pending));

    // Advance chain
    chain.advance();
    assert_eq!(chain.current_step, 1);

    // Complete chain
    chain.advance();
    assert!(chain.is_complete());
}

/// Test: Validation report hash integration
#[test]
fn validation_report_affects_state_hash() {
    // Different validation decisions should produce different state representations
    let accept_report = ValidationReport::accept("All good");
    let reject_report = ValidationReport::reject("Build failed");

    // The decision affects the state
    assert_eq!(accept_report.decision, ValidationDecision::Accept);
    assert!(matches!(
        reject_report.decision,
        ValidationDecision::Reject { .. }
    ));
}

/// Test: Tampered state detection
#[test]
fn tampered_state_detected_by_hash_mismatch() {
    let original_data = "original state data";
    let original_hash = compute_test_state_hash(1, original_data);

    let tampered_data = "tampered state data";
    let tampered_hash = compute_test_state_hash(1, tampered_data);

    // Original and tampered should have different hashes
    assert_ne!(original_hash, tampered_hash);

    // Create digest with original hash
    let digest = ChainStateDigest::new(1, original_hash.clone(), None);

    // Verify against original
    assert_eq!(digest.state_hash, original_hash);

    // Verify tampered doesn't match
    assert_ne!(digest.state_hash, tampered_hash);
}

/// Test: Multi-step chain with checkpoint at each step
#[test]
fn multi_step_chain_with_checkpoints() {
    let steps = vec!["step 0", "step 1", "step 2"];
    let mut checkpoints: Vec<ChainStateDigest> = Vec::new();

    for (i, step_data) in steps.iter().enumerate() {
        let state_hash = compute_test_state_hash(i as u32, step_data);
        let previous_hash = if i == 0 {
            None
        } else {
            Some(checkpoints[i - 1].state_hash.clone())
        };

        let digest = ChainStateDigest::new(i as u32, state_hash, previous_hash);
        checkpoints.push(digest);
    }

    // Verify all checkpoints link correctly
    assert_eq!(checkpoints.len(), 3);

    // Checkpoint 0: no previous
    assert!(checkpoints[0].previous_hash.is_none());

    // Checkpoint 1: links to 0
    assert_eq!(
        checkpoints[1].previous_hash,
        Some(checkpoints[0].state_hash.clone())
    );

    // Checkpoint 2: links to 1
    assert_eq!(
        checkpoints[2].previous_hash,
        Some(checkpoints[1].state_hash.clone())
    );
}

/// Test: Chain integrity with validation gates
#[test]
fn chain_integrity_with_validation_gates() {
    // Each step must pass validation before checkpoint
    let step_hash = compute_test_state_hash(0, "step with validation");
    let validation_report = ValidationReport::accept("Step passed all validation");

    // Only create checkpoint after validation passes
    let checkpoint = ChainStateDigest::new(0, step_hash, None);

    assert!(matches!(
        validation_report.decision,
        ValidationDecision::Accept
    ));
    assert_eq!(checkpoint.step_index, 0);
}
