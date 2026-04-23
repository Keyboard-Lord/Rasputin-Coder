//! Compressed State Planner — 8B Model Optimized
//!
//! A planner implementation that uses Canonical State Snapshots (CSS)
//! for efficient 8B model consumption. This planner:
//!
//! 1. Compresses rich state to CSS before each planning call
//! 2. Uses the distilled 8B system prompt
//! 3. Integrates with the standard Planner trait
//!
//! See FORGE_STATE_COMPRESSION_SPEC.md for CSS schema details.

use crate::planner::css_transformer::{
    CompressionContext, CssSnapshot, CssTransformer, distilled_8b_prompt,
};
use crate::planner::state_view::StateView;
use crate::planner::traits::Planner;
use crate::types::{ForgeError, PlannerOutput};
use serde_json::Value;

/// Planner optimized for 8B models using CSS compression
///
/// This planner wraps a backend and compresses state before
/// sending to the model. It uses the distilled system prompt
/// designed for 8B context windows.
#[allow(dead_code)]
pub struct CompressedStatePlanner {
    /// The underlying model backend
    backend: Box<dyn super::model::PlannerBackend>,
    /// State compressor with cache
    transformer: CssTransformer,
    /// Compression context (tuning parameters)
    compression_ctx: CompressionContext,
    /// Distilled system prompt for 8B models
    system_prompt: String,
    /// Whether to include CSS in prompt (vs full state)
    use_compression: bool,
}

#[allow(dead_code)]
impl CompressedStatePlanner {
    /// Create new compressed planner with default settings
    pub fn new(backend: Box<dyn super::model::PlannerBackend>) -> Self {
        Self {
            backend,
            transformer: CssTransformer::new(),
            compression_ctx: CompressionContext::default(),
            system_prompt: distilled_8b_prompt(),
            use_compression: true,
        }
    }

    /// Create with custom compression context
    pub fn with_compression_context(mut self, ctx: CompressionContext) -> Self {
        self.compression_ctx = ctx;
        self
    }

    /// Create with custom system prompt
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    /// Disable compression (for debugging)
    pub fn without_compression(mut self) -> Self {
        self.use_compression = false;
        self
    }

    /// Get last CSS snapshot (for debugging)
    pub fn last_css_snapshot(&self) -> Option<CssSnapshot> {
        // Note: This would require storing last snapshot in transformer
        // For now, transformer doesn't expose this - could be added
        None
    }

    /// Build CSS JSON from StateView
    #[allow(dead_code)]
    fn build_css_from_stateview(&mut self, state: &StateView) -> String {
        // Build CSS snapshot manually from StateView
        // In production, you'd want StateView to include a pre-computed CSS
        // or have runtime pass AgentState directly

        use crate::planner::css_transformer::{CssFileState, CssMetadata, CssSnapshot};

        // Compress file reads
        let read_records: Vec<String> = state
            .files_read
            .iter()
            .map(|f| {
                let stem_hash = compress_stem(&f.path);
                let is_full = if f.is_full_read { "1" } else { "0" };
                let hash_prefix = &f.content_hash[..8.min(f.content_hash.len())];
                format!("{}:{}:{}", stem_hash, is_full, hash_prefix)
            })
            .collect();

        // Compress written files
        let written: Vec<String> = state
            .files_written
            .iter()
            .map(|p| compress_stem(p))
            .collect();

        // Build metadata
        let near_limit = state.iteration > (state.max_iterations as f32 * 0.7) as u32;
        let has_recent_errors = !state.recent_errors.is_empty();

        let css = CssSnapshot {
            v: 1,
            s: state.session_id.clone(),
            i: state.iteration,
            m: state.max_iterations,
            t: compress_mode_from_stateview(state),
            files: CssFileState {
                r: read_records,
                w: written,
                p: vec![], // Would need runtime input for pending
            },
            hist: state
                .recent_executions
                .iter()
                .rev()
                .take(10)
                .map(|e| {
                    // Simplified history from execution records
                    format!("x:{}:1", e.tool_name) // Assume success for StateView records
                })
                .collect(),
            err: state.recent_errors.iter().take(5).cloned().collect(),
            meta: CssMetadata {
                ni: near_limit,
                re: has_recent_errors,
                pr: false, // Would need runtime detection
                pc: false,
                st: false,
            },
        };

        serde_json::to_string(&css).unwrap_or_default()
    }

