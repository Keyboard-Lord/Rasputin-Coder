//! FORGE State Compression System
//!
//! Implements Canonical State Snapshot (CSS) transformation per
//! FORGE_STATE_COMPRESSION_SPEC.md
//!
//! Purpose: Distill rich AgentState into compact, 8B-model-compatible format
//! while preserving all decision-critical semantics.

use crate::state::AgentState;
use crate::types::{ExecutionMode, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Instant;

// ============================================================================
// CSS SCHEMA TYPES
// ============================================================================

/// Schema version for forward compatibility
pub const CSS_VERSION: u8 = 1;

/// Target token budget for 8B models
pub const DEFAULT_TOKEN_BUDGET: usize = 3072;

/// Canonical State Snapshot — compact planner-facing state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CssSnapshot {
    /// Schema version
    pub v: u8,
    /// Compact session ID
    pub s: String,
    /// Current iteration
    pub i: u32,
    /// Max iterations
    pub m: u32,
    /// Execution mode (3-char mnemonic)
    pub t: String,
    /// Compressed file state
    pub files: CssFileState,
    /// Execution history (last N entries)
    pub hist: Vec<String>,
    /// Error taxonomy (last N errors)
    pub err: Vec<String>,
    /// Decision metadata
    pub meta: CssMetadata,
}

/// Compressed file state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CssFileState {
    /// Read records: "stem_hash:is_full:content_hash_prefix"
    pub r: Vec<String>,
    /// Written file stems
    pub w: Vec<String>,
    /// Pending validation: "stem_hash:expected_hash:status"
    pub p: Vec<String>,
}

/// Decision metadata flags
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CssMetadata {
    /// Near iteration limit
    pub ni: bool,
    /// Recent errors (in last 3 iterations)
    pub re: bool,
    /// Pending reads required
    pub pr: bool,
    /// Pending completion (attempted but rejected)
    pub pc: bool,
    /// Stale reads detected
    pub st: bool,
}

/// Compression context for fine-tuning
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CompressionContext {
    /// Target max tokens
    pub target_tokens: usize,
    /// Max history entries
    pub history_window: usize,
    /// Max error entries
    pub error_window: usize,
    /// Include decision metadata
    pub include_meta: bool,
    /// Include content hash prefixes
    pub include_content_hashes: bool,
}

impl Default for CompressionContext {
    fn default() -> Self {
        Self {
            target_tokens: DEFAULT_TOKEN_BUDGET,
            history_window: 10,
            error_window: 5,
            include_meta: true,
            include_content_hashes: true,
        }
    }
}

/// CSS cache entry for avoiding recompression
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CssCacheEntry {
    pub css: CssSnapshot,
    pub state_hash: u64,
    pub created_at: Instant,
}

/// CSS cache with time-based invalidation
#[allow(dead_code)]
pub struct CssCache {
    entries: std::collections::HashMap<String, CssCacheEntry>,
    max_age: std::time::Duration,
}

#[allow(dead_code)]
impl CssCache {
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            max_age: std::time::Duration::from_secs(60), // 1 minute TTL
        }
    }

    /// Get cached CSS or return None if stale/missing
    pub fn get(&self, session_id: &SessionId, state_hash: u64) -> Option<CssSnapshot> {
        let key = session_id.to_string();
        let entry = self.entries.get(&key)?;

        // Check TTL and state hash match
        if entry.created_at.elapsed() < self.max_age && entry.state_hash == state_hash {
            Some(entry.css.clone())
        } else {
            None
        }
    }

    /// Insert CSS into cache
    pub fn insert(&mut self, session_id: &SessionId, state_hash: u64, css: CssSnapshot) {
        let key = session_id.to_string();
        self.entries.insert(
            key,
            CssCacheEntry {
                css,
                state_hash,
                created_at: Instant::now(),
            },
        );
    }

    /// Clean expired entries
    pub fn clean_expired(&mut self) {
        let now = Instant::now();
        self.entries
            .retain(|_, entry| now.duration_since(entry.created_at) < self.max_age);
    }
}

