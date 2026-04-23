//! Stable Cryptographic Hash Functions for Rasputin
//!
//! Replaces DefaultHasher (SipHash) with stable, portable SHA-256.
//! Per audit requirement: hashes must be stable across runs and platforms.

use sha2::{Digest, Sha256 as Sha2Digest};

/// SHA-256 content hashing wrapper.
pub struct Sha256;

impl Sha256 {
    /// Compute SHA-256 hash of input bytes, return as lowercase hex.
    pub fn digest(data: &[u8]) -> String {
        let mut hasher = Sha2Digest::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }
}

/// Compute content hash in canonical format: sha256:hex
/// This is the standard format used throughout Rasputin for content identity.
pub fn compute_content_hash(content: &str) -> String {
    let hash = Sha256::digest(content.as_bytes());
    format!("sha256:{}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_is_deterministic() {
        let data = "hello world";
        let hash1 = compute_content_hash(data);
        let hash2 = compute_content_hash(data);
        assert_eq!(hash1, hash2, "Hash must be deterministic");
    }

    #[test]
    fn test_different_inputs_different_hashes() {
        let hash1 = compute_content_hash("hello");
        let hash2 = compute_content_hash("world");
        assert_ne!(
            hash1, hash2,
            "Different inputs must produce different hashes"
        );
    }

    #[test]
    fn test_hash_format() {
        let hash = compute_content_hash("test");
        assert!(
            hash.starts_with("sha256:"),
            "Hash must start with 'sha256:'"
        );
        assert_eq!(
            hash.len(),
            64 + 7,
            "SHA-256 hex is 64 chars + 'sha256:' prefix"
        );
    }

    #[test]
    fn test_empty_content() {
        let hash = compute_content_hash("");
        assert!(hash.starts_with("sha256:"));
        assert!(!hash.is_empty());
    }

    #[test]
    fn test_matches_known_sha256_vector() {
        assert_eq!(
            compute_content_hash("abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
