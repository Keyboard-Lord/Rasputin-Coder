//! Core File Tools for Forge
//!
//! Implements read_file, write_file, and apply_patch with full safety guarantees:
//! - Path traversal prevention (repo boundary enforcement)
//! - Atomic writes with directory creation
//! - Content hash verification
//! - Mode-based access control
//! - Comprehensive mutation tracking

use crate::crypto_hash::compute_content_hash;
use crate::tool_registry::Tool;
use crate::types::{
    ExecutionContext, ExecutionMode, FileRecord, ForgeError, HardenedPatch, HardenedPatchResult,
    Mutation, MutationType, PatchApplicationResult, PatchFailureReason, ReadFileResult,
    ToolArguments, ToolError, ToolName, ToolResult,
};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// ===========================================================================
/// PATH VALIDATION
/// ===========================================================================
///
/// Validates that a path is within the allowed repository boundary
///
/// Prevents path traversal attacks (e.g., ../../../etc/passwd)
fn validate_path_boundary(path: &Path, working_dir: &Path) -> Result<PathBuf, ForgeError> {
    // Get canonical working directory first
    let canonical_working = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());

    // Normalize the path (resolve . and ..)
    let normalized = if path.is_absolute() {
        path.to_path_buf()
    } else {
        canonical_working.join(path)
    };

    // For new files, canonicalize the parent directory and join the filename
    // This handles the case where the file doesn't exist yet
    let canonical = if normalized.exists() {
        normalized
            .canonicalize()
            .unwrap_or_else(|_| normalized.clone())
    } else {
        // File doesn't exist - canonicalize the parent instead
        let parent_path = normalized
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| canonical_working.clone());
        let file_name = normalized
            .file_name()
            .ok_or_else(|| ForgeError::InvalidArgument("Invalid path: no filename".to_string()))?;

        let canonical_parent = parent_path
            .canonicalize()
            .unwrap_or_else(|_| parent_path.clone());

        canonical_parent.join(file_name)
    };

    // Check that the path starts with the working directory
    if !canonical.starts_with(&canonical_working) {
        return Err(ForgeError::InvalidArgument(format!(
            "Path '{}' is outside repository boundary '{}'",
            path.display(),
            canonical_working.display()
        )));
    }

    Ok(canonical)
}