// ============================================================================
// CSS TRANSFORMER
// ============================================================================

/// Transforms runtime state to Canonical State Snapshot
#[allow(dead_code)]
pub struct CssTransformer {
    cache: CssCache,
}

#[allow(dead_code)]
impl CssTransformer {
    pub fn new() -> Self {
        Self {
            cache: CssCache::new(),
        }
    }

    /// Compress AgentState to CSS
    pub fn compress(&mut self, state: &AgentState, ctx: &CompressionContext) -> CssSnapshot {
        let state_hash = compute_state_hash(state);

        // Check cache
        if let Some(cached) = self.cache.get(&state.session_id, state_hash) {
            return cached;
        }

        let css = CssSnapshot {
            v: CSS_VERSION,
            s: compress_session_id(&state.session_id),
            i: state.iteration,
            m: state.max_iterations,
            t: compress_mode(state.mode),
            files: self.compress_files(state, ctx),
            hist: self.compress_history(state, ctx),
            err: self.compress_errors(state, ctx),
            meta: self.compute_metadata(state),
        };

        // Cache and return
        self.cache
            .insert(&state.session_id, state_hash, css.clone());
        css
    }

    /// Compress file state
    fn compress_files(&self, state: &AgentState, ctx: &CompressionContext) -> CssFileState {
        let mut files = CssFileState::default();

        // Compress read records
        for (path, record) in &state.files_read {
            let stem_hash = compress_path(path);
            let is_full = if record.is_full_read { "1" } else { "0" };
            let content_prefix = if ctx.include_content_hashes {
                &record.content_hash[..8.min(record.content_hash.len())]
            } else {
                ""
            };

            files
                .r
                .push(format!("{}:{}:{}", stem_hash, is_full, content_prefix));
        }

        // Compress written files
        for path in &state.files_written {
            files.w.push(compress_path(path));
        }

        files
    }

    /// Compress execution history
    fn compress_history(&self, state: &AgentState, ctx: &CompressionContext) -> Vec<String> {
        state
            .change_history
            .iter()
            .rev()
            .take(ctx.history_window)
            .map(|record| {
                let action_code = action_code_for_mutation(&record.mutation.mutation_type);
                let tool_name = mutation_type_to_string(&record.mutation.mutation_type);
                let outcome = if matches!(
                    record.validation_report.decision,
                    crate::types::ValidationDecision::Accept
                ) {
                    "1"
                } else {
                    "0"
                };

                // Include error code on failure
                if outcome == "0" {
                    let error_code = compress_error_message(&record.validation_report.message);
                    format!("{}:{}:{}:{}", action_code, tool_name, outcome, error_code)
                } else {
                    format!("{}:{}:{}", action_code, tool_name, outcome)
                }
            })
            .collect()
    }

    /// Compress errors
    fn compress_errors(&self, state: &AgentState, ctx: &CompressionContext) -> Vec<String> {
        // Extract errors from recent change history
        state
            .change_history
            .iter()
            .rev()
            .take(ctx.error_window * 2) // Scan more, filter
            .filter(|r| {
                matches!(
                    r.validation_report.decision,
                    crate::types::ValidationDecision::Reject
                )
            })
            .take(ctx.error_window)
            .map(|r| compress_error_message(&r.validation_report.message))
            .collect()
    }

    /// Compute decision metadata
    fn compute_metadata(&self, state: &AgentState) -> CssMetadata {
        let near_limit = state.iteration > (state.max_iterations as f32 * 0.7) as u32;

        let recent_errors = state.change_history.iter().rev().take(3).any(|r| {
            matches!(
                r.validation_report.decision,
                crate::types::ValidationDecision::Reject
            )
        });

        CssMetadata {
            ni: near_limit,
            re: recent_errors,
            pr: !state.pending_validations.is_empty(),
            pc: state.cardinality_violations > 0,
            st: state.has_hash_mismatch,
        }
    }

