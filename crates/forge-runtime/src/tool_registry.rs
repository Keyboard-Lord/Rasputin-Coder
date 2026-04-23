//! FORGE PHASE 1.5: Formal Tool Registry Contract
//!
//! Refactored to use strongly-typed contracts and formal execution boundaries.
//!
//! Key improvements:
//! - ToolName newtype prevents invalid tool names
//! - ToolArguments provides validated argument access
//! - ExecutionContext passed to all tools
//! - Registry owns registration, executor owns resolution
//! - Unknown tools fail closed with ForgeError

use crate::tools::browser_preview_tool::BrowserPreviewTool;
use crate::tools::code_intelligence_tools::{
    DependencyGraphTool, EntryPointDetectorTool, LintRunnerTool, SymbolIndexTool, TestRunnerTool,
};
use crate::tools::execute_command_tool::ExecuteCommandTool;
use crate::tools::file_tools::{ApplyPatchTool, ReadFileTool, WriteFileTool};
use crate::tools::search_tools::{GrepSearchTool, ListDirTool};
use crate::types::{
    ExecutionContext, ExecutionMode, FileRecord, ForgeError, ToolArguments, ToolCall, ToolName,
    ToolResult,
};
use std::collections::HashMap;
use std::path::PathBuf;

/// ===========================================================================
/// TOOL TRAIT - Formal contract for all tools
/// ===========================================================================
///
/// Tool trait - all tools must implement this interface
pub trait Tool: Send + Sync {
    /// Get the canonical name of this tool
    fn name(&self) -> ToolName;

    /// Get a description of what this tool does
    #[allow(dead_code)]
    fn description(&self) -> &str;

    /// Check if this tool is allowed in the given execution mode
    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        // Default: all tools allowed in all modes except Analysis
        mode != ExecutionMode::Analysis || self.name().as_str() == "read_file"
    }

    /// Execute the tool with the given arguments and context
    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError>;
}