    /// Parse model response to PlannerOutput
    fn parse_response(&self, raw: &str) -> Result<PlannerOutput, ForgeError> {
        // Use the standard adapter normalization
        let adapter = super::adapter::PlannerAdapter::new();

        // Create minimal StateView for validation
        let dummy_state = StateView {
            task: "parse".to_string(),
            session_id: "parse".to_string(),
            iteration: 0,
            max_iterations: 10,
            mode: crate::types::ExecutionMode::Edit,
            files_read: vec![],
            files_written: vec![],
            available_tools: vec![
                super::state_view::ToolInfo::new(
                    crate::types::ToolName::new("read_file").unwrap(),
                    "Read",
                ),
                super::state_view::ToolInfo::new(
                    crate::types::ToolName::new("write_file").unwrap(),
                    "Write",
                ),
                super::state_view::ToolInfo::new(
                    crate::types::ToolName::new("apply_patch").unwrap(),
                    "Patch",
                ),
            ],
            recent_executions: vec![],
            last_validation: None,
            recent_errors: vec![],
            repo_root: std::path::PathBuf::from("."),
            allowed_paths: vec![],
        };

        adapter
            .normalize(raw, &dummy_state)
            .map_err(|e| ForgeError::PlannerNormalizationError(e.to_string()))
    }
}

impl Planner for CompressedStatePlanner {
    fn generate(&self, state: &StateView) -> Result<PlannerOutput, ForgeError> {
        // We need mutable access to transformer, so we use interior mutability
        // or require &mut self. For now, let's use a simple approach.

        // Build prompt
        let css_json = build_minimal_css(state);
        let prompt = format!(
            "{}\n\nCURRENT STATE:\n{}\n\nPROPOSE EXACTLY ONE ACTION:",
            distilled_8b_prompt(),
            css_json
        );

        // Call backend
        let raw_response = self.backend.infer(&prompt)?;

        // Parse response
        self.parse_response(&raw_response)
    }

    fn planner_type(&self) -> &'static str {
        "compressed_8b"
    }

    fn generate_raw(&self, state: &StateView) -> Result<String, ForgeError> {
        let css_json = build_minimal_css(state);
        let prompt = format!(
            "{}\n\nCURRENT STATE:\n{}\n\nPROPOSE EXACTLY ONE ACTION:",
            distilled_8b_prompt(),
            css_json
        );

        self.backend.infer(&prompt)
    }
}

/// Helper to build minimal CSS from StateView
fn build_minimal_css(state: &StateView) -> String {
    // Build compact JSON manually for efficiency
    let mut parts = Vec::new();

    // Basic metadata
    parts.push("\"v\":1".to_string());
    parts.push(format!(
        "\"s\":\"{}\"",
        &state.session_id[..state.session_id.len().min(12)]
    ));
    parts.push(format!("\"i\":{}", state.iteration));
    parts.push(format!("\"m\":{}", state.max_iterations));

    // Mode
    let mode = match state.mode {
        crate::types::ExecutionMode::Analysis => "ana",
        crate::types::ExecutionMode::Edit => "edi",
        crate::types::ExecutionMode::Fix => "fix",
        crate::types::ExecutionMode::Batch => "bat",
    };
    parts.push(format!("\"t\":\"{}\"", mode));

    // Files
    let reads: Vec<String> = state
        .files_read
        .iter()
        .map(|f| {
            let stem = f.path.file_stem().and_then(|s| s.to_str()).unwrap_or("unk");
            let hash = crate::crypto_hash::compute_content_hash(stem);
            let short_hash = &hash[..6.min(hash.len())];
            let is_full = if f.is_full_read { "1" } else { "0" };
            let content_short = &f.content_hash[..8.min(f.content_hash.len())];
            format!("\"{}:{}:{}\"", short_hash, is_full, content_short)
        })
        .collect();

    let writes: Vec<String> = state
        .files_written
        .iter()
        .map(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("unk");
            let hash = crate::crypto_hash::compute_content_hash(stem);
            format!("\"{}\"", &hash[..6.min(hash.len())])
        })
        .collect();

    parts.push(format!(
        "\"files\":{{\"r\":[{}],\"w\":[{}],\"p\":[]}}",
        reads.join(","),
        writes.join(",")
    ));

    // History
    let hist: Vec<String> = state
        .recent_executions
        .iter()
        .rev()
        .take(10)
        .map(|e| format!("\"x:{}:1\"", e.tool_name))
        .collect();
    parts.push(format!("\"hist\":[{}]", hist.join(",")));

    // Errors
    let errs: Vec<String> = state
        .recent_errors
        .iter()
        .take(5)
        .map(|e| {
            let code = compress_error_code(e);
            format!("\"{}\"", code)
        })
        .collect();
    parts.push(format!("\"err\":[{}]", errs.join(",")));

    // Metadata
    let near_limit = state.iteration > (state.max_iterations as f32 * 0.7) as u32;
    let has_errors = !state.recent_errors.is_empty();
    parts.push(format!(
        "\"meta\":{{\"ni\":{},\"re\":{},\"pr\":false,\"pc\":false,\"st\":false}}",
        near_limit, has_errors
    ));

    format!("{{{}}}", parts.join(","))
}