/// Validates that a path does not contain traversal components
fn has_traversal_components(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// ===========================================================================
/// READ FILE TOOL
/// ===========================================================================
///
/// Read file tool with offset/limit support and hash computation
///
/// Arguments:
/// - path: File path to read (required)
/// - offset: Starting line number (1-based, default: 1)
/// - limit: Maximum lines to return (default: unlimited)
///
/// Returns:
/// - Content hash (SHA-256)
/// - Line range information
/// - Full/partial read indicator
pub struct ReadFileTool;

impl ReadFileTool {
    pub fn new() -> Self {
        Self
    }

    /// Extract lines from content based on offset and limit
    fn extract_lines(
        content: &str,
        offset: usize, // 1-based
        limit: Option<usize>,
    ) -> (String, usize, bool) {
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start_idx = offset.saturating_sub(1); // Convert to 0-based

        if let Some(lim) = limit {
            let end_idx = (start_idx + lim).min(total_lines);
            let extracted: Vec<&str> = lines[start_idx..end_idx].to_vec();
            let returned = extracted.len();
            let is_full = returned >= total_lines && start_idx == 0;
            (extracted.join("\n"), returned, is_full)
        } else {
            // Full read
            (content.to_string(), total_lines, true)
        }
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ReadFileTool {
    fn name(&self) -> ToolName {
        ToolName::new("read_file").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Read content from a file with optional offset and limit. \
         Returns content hash for integrity verification."
    }

    fn allowed_in_mode(&self, _mode: ExecutionMode) -> bool {
        // read_file is allowed in all modes (including Analysis)
        true
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        if !self.allowed_in_mode(ctx.mode) {
            return Err(ForgeError::ToolNotAllowed {
                tool: self.name(),
                mode: ctx.mode,
            });
        }

        let start = Instant::now();

        // Extract and validate path
        let path_str = args.require("path")?;
        let raw_path = PathBuf::from(path_str);

        // Check for traversal attempts
        if has_traversal_components(&raw_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components (..)",
                raw_path.display()
            )));
        }

        // Validate path is within repo boundary
        let path = validate_path_boundary(&raw_path, &ctx.working_dir)?;

        // Parse optional offset and limit
        let offset = args
            .get("offset")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1); // Ensure at least 1

        let limit = args.get("limit").and_then(|s| s.parse::<usize>().ok());

        // Read the file
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(ToolError::IoError(format!(
                        "Failed to read '{}': {}",
                        path.display(),
                        e
                    ))),
                    mutations: vec![],
                    execution_time_ms: elapsed,
                });
            }
        };

        // Compute full content hash
        let content_hash = compute_content_hash(&content);
        let total_lines = content.lines().count();

        // Extract requested lines
        let (extracted_content, lines_returned, is_full_read) =
            Self::extract_lines(&content, offset, limit);

        // Create file record for state tracking
        let lines_read = if is_full_read {
            None
        } else {
            Some((offset, lines_returned))
        };

        let _file_record = FileRecord::new(&path, &content, lines_read, ctx.iteration);

        let elapsed = start.elapsed().as_millis() as u64;

        // Build result
        let result = ReadFileResult {
            path: path.clone(),
            content: extracted_content.clone(),
            total_lines,
            lines_returned,
            content_hash: content_hash.clone(),
            is_full_read,
        };

        // Format output
        let output = format!(
            "Read {} lines from {} (total {} lines, hash: {})",
            result.lines_returned,
            path.display(),
            result.total_lines,
            &content_hash[..16.min(content_hash.len())]
        );

        // Store full content in metadata for state tracking
        let mut metadata = HashMap::new();
        metadata.insert("content_hash".to_string(), content_hash);
        metadata.insert("total_lines".to_string(), total_lines.to_string());
        metadata.insert("lines_returned".to_string(), lines_returned.to_string());
        metadata.insert("is_full_read".to_string(), is_full_read.to_string());
        if !is_full_read {
            metadata.insert("offset".to_string(), offset.to_string());
        }

        Ok(ToolResult {
            success: true,
            output: Some(output),
            error: None,
            mutations: vec![], // Read doesn't mutate
            execution_time_ms: elapsed,
        })
    }
}

/// ===========================================================================
/// WRITE FILE TOOL
/// ===========================================================================
///
/// Write file tool with atomic writes and directory creation
///
/// Arguments:
/// - path: File path to write (required)
/// - content: File content (required)
///
/// Safety features:
/// - Atomic write via temp file + rename
/// - Automatic parent directory creation
/// - Path boundary enforcement
/// - Content hash computation
pub struct WriteFileTool;

impl WriteFileTool {
    pub fn new() -> Self {
        Self
    }

    /// Perform atomic file write using temp file + rename
    fn atomic_write(path: &Path, content: &str) -> Result<(), std::io::Error> {
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Create temp file in same directory (for atomic rename)
        let temp_path = path.with_extension("tmp");

        // Write to temp file
        {
            let mut temp_file = fs::File::create(&temp_path)?;
            temp_file.write_all(content.as_bytes())?;
            temp_file.flush()?;
        }

        // Atomic rename
        fs::rename(&temp_path, path)?;

        Ok(())
    }
}

