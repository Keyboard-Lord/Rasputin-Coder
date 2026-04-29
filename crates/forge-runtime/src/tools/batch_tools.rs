//! Batch Processing Tools (V2.5)
//!
//! Tools for handling multiple files and large-scale operations efficiently.
//! Provides progress tracking, checkpoint/resume, and bounded execution.

use crate::tool_registry::Tool;
use crate::types::{ExecutionContext, ExecutionMode, ForgeError, Mutation, MutationType, ToolArguments, ToolName, ToolResult};
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Maximum files per batch operation
pub const MAX_BATCH_SIZE: usize = 1000;

/// Progress tracking for batch operations
#[derive(Debug, Clone)]
pub struct BatchProgress {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub current_file: Option<String>,
    pub errors: Vec<(String, String)>, // (path, error_message)
}

impl BatchProgress {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            completed: 0,
            failed: 0,
            current_file: None,
            errors: vec![],
        }
    }

    pub fn percentage(&self) -> f32 {
        if self.total == 0 {
            return 100.0;
        }
        ((self.completed + self.failed) as f32 / self.total as f32) * 100.0
    }

    pub fn is_complete(&self) -> bool {
        self.completed + self.failed >= self.total
    }
}

/// Batch file reader - reads multiple files efficiently
pub struct BatchReadFilesTool;

impl Tool for BatchReadFilesTool {
    fn name(&self) -> ToolName {
        ToolName::new("batch_read_files").unwrap()
    }

    fn description(&self) -> &str {
        "Read multiple files in a batch operation. More efficient than individual read_file calls."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        matches!(mode, ExecutionMode::Analysis | ExecutionMode::Edit | ExecutionMode::Batch)
    }

    fn execute(&self, args: &ToolArguments, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError> {
        let paths_str = args.require("paths")?;
        let max_file_size: usize = args
            .get("max_file_size")
            .and_then(|s: &str| s.parse().ok())
            .unwrap_or(1024 * 1024); // 1MB default

        let paths: Vec<&str> = paths_str.split(',').map(|s| s.trim()).collect();

        if paths.len() > MAX_BATCH_SIZE {
            return Err(ForgeError::InvalidArgument(format!(
                "Too many files: {} (max: {})",
                paths.len(),
                MAX_BATCH_SIZE
            )));
        }

        let mut results = HashMap::new();
        let mut progress = BatchProgress::new(paths.len());
        let mut total_bytes = 0;

        for path_str in &paths {
            progress.current_file = Some(path_str.to_string());
            let full_path = ctx.working_dir.join(path_str);

            // Validate path boundary
            if !full_path.starts_with(&ctx.working_dir) {
                progress.failed += 1;
                progress.errors.push((
                    path_str.to_string(),
                    "Path outside working directory".to_string(),
                ));
                continue;
            }

            match fs::metadata(&full_path) {
                Ok(metadata) => {
                    if metadata.len() > max_file_size as u64 {
                        progress.failed += 1;
                        progress.errors.push((
                            path_str.to_string(),
                            format!("File too large: {} bytes", metadata.len()),
                        ));
                        continue;
                    }

                    match fs::read_to_string(&full_path) {
                        Ok(content) => {
                            total_bytes += content.len();
                            results.insert(path_str.to_string(), content);
                            progress.completed += 1;
                        }
                        Err(e) => {
                            progress.failed += 1;
                            progress.errors.push((
                                path_str.to_string(),
                                format!("Read error: {}", e),
                            ));
                        }
                    }
                }
                Err(e) => {
                    progress.failed += 1;
                    progress.errors.push((
                        path_str.to_string(),
                        format!("Metadata error: {}", e),
                    ));
                }
            }
        }

        // Build output
        let mut output = String::new();
        output.push_str(&format!(
            "Batch read complete: {}/{} files, {} bytes\n\n",
            progress.completed, progress.total, total_bytes
        ));

        for (path, content) in &results {
            output.push_str(&format!("=== {} ===\n", path));
            output.push_str(content);
            output.push_str("\n\n");
        }

        if !progress.errors.is_empty() {
            output.push_str("ERRORS:\n");
            for (path, error) in &progress.errors {
                output.push_str(&format!("  {}: {}\n", path, error));
            }
        }

        // Truncate if too large
        if output.len() > 500_000 {
            output.truncate(500_000);
            output.push_str("\n\n[... output truncated due to size ...]");
        }

        Ok(ToolResult {
            success: progress.failed == 0,
            output: Some(output),
            error: if progress.failed > 0 {
                Some(crate::types::ToolError::ExecutionFailed(
                    format!("{} files failed", progress.failed)
                ))
            } else {
                None
            },
            mutations: vec![],
            execution_time_ms: 0,
        })
    }
}

/// Batch file writer - writes multiple files efficiently
pub struct BatchWriteFilesTool;

impl Tool for BatchWriteFilesTool {
    fn name(&self) -> ToolName {
        ToolName::new("batch_write_files").unwrap()
    }

