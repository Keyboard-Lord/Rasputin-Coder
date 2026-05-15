//! FORGE Phase 0: Core Tool Implementations
//!
//! This module provides the fundamental tools for Forge:
//! - read_file: Read file content with offset/limit and hash computation
//! - write_file: Atomic file writes with directory creation
//! - apply_patch: Hardened patching with expected_hash verification
//! - grep_search: Regex/literal search with context lines
//! - list_dir: Directory listing with recursion and filtering
//! - execute_command: Shell command execution with safety controls
//! - batch_tools: Batch operations for large-scale file processing (V2.5)
//!
//! All tools enforce:
//! - Mode restrictions via ExecutionContext
//! - Path boundaries (repo root containment)
//! - Content hash computation for integrity
//! - Proper mutation capture for validation

pub mod batch_tools;
pub mod browser_preview_tool;
pub mod code_intelligence_tools;
pub mod execute_command_tool;
pub mod file_tools;
pub mod search_tools;