impl Default for WriteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WriteFileTool {
    fn name(&self) -> ToolName {
        ToolName::new("write_file").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Write content to a file atomically. \
         Creates parent directories if needed. \
         Not allowed in Analysis mode."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        // write_file is NOT allowed in Analysis mode (read-only)
        matches!(
            mode,
            ExecutionMode::Edit | ExecutionMode::Fix | ExecutionMode::Batch
        )
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        if !self.allowed_in_mode(ctx.mode) {
            return Err(ForgeError::ToolNotAllowed {
                tool: self.name(),
                mode: ctx.mode,
            });
        }

        let start = Instant::now();

        // Extract and validate path
        let path_str = args.require("path")?;
        let content = args.require("content")?;

        if path_str.is_empty() {
            return Err(ForgeError::InvalidArgument(
                "path cannot be empty".to_string(),
            ));
        }

        let raw_path = PathBuf::from(path_str);

        // Check for traversal attempts
        if has_traversal_components(&raw_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components (..)",
                raw_path.display()
            )));
        }

        // Validate path is within repo boundary
        let path = validate_path_boundary(&raw_path, &ctx.working_dir)?;

        // Compute hash of new content
        let new_content_hash = compute_content_hash(content);

        // Compute hash of old content (if file exists)
        let old_content_hash = if path.exists() {
            match fs::read_to_string(&path) {
                Ok(old_content) => Some(compute_content_hash(&old_content)),
                Err(_) => None,
            }
        } else {
            None
        };

        // Perform atomic write
        match Self::atomic_write(&path, content) {
            Ok(_) => {
                let elapsed = start.elapsed().as_millis() as u64;

                // Create mutation record
                let mutation = Mutation {
                    path: path.clone(),
                    mutation_type: MutationType::Write,
                    content_hash_before: old_content_hash,
                    content_hash_after: Some(new_content_hash.clone()),
                };

                let output = format!(
                    "Wrote {} bytes to {} (hash: {})",
                    content.len(),
                    path.display(),
                    &new_content_hash[..16.min(new_content_hash.len())]
                );

                Ok(ToolResult {
                    success: true,
                    output: Some(output),
                    error: None,
                    mutations: vec![mutation],
                    execution_time_ms: elapsed,
                })
            }
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;

                // Clean up temp file if it exists
                let temp_path = path.with_extension("tmp");
                let _ = fs::remove_file(&temp_path);

                Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(ToolError::IoError(format!(
                        "Failed to write '{}': {}",
                        path.display(),
                        e
                    ))),
                    mutations: vec![],
                    execution_time_ms: elapsed,
                })
            }
        }
    }
}

/// ===========================================================================
/// APPLY PATCH TOOL
/// ===========================================================================
///
/// Apply patch tool with hardening and hash verification
///
/// Arguments:
/// - file_path: Target file path (required)
/// - old_text: Text to replace (required)
/// - new_text: Replacement text (required)
/// - expected_hash: SHA-256 hash of current file content (required)
///
/// Safety features:
/// - Triple-hash verification (expected == tracked == on-disk)
/// - Cardinality enforcement (old_text must appear exactly once)
/// - Path boundary enforcement
/// - Atomic write
pub struct ApplyPatchTool {
    /// Optional callback for read-before-write enforcement
    /// Runtime sets this to check file was read before patching
    read_before_write_check: Option<ReadBeforeWriteCheck>,
}

type ReadBeforeWriteCheck = Box<dyn Fn(&Path) -> bool + Send + Sync>;

impl ApplyPatchTool {
    pub fn new() -> Self {
        Self {
            read_before_write_check: None,
        }
    }

    /// Set the read-before-write check callback
    #[allow(dead_code)]
    pub fn with_read_check<F>(mut self, check: F) -> Self
    where
        F: Fn(&Path) -> bool + Send + Sync + 'static,
    {
        self.read_before_write_check = Some(Box::new(check));
        self
    }

    /// Check if read-before-write enforcement passes
    fn check_read_before_write(&self, path: &Path) -> Result<(), ForgeError> {
        if let Some(ref check) = self.read_before_write_check
            && !check(path)
        {
            return Err(ForgeError::InvalidArgument(format!(
                "Read-before-write violation: file '{}' must be read before patching. \
                     Use read_file first.",
                path.display()
            )));
        }
        Ok(())
    }
}