    fn description(&self) -> &str {
        "Write multiple files from a batch specification. Files are provided as JSON mapping."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        matches!(
            mode,
            ExecutionMode::Edit | ExecutionMode::Fix | ExecutionMode::Batch
        )
    }

    fn execute(&self, args: &ToolArguments, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError> {
        let files_json = args.require("files")?;

        let files: HashMap<String, String> = serde_json::from_str(files_json)
            .map_err(|e| ForgeError::InvalidArgument(format!("Invalid JSON: {}", e)))?;

        if files.len() > MAX_BATCH_SIZE {
            return Err(ForgeError::InvalidArgument(format!(
                "Too many files: {} (max: {})",
                files.len(),
                MAX_BATCH_SIZE
            )));
        }

        let mut progress = BatchProgress::new(files.len());
        let mut mutations = vec![];
        let mut total_bytes = 0;

        for (path_str, content) in files {
            progress.current_file = Some(path_str.clone());
            let full_path = ctx.working_dir.join(&path_str);

            // Validate path boundary
            if !full_path.starts_with(&ctx.working_dir) {
                progress.failed += 1;
                progress.errors.push((
                    path_str,
                    "Path outside working directory".to_string(),
                ));
                continue;
            }

            // Create parent directories if needed
            if let Some(parent) = full_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    progress.failed += 1;
                    progress.errors.push((
                        path_str.clone(),
                        format!("Failed to create directories: {}", e),
                    ));
                    continue;
                }
            }

            // Write file
            match fs::write(&full_path, &content) {
                Ok(_) => {
                    total_bytes += content.len();
                    progress.completed += 1;
                    mutations.push(Mutation {
                        path: full_path,
                        mutation_type: MutationType::Write,
                        content_hash_before: None,
                        content_hash_after: None,
                    });
                }
                Err(e) => {
                    progress.failed += 1;
                    progress.errors.push((
                        path_str,
                        format!("Write error: {}", e),
                    ));
                }
            }
        }

        let mut output = format!(
            "Batch write complete: {}/{} files, {} bytes written\n",
            progress.completed, progress.total, total_bytes
        );

        if !progress.errors.is_empty() {
            output.push_str("\nERRORS:\n");
            for (path, error) in &progress.errors {
                output.push_str(&format!("  {}: {}\n", path, error));
            }
        }

        Ok(ToolResult {
            success: progress.failed == 0,
            output: Some(output),
            error: if progress.failed > 0 {
                Some(crate::types::ToolError::ExecutionFailed(
                    format!("{} files failed to write", progress.failed)
                ))
            } else {
                None
            },
            mutations,
            execution_time_ms: 0,
        })
    }
}

/// Find and replace across multiple files
pub struct BatchReplaceTool;

impl Tool for BatchReplaceTool {
    fn name(&self) -> ToolName {
        ToolName::new("batch_replace").unwrap()
    }

    fn description(&self) -> &str {
        "Replace text across multiple files matching a pattern."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        matches!(
            mode,
            ExecutionMode::Edit | ExecutionMode::Fix | ExecutionMode::Batch
        )
    }