    /// Estimate token count
    pub fn estimate_tokens(&self, css: &CssSnapshot) -> usize {
        // Conservative: ~4 chars per token
        let json = serde_json::to_string(css).unwrap_or_default();
        json.len() / 4
    }

    /// Force recompression (bypass cache)
    pub fn force_compress(&self, state: &AgentState, ctx: &CompressionContext) -> CssSnapshot {
        // Bypass cache - same logic as compress but without cache lookup
        CssSnapshot {
            v: CSS_VERSION,
            s: compress_session_id(&state.session_id),
            i: state.iteration,
            m: state.max_iterations,
            t: compress_mode(crate::types::ExecutionMode::Edit),
            files: self.compress_files(state, ctx),
            hist: self.compress_history(state, ctx),
            err: self.compress_errors(state, ctx),
            meta: self.compute_metadata(state),
        }
    }
}

impl Default for CssTransformer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// COMPRESSION FUNCTIONS
// ============================================================================

/// Compress session ID to compact form
#[allow(dead_code)]
fn compress_session_id(session_id: &SessionId) -> String {
    let full = session_id.to_string();
    // Take "forge-" prefix and first 8 chars of ID
    if full.starts_with("forge-") && full.len() > 14 {
        full[..14].to_string()
    } else {
        full[..full.len().min(12)].to_string()
    }
}

/// Compress execution mode to 3-char mnemonic
#[allow(dead_code)]
fn compress_mode(mode: ExecutionMode) -> String {
    match mode {
        ExecutionMode::Analysis => "ana",
        ExecutionMode::Edit => "edi",
        ExecutionMode::Fix => "fix",
        ExecutionMode::Batch => "bat",
    }
    .to_string()
}

/// Compress path to 6-char stem hash
#[allow(dead_code)]
fn compress_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // Use DefaultHasher for speed (not cryptographic, but collision-resistant enough)
    let mut hasher = DefaultHasher::new();
    stem.hash(&mut hasher);
    let hash = hasher.finish();

    // Take first 3 bytes (6 hex chars)
    format!("{:06x}", hash & 0xFFFFFF)
}

/// Get action code for mutation type
#[allow(dead_code)]
fn action_code_for_mutation(mutation_type: &crate::types::MutationType) -> &'static str {
    use crate::types::MutationType;
    match mutation_type {
        MutationType::Write => "w",
        MutationType::Patch => "p",
        MutationType::Delete => "d",
        MutationType::Move => "m",
    }
}

/// Convert mutation type to string
#[allow(dead_code)]
fn mutation_type_to_string(mutation_type: &crate::types::MutationType) -> String {
    use crate::types::MutationType;
    match mutation_type {
        MutationType::Write => "write_file",
        MutationType::Patch => "apply_patch",
        MutationType::Delete => "delete_file",
        MutationType::Move => "move_file",
    }
    .to_string()
}

/// Compress error message to taxonomy code
#[allow(dead_code)]
fn compress_error_message(error: &str) -> String {
    let lower = error.to_lowercase();

    if lower.contains("hash mismatch") {
        "PATCH_HASH_MISMATCH"
    } else if lower.contains("text not found") || lower.contains("not found in") {
        "PATCH_TEXT_NOT_FOUND"
    } else if lower.contains("multiple") && lower.contains("occurrence") {
        "PATCH_MULTI_OCCUR"
    } else if lower.contains("file not found") || lower.contains("not exist") {
        "FILE_NOT_FOUND"
    } else if lower.contains("permission") {
        "PERM_DENIED"
    } else if lower.contains("not read") {
        "FILE_NOT_READ"
    } else if lower.contains("not fully read") {
        "FILE_NOT_FULLY_READ"
    } else if lower.contains("validation") {
        "VALIDATION_FAILED"
    } else if lower.contains("iteration") && lower.contains("limit") {
        "ITERATION_LIMIT"
    } else {
        "UNKNOWN_ERROR"
    }
    .to_string()
}