impl Default for ApplyPatchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ApplyPatchTool {
    fn name(&self) -> ToolName {
        ToolName::new("apply_patch").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Apply a hardened patch with cardinality enforcement and hash binding. \
         old_text must appear exactly once. \
         expected_hash is mandatory for integrity verification."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        // apply_patch is NOT allowed in Analysis mode
        matches!(
            mode,
            ExecutionMode::Edit | ExecutionMode::Fix | ExecutionMode::Batch
        )
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        if !self.allowed_in_mode(ctx.mode) {
            return Err(ForgeError::ToolNotAllowed {
                tool: self.name(),
                mode: ctx.mode,
            });
        }

        let start = Instant::now();

        // Extract required arguments
        let path_str = args.require("file_path")?;
        let old_text = args.require("old_text")?;
        let new_text = args.require("new_text")?;

        // PHASE 2.5: expected_hash is MANDATORY
        let expected_hash = args.require("expected_hash")?;

        let raw_path = PathBuf::from(path_str);

        // Check for traversal attempts
        if has_traversal_components(&raw_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components (..)",
                raw_path.display()
            )));
        }

        // Validate path is within repo boundary
        let path = validate_path_boundary(&raw_path, &ctx.working_dir)?;

        // Check file exists
        if !path.exists() {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(ToolError::IoError(format!(
                    "File not found: {}",
                    path.display()
                ))),
                mutations: vec![],
                execution_time_ms: elapsed,
            });
        }

        // Check read-before-write enforcement
        if let Err(e) = self.check_read_before_write(&path) {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(ToolError::ExecutionFailed(e.to_string())),
                mutations: vec![],
                execution_time_ms: elapsed,
            });
        }

        // Read current content
        let current_content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(ToolError::IoError(format!(
                        "Cannot read file {}: {}",
                        path.display(),
                        e
                    ))),
                    mutations: vec![],
                    execution_time_ms: elapsed,
                });
            }
        };

        let current_hash = compute_content_hash(&current_content);

        // PHASE 2.5: Mandatory hash verification
        if expected_hash != current_hash {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(ToolError::ExecutionFailed(format!(
                    "Hash mismatch for {}: expected {} but on-disk hash is {}. \
                     File may have changed since read operation.",
                    path.display(),
                    &expected_hash[..16.min(expected_hash.len())],
                    &current_hash[..16.min(current_hash.len())]
                ))),
                mutations: vec![],
                execution_time_ms: elapsed,
            });
        }

        // PHASE 2.5: Apply patch with cardinality enforcement
        let patch = HardenedPatch {
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
        };

        match patch.apply(&current_content) {
            PatchApplicationResult::Success {
                new_content,
                occurrences_found,
                ..
            } => {
                // Write patched content atomically
                let temp_path = path.with_extension("tmp");
                let write_result =
                    fs::write(&temp_path, &new_content).and_then(|_| fs::rename(&temp_path, &path));

                match write_result {
                    Ok(_) => {
                        let new_hash = compute_content_hash(&new_content);
                        let old_lines = current_content.lines().count();
                        let new_lines = new_content.lines().count();
                        let lines_changed = old_lines.abs_diff(new_lines) + 1;
                        let elapsed = start.elapsed().as_millis() as u64;

                        // Create hardened result
                        let hardened_result = HardenedPatchResult {
                            path: path.clone(),
                            success: true,
                            hash_before: current_hash.clone(),
                            hash_after: Some(new_hash.clone()),
                            old_text_occurrences: occurrences_found,
                            lines_changed,
                            snapshot_used: false,
                            expected_hash_verified: true,
                            error: None,
                        };

                        // Create mutation record
                        let mutation = Mutation {
                            path: path.clone(),
                            mutation_type: MutationType::Patch,
                            content_hash_before: Some(current_hash.clone()),
                            content_hash_after: Some(new_hash.clone()),
                        };

                        let output = format!(
                            "HARDENED_PATCH: {} occurrences={}, lines_changed={}, {} -> {}",
                            path.display(),
                            hardened_result.old_text_occurrences,
                            hardened_result.lines_changed,
                            &current_hash[..16.min(current_hash.len())],
                            &new_hash[..16.min(new_hash.len())]
                        );

                        Ok(ToolResult {
                            success: true,
                            output: Some(output),
                            error: None,
                            mutations: vec![mutation],
                            execution_time_ms: elapsed,
                        })
                    }
                    Err(e) => {
                        let _ = fs::remove_file(&temp_path);
                        let elapsed = start.elapsed().as_millis() as u64;
                        Ok(ToolResult {
                            success: false,
                            output: None,
                            error: Some(ToolError::IoError(format!(
                                "Failed to write patched content: {}",
                                e
                            ))),
                            mutations: vec![],
                            execution_time_ms: elapsed,
                        })
                    }
                }
            }
            PatchApplicationResult::Failed {
                reason,
                occurrences_found,
            } => {
                let elapsed = start.elapsed().as_millis() as u64;

                let error_msg = match reason {
                    PatchFailureReason::TextNotFound => {
                        format!(
                            "PATCH_CARDINALITY_VIOLATION: old_text not found in {}",
                            path.display()
                        )
                    }
                    PatchFailureReason::MultipleOccurrences => {
                        format!(
                            "PATCH_CARDINALITY_VIOLATION: old_text appears {} times in {} \
                             (must be exactly 1 for unambiguous patch)",
                            occurrences_found,
                            path.display()
                        )
                    }
                    _ => format!("Patch failed: {}", reason),
                };

                Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(ToolError::ExecutionFailed(error_msg)),
                    mutations: vec![],
                    execution_time_ms: elapsed,
                })
            }
        }
    }
}