    fn execute(&self, args: &ToolArguments, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError> {
        let file_pattern = args.require("file_pattern")?;
        let old_text = args.require("old_text")?;
        let new_text = args.require("new_text")?;
        let max_files: usize = args
            .get("max_files")
            .and_then(|s: &str| s.parse().ok())
            .unwrap_or(100);

        // Find matching files
        let files = find_files_matching(&ctx.working_dir, file_pattern, max_files)?;

        if files.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: Some(format!("No files matching pattern: {}", file_pattern)),
                error: None,
                mutations: vec![],
                execution_time_ms: 0,
            });
        }

        let mut progress = BatchProgress::new(files.len());
        let mut mutations = vec![];
        let mut replacements_count = 0;

        for path in files {
            let path_str: String = path.to_string_lossy().into_owned();
            progress.current_file = Some(path_str.clone());

            match fs::read_to_string(&path) {
                Ok(content) => {
                    let new_content = content.replace(old_text, new_text);
                    let count = content.matches(old_text).count();

                    if count > 0 {
                        match fs::write(&path, new_content) {
                            Ok(_) => {
                                replacements_count += count;
                                progress.completed += 1;
                                mutations.push(Mutation {
                                    path: path.clone(),
                                    mutation_type: MutationType::Patch,
                                    content_hash_before: None,
                                    content_hash_after: None,
                                });
                            }
                            Err(e) => {
                                progress.failed += 1;
                                progress.errors.push((
                                    path_str,
                                    format!("Write error: {}", e),
                                ));
                            }
                        }
                    } else {
                        progress.completed += 1; // No match is still success
                    }
                }
                Err(e) => {
                    progress.failed += 1;
                    progress.errors.push((path_str, format!("Read error: {}", e)));
                }
            }
        }

        let output = format!(
            "Batch replace complete: {}/{} files processed, {} replacements made\n",
            progress.completed, progress.total, replacements_count
        );

        Ok(ToolResult {
            success: progress.failed == 0,
            output: Some(output),
            error: if progress.failed > 0 {
                Some(crate::types::ToolError::ExecutionFailed(
                    format!("{} files failed", progress.failed)
                ))
            } else {
                None
            },
            mutations,
            execution_time_ms: 0,
        })
    }
}

/// Directory sync tool - mirrors a directory structure
pub struct SyncDirectoryTool;

impl Tool for SyncDirectoryTool {
    fn name(&self) -> ToolName {
        ToolName::new("sync_directory").unwrap()
    }

    fn description(&self) -> &str {
        "Synchronize directory structure from a specification. Creates missing directories."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        matches!(
            mode,
            ExecutionMode::Edit | ExecutionMode::Fix | ExecutionMode::Batch
        )
    }

    fn execute(&self, args: &ToolArguments, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError> {
        let structure_json = args.require("structure")?;
        let base_path = args
            .get("base_path")
            .map(|s| ctx.working_dir.join(s))
            .unwrap_or_else(|| ctx.working_dir.clone());

        let structure: Vec<String> = serde_json::from_str(structure_json)
            .map_err(|e| ForgeError::InvalidArgument(format!("Invalid JSON: {}", e)))?;

        let mut created = 0;
        let mut errors = vec![];

        for dir_path in structure {
            let full_path = base_path.join(&dir_path);

            // Validate path
            if !full_path.starts_with(&ctx.working_dir) {
                errors.push(format!("{}: Path outside working directory", dir_path));
                continue;
            }

            match fs::create_dir_all(&full_path) {
                Ok(_) => created += 1,
                Err(e) => errors.push(format!("{}: {}", dir_path, e)),
            }
        }

        let output = format!(
            "Directory sync: {} directories created, {} errors",
            created,
            errors.len()
        );

        Ok(ToolResult {
            success: errors.is_empty(),
            output: Some(output),
            error: if errors.is_empty() {
                None
            } else {
                Some(crate::types::ToolError::ExecutionFailed(
                    errors.join("; ")
                ))
            },
            mutations: vec![],
            execution_time_ms: 0,
        })
    }
}

/// Helper: Find files matching a glob pattern
fn find_files_matching(
    base_dir: &Path,
    pattern: &str,
    max_files: usize,
) -> Result<Vec<PathBuf>, ForgeError> {
    let mut results = vec![];

    for entry in walkdir::WalkDir::new(base_dir)
        .max_depth(10)
        .into_iter()
        .filter_entry(|e| !is_hidden(e))
    {
        if results.len() >= max_files {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let relative_path = path.strip_prefix(base_dir).unwrap_or(path);
        let path_str = relative_path.to_string_lossy();

        // Simple pattern matching (can be enhanced with glob crate)
        if pattern == "*" || path_str.ends_with(pattern.trim_start_matches("*.")) {
            results.push(path.to_path_buf());
        }
    }

    Ok(results)
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_progress() {
        let mut progress = BatchProgress::new(100);
        assert_eq!(progress.percentage(), 0.0);
        assert!(!progress.is_complete());

        progress.completed = 50;
        assert_eq!(progress.percentage(), 50.0);

        progress.completed = 100;
        assert!(progress.is_complete());
    }
}