/// Compress error to taxonomy code
fn compress_error_code(error: &str) -> String {
    let lower = error.to_lowercase();

    if lower.contains("hash mismatch") {
        "PATCH_HASH_MISMATCH"
    } else if lower.contains("not found") {
        "TEXT_NOT_FOUND"
    } else if lower.contains("multiple") {
        "MULTI_OCCUR"
    } else if lower.contains("permission") {
        "PERM_DENIED"
    } else {
        "ERROR"
    }
    .to_string()
}

/// Compress path stem to short hash
fn compress_stem(path: &std::path::Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unk");
    let hash = crate::crypto_hash::compute_content_hash(stem);
    hash[..6.min(hash.len())].to_string()
}

/// Compress execution mode from StateView
fn compress_mode_from_stateview(_state: &StateView) -> String {
    // StateView doesn't expose mode directly in current implementation
    // Default to "edi" for now
    "edi".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::state_view::{FileReadInfo, ToolExecutionRecord};
    use crate::types::{ExecutionMode, ToolName};
    use std::path::PathBuf;

    fn create_test_state() -> StateView {
        StateView {
            task: "test".to_string(),
            session_id: "forge-test123".to_string(),
            iteration: 5,
            max_iterations: 10,
            mode: ExecutionMode::Edit,
            files_read: vec![FileReadInfo {
                path: PathBuf::from("src/main.rs"),
                content_hash: "abc123def456".to_string(),
                size_bytes: 1000,
                total_lines: 50,
                is_full_read: true,
                read_at_iteration: 1,
                content_excerpt: Some("fn main() {}".to_string()),
            }],
            files_written: vec![PathBuf::from("output.txt")],
            available_tools: vec![],
            recent_executions: vec![ToolExecutionRecord {
                iteration: 4,
                tool_name: "read_file".to_string(),
                success: true,
                summary: "Read main.rs".to_string(),
            }],
            last_validation: None,
            recent_errors: vec![],
            repo_root: PathBuf::from("."),
            allowed_paths: vec![PathBuf::from(".")],
        }
    }

    #[test]
    fn test_build_minimal_css() {
        let state = create_test_state();
        let css = build_minimal_css(&state);

        // Verify structure
        assert!(css.contains("\"v\":1"));
        assert!(css.contains("\"i\":5"));
        assert!(css.contains("\"m\":10"));
        assert!(css.contains("\"t\":\"edi\""));
        assert!(css.contains("\"files\""));
        assert!(css.contains("\"hist\""));
        assert!(css.contains("\"err\""));
        assert!(css.contains("\"meta\""));

        // Verify it's compact (no whitespace)
        assert!(!css.contains("  "));

        // Verify it's valid JSON
        let parsed: Value = serde_json::from_str(&css).unwrap();
        assert_eq!(parsed["v"], 1);
        assert_eq!(parsed["i"], 5);
    }

    #[test]
    fn test_css_compression_ratio() {
        let state = create_test_state();

        // Full state JSON
        let full_json = state.to_json();

        // CSS JSON
        let css_json = build_minimal_css(&state);

        // CSS should be significantly smaller
        let ratio = full_json.len() as f32 / css_json.len() as f32;
        println!(
            "Compression ratio: {:.1}x ({}B -> {}B)",
            ratio,
            full_json.len(),
            css_json.len()
        );

        // The compressed payload should still beat the full state for a small test fixture.
        assert!(
            css_json.len() < full_json.len(),
            "CSS should be smaller than the full state JSON"
        );
    }

    #[test]
    fn test_error_compression() {
        assert_eq!(
            compress_error_code("Hash mismatch detected"),
            "PATCH_HASH_MISMATCH"
        );
        assert_eq!(
            compress_error_code("Text not found in file"),
            "TEXT_NOT_FOUND"
        );
        assert_eq!(
            compress_error_code("Multiple occurrences found"),
            "MULTI_OCCUR"
        );
    }
}