/// ===========================================================================
/// TOOL REGISTRY - Registration and resolution
/// ===========================================================================
///
/// Registry of available tools
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new registry with all built-in tools registered
    pub fn new() -> Self {
        let mut tools: HashMap<String, Box<dyn Tool>> = HashMap::new();

        // Register file tools
        Self::register_tool(&mut tools, ReadFileTool::new());
        Self::register_tool(&mut tools, WriteFileTool::new());
        Self::register_tool(&mut tools, ApplyPatchTool::new());

        // Register search tools
        Self::register_tool(&mut tools, GrepSearchTool::new());
        Self::register_tool(&mut tools, ListDirTool::new());

        // Register bounded code intelligence tools
        Self::register_tool(&mut tools, DependencyGraphTool::new());
        Self::register_tool(&mut tools, SymbolIndexTool::new());
        Self::register_tool(&mut tools, EntryPointDetectorTool::new());
        Self::register_tool(&mut tools, LintRunnerTool::new());
        Self::register_tool(&mut tools, TestRunnerTool::new());

        // Register execution tool
        Self::register_tool(&mut tools, ExecuteCommandTool::new());

        // Register browser preview tool
        Self::register_tool(&mut tools, BrowserPreviewTool::new());

        Self { tools }
    }

    fn register_tool<T: Tool + 'static>(tools: &mut HashMap<String, Box<dyn Tool>>, tool: T) {
        let name = tool.name().as_str().to_string();
        tools.insert(name, Box::new(tool));
    }

    /// Resolve a tool by name
    /// Returns ForgeError::UnknownTool if tool not found (fail-closed)
    pub fn resolve(&self, name: &ToolName) -> Result<&dyn Tool, ForgeError> {
        match self.tools.get(name.as_str()) {
            Some(tool) => Ok(tool.as_ref()),
            None => Err(ForgeError::UnknownTool(name.clone())),
        }
    }

    /// Check if a tool exists in the registry
    #[allow(dead_code)]
    pub fn has_tool(&self, name: &ToolName) -> bool {
        self.tools.contains_key(name.as_str())
    }

    /// List all registered tool names
    #[allow(dead_code)]
    pub fn list_tools(&self) -> Vec<ToolName> {
        self.tools
            .keys()
            .filter_map(|name| ToolName::new(name).ok())
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// BUILT-IN TOOLS
// ===========================================================================
// Tools are now implemented in the tools module.

/// ===========================================================================
/// EXECUTOR - Tool execution with validation and context
/// ===========================================================================
///
/// Executor handles tool resolution and execution
pub struct ToolExecutor {
    registry: ToolRegistry,
}

impl ToolExecutor {
    pub fn new() -> Self {
        Self {
            registry: ToolRegistry::new(),
        }
    }

    /// Execute a tool call with full validation
    pub fn execute(
        &self,
        call: &ToolCall,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        // Stage 1: Resolve tool (fail-closed on unknown)
        let tool = self.registry.resolve(&call.name)?;

        // Stage 2: Check mode compatibility
        if !tool.allowed_in_mode(ctx.mode) {
            return Err(ForgeError::ToolNotAllowed {
                tool: call.name.clone(),
                mode: ctx.mode,
            });
        }

        // Stage 3: Execute with context
        tool.execute(&call.arguments, ctx)
    }

    /// Execute with read-before-write enforcement
    /// Returns (ToolResult, optional FileRecord for read_file)
    #[allow(dead_code)]
    pub fn execute_with_enforcement(
        &self,
        call: &ToolCall,
        ctx: &ExecutionContext,
        is_file_read: &dyn Fn(&PathBuf) -> bool,
        is_file_fully_read: &dyn Fn(&PathBuf) -> bool,
    ) -> Result<(ToolResult, Option<FileRecord>), ForgeError> {
        // Check apply_patch preconditions
        if call.name.as_str() == "apply_patch" {
            let path_str = call
                .arguments
                .get("file_path")
                .ok_or_else(|| ForgeError::MissingArgument("file_path".to_string()))?;
            let path = PathBuf::from(path_str);

            // Rule 1: File must have been read
            if !is_file_read(&path) {
                return Err(ForgeError::InvalidArgument(format!(
                    "Read-before-write violation: file {} must be read before patching",
                    path.display()
                )));
            }

            // Rule 2: File must be fully read (not partial)
            if !is_file_fully_read(&path) {
                return Err(ForgeError::InvalidArgument(format!(
                    "File {} was partially read - full read required for patching",
                    path.display()
                )));
            }
        }

        // Execute the tool
        let result = self.execute(call, ctx)?;

        // Extract FileRecord from read_file results
        let file_record = if call.name.as_str() == "read_file" && result.success {
            let path_str = call
                .arguments
                .get("path")
                .ok_or_else(|| ForgeError::MissingArgument("path".to_string()))?;
            let _path = PathBuf::from(path_str);

            // Reconstruct the file record (we'd need to read again to get full content)
            // In production, we'd cache this, but for now we'll let the runtime handle it
            None // Runtime will handle this separately
        } else {
            None
        };

        Ok((result, file_record))
    }

    /// Get reference to registry for introspection
    #[allow(dead_code)]
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }
}

impl Default for ToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// ===========================================================================
/// FILE OPERATIONS (for revert and cleanup)
/// ===========================================================================
///
/// Delete a file (used for revert operations)
pub fn delete_file(path: &PathBuf) -> Result<(), ForgeError> {
    use std::fs;
    if path.exists() {
        match fs::remove_file(path) {
            Ok(_) => {
                println!("  [REVERT] Deleted: {}", path.display());
                Ok(())
            }
            Err(e) => Err(ForgeError::IoError(format!(
                "Failed to delete {}: {}",
                path.display(),
                e
            ))),
        }
    } else {
        Ok(()) // Idempotent - file already gone
    }
}