/// Compute hash of state for cache invalidation
#[allow(dead_code)]
fn compute_state_hash(state: &AgentState) -> u64 {
    let mut hasher = DefaultHasher::new();
    state.session_id.to_string().hash(&mut hasher);
    state.iteration.hash(&mut hasher);
    state.files_read.len().hash(&mut hasher);
    state.files_written.len().hash(&mut hasher);
    state.change_history.len().hash(&mut hasher);
    hasher.finish()
}

// ============================================================================
// 8B MODEL DISTILLED PROMPT
// ============================================================================

/// Returns the distilled system prompt for 8B models
pub fn distilled_8b_prompt() -> String {
    r#"You are Forge-8B, a deterministic planning agent.

STATE FORMAT: Compact JSON with codes. Read carefully.

DECISION RULES:
1. One action per turn. No narration.
2. READ before WRITE. Check state.files.r vs state.files.w.
3. Near limit (state.meta.ni=true)? Prioritize completion.
4. Recent errors (state.meta.re=true)? Retry with fix.

OUTPUT FORMAT (exactly one):
{"type":"tool_call","tool_call":{"name":"TOOL","arguments":{}}}
{"type":"completion","reason":"specific state evidence"}
{"type":"failure","reason":"observable blocker","recoverable":true/false}

TOOL AVAILABILITY:
- read_file: Always available
- write_file: Not in state.files.r → must read first
- apply_patch: Requires: (1) in state.files.r, (2) hash match

ERROR CODES YOU MAY SEE:
- PATCH_HASH_MISMATCH: File changed after read. Re-read.
- FILE_NOT_FOUND: Create file first if needed.
- TEXT_NOT_FOUND: Old text doesn't exist. Verify.

COMPLETION RULES:
- Only when state.meta.pr=false (no pending reads)
- Cite specific file hashes from state.files.r
- Never complete on iteration 0 without action."#
        .to_string()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_path_compression() {
        let path = PathBuf::from("src/main.rs");
        let compressed = compress_path(&path);
        assert_eq!(compressed.len(), 6);
        assert!(compressed.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_path_compression_deterministic() {
        let path = PathBuf::from("src/main.rs");
        let c1 = compress_path(&path);
        let c2 = compress_path(&path);
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_mode_compression() {
        assert_eq!(compress_mode(ExecutionMode::Edit), "edi");
        assert_eq!(compress_mode(ExecutionMode::Analysis), "ana");
    }

    #[test]
    fn test_error_compression() {
        assert_eq!(
            compress_error_message("Hash mismatch detected"),
            "PATCH_HASH_MISMATCH"
        );
        assert_eq!(
            compress_error_message("File not found: foo.txt"),
            "FILE_NOT_FOUND"
        );
    }

    #[test]
    fn test_css_snapshot_json() {
        let css = CssSnapshot {
            v: 1,
            s: "forge-abc123".to_string(),
            i: 5,
            m: 10,
            t: "edi".to_string(),
            files: CssFileState {
                r: vec!["abc123:1:8f3d2a9b".to_string()],
                w: vec!["def456".to_string()],
                p: vec![],
            },
            hist: vec!["r:read_file:1".to_string()],
            err: vec![],
            meta: CssMetadata {
                ni: true,
                re: false,
                pr: true,
                pc: false,
                st: false,
            },
        };

        let json = serde_json::to_string(&css).unwrap();
        assert!(json.contains("\"v\":1"));
        assert!(json.contains("forge-abc123"));

        // Verify round-trip
        let recovered: CssSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(css, recovered);
    }

    #[test]
    fn test_css_token_estimate() {
        let transformer = CssTransformer::new();
        let css = CssSnapshot {
            v: 1,
            s: "forge-abc123".to_string(),
            i: 5,
            m: 10,
            t: "edi".to_string(),
            files: CssFileState::default(),
            hist: vec![],
            err: vec![],
            meta: CssMetadata::default(),
        };

        let tokens = transformer.estimate_tokens(&css);
        assert!(tokens > 0);
        assert!(tokens < 1000); // Should be very small
    }
}