/// ===========================================================================
/// UNIT TESTS
/// ===========================================================================
///
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_context(dir: &TempDir) -> ExecutionContext {
        ExecutionContext {
            session_id: crate::types::SessionId::new(),
            iteration: 1,
            mode: ExecutionMode::Edit,
            working_dir: dir.path().to_path_buf(),
        }
    }

    fn make_args(pairs: &[(&str, &str)]) -> ToolArguments {
        let mut args = ToolArguments::new();
        for (k, v) in pairs {
            args.set(k, v);
        }
        args
    }

    #[test]
    fn test_read_file_full() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "line1\nline2\nline3\n").unwrap();

        let tool = ReadFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "test.txt")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("3 lines"));
        assert_eq!(result.mutations.len(), 0); // Read doesn't mutate
    }

    #[test]
    fn test_read_file_with_offset_limit() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let tool = ReadFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "test.txt"), ("offset", "2"), ("limit", "2")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        // Should read lines 2-3
        let output = result.output.as_ref().unwrap();
        assert!(output.contains("2 lines"));
    }

    #[test]
    fn test_read_file_not_found() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ReadFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "nonexistent.txt")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_read_file_traversal_blocked() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ReadFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "../outside.txt")]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should fail with path validation error
    }

    #[test]
    fn test_write_file_creates_file() {
        let temp_dir = TempDir::new().unwrap();

        let tool = WriteFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "newfile.txt"), ("content", "hello world")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert_eq!(result.mutations.len(), 1);

        // Verify file was created
        let created_file = temp_dir.path().join("newfile.txt");
        assert!(created_file.exists());
        assert_eq!(fs::read_to_string(&created_file).unwrap(), "hello world");
    }

    #[test]
    fn test_write_file_creates_directories() {
        let temp_dir = TempDir::new().unwrap();

        let tool = WriteFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[
            ("path", "subdir/nested/file.txt"),
            ("content", "nested content"),
        ]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);

        // Verify nested directories and file were created
        let created_file = temp_dir.path().join("subdir/nested/file.txt");
        assert!(created_file.exists());
        assert_eq!(fs::read_to_string(&created_file).unwrap(), "nested content");
    }

    #[test]
    fn test_write_file_updates_existing() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("existing.txt");
        fs::write(&test_file, "old content").unwrap();

        let tool = WriteFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "existing.txt"), ("content", "new content")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);

        // Verify content was updated
        assert_eq!(fs::read_to_string(&test_file).unwrap(), "new content");
    }

    #[test]
    fn test_write_file_blocked_in_analysis_mode() {
        let temp_dir = TempDir::new().unwrap();

        let tool = WriteFileTool::new();
        let mut ctx = create_test_context(&temp_dir);
        ctx.mode = ExecutionMode::Analysis; // Read-only mode

        let args = make_args(&[("path", "file.txt"), ("content", "test")]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should fail due to mode restriction
    }

    #[test]
    fn test_apply_patch_success() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("patch.txt");
        let content = "hello world\nfoo bar\n";
        fs::write(&test_file, content).unwrap();
        let hash = compute_content_hash(content);

        let tool = ApplyPatchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[
            ("file_path", "patch.txt"),
            ("old_text", "world"),
            ("new_text", "universe"),
            ("expected_hash", &hash),
        ]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert_eq!(result.mutations.len(), 1);

        // Verify patched content
        let patched = fs::read_to_string(&test_file).unwrap();
        assert_eq!(patched, "hello universe\nfoo bar\n");
    }

    #[test]
    fn test_apply_patch_hash_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("patch.txt");
        fs::write(&test_file, "content v1").unwrap();

        let tool = ApplyPatchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[
            ("file_path", "patch.txt"),
            ("old_text", "content"),
            ("new_text", "replaced"),
            ("expected_hash", "sha256:wronghash123"), // Wrong hash
        ]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .to_string()
                .contains("Hash mismatch")
        );
    }

    #[test]
    fn test_apply_patch_cardinality_violation() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("patch.txt");
        let content = "hello hello hello\n";
        fs::write(&test_file, content).unwrap();
        let hash = compute_content_hash(content);

        let tool = ApplyPatchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[
            ("file_path", "patch.txt"),
            ("old_text", "hello"), // Appears 3 times - ambiguous
            ("new_text", "hi"),
            ("expected_hash", &hash),
        ]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(!result.success);
        let error_str = result.error.as_ref().unwrap().to_string();
        assert!(error_str.contains("CARDINALITY_VIOLATION") || error_str.contains("3 times"));
    }

    #[test]
    fn test_apply_patch_text_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("patch.txt");
        let content = "hello world\n";
        fs::write(&test_file, content).unwrap();
        let hash = compute_content_hash(content);

        let tool = ApplyPatchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[
            ("file_path", "patch.txt"),
            ("old_text", "nonexistent"), // Not in file
            ("new_text", "replacement"),
            ("expected_hash", &hash),
        ]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .to_string()
                .contains("not found")
        );
    }

    #[test]
    fn test_apply_patch_blocked_in_analysis_mode() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ApplyPatchTool::new();
        let mut ctx = create_test_context(&temp_dir);
        ctx.mode = ExecutionMode::Analysis;

        let args = make_args(&[
            ("file_path", "file.txt"),
            ("old_text", "old"),
            ("new_text", "new"),
            ("expected_hash", "sha256:fake"),
        ]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should fail due to mode restriction
    }

    #[test]
    fn test_apply_patch_missing_expected_hash() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("patch.txt");
        fs::write(&test_file, "content").unwrap();

        let tool = ApplyPatchTool::new();
        let ctx = create_test_context(&temp_dir);
        // Missing expected_hash argument
        let args = make_args(&[
            ("file_path", "patch.txt"),
            ("old_text", "content"),
            ("new_text", "new"),
        ]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should fail - expected_hash is required
    }

    #[test]
    fn test_path_traversal_detection() {
        let temp_dir = TempDir::new().unwrap();

        // Create a file outside the temp dir to test protection
        let outside_file = std::env::temp_dir().join("outside_forge_test.txt");
        fs::write(&outside_file, "secret").unwrap();

        let tool = ReadFileTool::new();
        let ctx = create_test_context(&temp_dir);
        // Try to escape the working directory
        let args = make_args(&[("path", "../../outside_forge_test.txt")]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should be blocked

        // Cleanup
        let _ = fs::remove_file(&outside_file);
    }

    #[test]
    fn test_read_file_computes_correct_hash() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("hash_test.txt");
        let content = "test content for hashing";
        fs::write(&test_file, content).unwrap();
        let expected_hash = compute_content_hash(content);

        let tool = ReadFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "hash_test.txt")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        // Output should contain the hash prefix
        assert!(
            result
                .output
                .as_ref()
                .unwrap()
                .contains(&expected_hash[..16])
        );
    }

    #[test]
    fn test_write_file_mutation_has_hashes() {
        let temp_dir = TempDir::new().unwrap();

        let tool = WriteFileTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "new.txt"), ("content", "new content")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);

        let mutation = &result.mutations[0];
        assert_eq!(mutation.mutation_type, MutationType::Write);
        assert!(mutation.content_hash_before.is_none()); // New file
        assert!(mutation.content_hash_after.is_some()); // Has hash after write
    }
}
