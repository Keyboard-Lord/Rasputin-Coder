//! Search Tools for Forge
//!
//! Implements grep_search and list_dir with full safety guarantees:
//! - Path traversal prevention (repo boundary enforcement)
//! - Regex support with safe pattern compilation
//! - Result limiting to prevent output explosion
//! - Recursive directory enumeration with depth limits
//! - Mode-based access control

use crate::tool_registry::Tool;
use crate::types::{ExecutionContext, ExecutionMode, ForgeError, ToolArguments, ToolResult};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// ===========================================================================
/// PATH VALIDATION (shared with file_tools)
/// ===========================================================================
///
/// Validates that a path is within the allowed repository boundary
fn validate_path_boundary(path: &Path, working_dir: &Path) -> Result<PathBuf, ForgeError> {
    let canonical_working = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());

    let normalized = if path.is_absolute() {
        path.to_path_buf()
    } else {
        canonical_working.join(path)
    };

    let canonical = if normalized.exists() {
        normalized
            .canonicalize()
            .unwrap_or_else(|_| normalized.clone())
    } else {
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
/// GREP SEARCH TOOL
/// ===========================================================================
///
/// Single match result from grep search
#[derive(Debug, Clone)]
pub struct GrepMatch {
    pub file_path: PathBuf,
    pub line_number: usize,
    pub content: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

/// Grep search tool with regex support and line context
///
/// Arguments:
/// - query: Search pattern (required)
/// - path: Directory or file to search (optional, defaults to working_dir)
/// - case_sensitive: Whether search is case-sensitive (default: true)
/// - max_results: Maximum number of matches to return (default: 50)
/// - file_pattern: Glob pattern to filter files (optional, e.g., "*.rs")
/// - context_lines: Number of context lines before/after match (default: 2)
/// - use_regex: Whether to treat query as regex (default: true)
///
/// Returns:
/// - List of matches with file path, line number, content, and context
pub struct GrepSearchTool;

impl GrepSearchTool {
    pub fn new() -> Self {
        Self
    }

    /// Check if file should be searched based on file_pattern
    fn matches_file_pattern(file_name: &str, pattern: Option<&str>) -> bool {
        match pattern {
            None => true,
            Some(pat) => {
                // Simple glob matching - support * and ?
                let regex_pattern = pat.replace(".", "\\.").replace("*", ".*").replace("?", ".");

                match regex::Regex::new(&format!("^{}$", regex_pattern)) {
                    Ok(re) => re.is_match(file_name),
                    Err(_) => file_name.contains(&pat.replace("*", "").replace("?", "")),
                }
            }
        }
    }

    /// Search a single file for matches
    fn search_file(
        &self,
        file_path: &Path,
        query: &str,
        case_sensitive: bool,
        context_lines: usize,
        use_regex: bool,
        max_results: &mut usize,
    ) -> Result<Vec<GrepMatch>, ForgeError> {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => return Ok(vec![]), // Binary or unreadable file - skip
        };

        let mut matches = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        // Compile regex if needed
        let regex = if use_regex {
            let pattern = if case_sensitive {
                query.to_string()
            } else {
                format!("(?i){}", query)
            };
            match regex::Regex::new(&pattern) {
                Ok(re) => Some(re),
                Err(e) => {
                    return Err(ForgeError::InvalidArgument(format!(
                        "Invalid regex pattern '{}': {}",
                        query, e
                    )));
                }
            }
        } else {
            None
        };

        for (idx, line) in lines.iter().enumerate() {
            let is_match = if let Some(ref re) = regex {
                re.is_match(line)
            } else {
                if case_sensitive {
                    line.contains(query)
                } else {
                    line.to_lowercase().contains(&query.to_lowercase())
                }
            };

            if is_match {
                let line_number = idx + 1;

                // Get context lines
                let start_ctx = idx.saturating_sub(context_lines);
                let end_ctx = (idx + context_lines + 1).min(lines.len());

                let context_before = lines[start_ctx..idx]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                let context_after = lines[idx + 1..end_ctx]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();

                matches.push(GrepMatch {
                    file_path: file_path.to_path_buf(),
                    line_number,
                    content: line.to_string(),
                    context_before,
                    context_after,
                });

                *max_results -= 1;
                if *max_results == 0 {
                    break;
                }
            }
        }

        Ok(matches)
    }

    /// Recursively collect searchable files
    fn collect_files(
        &self,
        dir: &Path,
        file_pattern: Option<&str>,
        max_files: usize,
    ) -> Result<Vec<PathBuf>, ForgeError> {
        let mut files = Vec::new();
        let mut entries = vec![dir.to_path_buf()];

        while let Some(current) = entries.pop() {
            if files.len() >= max_files {
                break;
            }

            let read_dir = match fs::read_dir(&current) {
                Ok(rd) => rd,
                Err(_) => continue, // Skip directories we can't read
            };

            for entry in read_dir.flatten() {
                let path = entry.path();
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if metadata.is_dir() {
                    // Skip hidden directories
                    if let Some(name) = path.file_name() {
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with('.')
                            || name_str == "target"
                            || name_str == "node_modules"
                        {
                            continue;
                        }
                    }
                    entries.push(path);
                } else if metadata.is_file()
                    && let Some(name) = path.file_name()
                {
                    let name_str = name.to_string_lossy();
                    if Self::matches_file_pattern(&name_str, file_pattern) {
                        files.push(path);
                    }
                }

                if files.len() >= max_files {
                    break;
                }
            }
        }

        Ok(files)
    }
}

impl Default for GrepSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for GrepSearchTool {
    fn name(&self) -> crate::types::ToolName {
        crate::types::ToolName::new("grep_search").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Search for patterns across files using regex or literal matching. \
         Returns matches with file path, line number, content, and surrounding context. \
         Allowed in all modes (read-only operation)."
    }

    fn allowed_in_mode(&self, _mode: ExecutionMode) -> bool {
        // grep_search is allowed in all modes (read-only search)
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

        // Extract required query
        let query = args.require("query")?;

        // Extract optional path (defaults to working_dir)
        let search_path = args
            .get("path")
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        // Validate path
        if has_traversal_components(&search_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components (..)",
                search_path.display()
            )));
        }

        let validated_path = validate_path_boundary(&search_path, &ctx.working_dir)?;

        // Parse optional parameters
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(true);

        let max_results = args
            .get("max_results")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(50)
            .min(100); // Hard cap at 100

        let file_pattern = args.get("file_pattern");

        let context_lines = args
            .get("context_lines")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(2)
            .min(5); // Cap context at 5 lines

        let use_regex = args
            .get("use_regex")
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(true);

        // Collect files to search
        let files_to_search = if validated_path.is_file() {
            vec![validated_path.clone()]
        } else {
            self.collect_files(&validated_path, file_pattern, 1000)?
        };

        // Search files
        let mut all_matches = Vec::new();
        let mut remaining_results = max_results;

        for file in files_to_search {
            if remaining_results == 0 {
                break;
            }

            match self.search_file(
                &file,
                query,
                case_sensitive,
                context_lines,
                use_regex,
                &mut remaining_results,
            ) {
                Ok(mut matches) => all_matches.append(&mut matches),
                Err(e) => {
                    // Continue on error for individual files
                    eprintln!("Warning: Failed to search {}: {}", file.display(), e);
                }
            }
        }

        let elapsed = start.elapsed().as_millis() as u64;

        // Format output
        let match_count = all_matches.len();
        let output = if match_count == 0 {
            format!(
                "No matches found for '{}' in {}",
                query,
                validated_path.display()
            )
        } else {
            let mut lines = vec![format!(
                "Found {} match(es) for '{}' in {}:",
                match_count,
                query,
                validated_path.display()
            )];

            for m in &all_matches {
                lines.push(format!("\n{}:{}:", m.file_path.display(), m.line_number));

                // Context before
                let ctx_start = m.line_number.saturating_sub(context_lines + 1);
                for (i, ctx) in m.context_before.iter().enumerate() {
                    lines.push(format!("  {} | {}", ctx_start + i + 1, ctx));
                }

                // Match line
                lines.push(format!("> {} | {}", m.line_number, m.content));

                // Context after
                for (i, ctx) in m.context_after.iter().enumerate() {
                    lines.push(format!("  {} | {}", m.line_number + i + 1, ctx));
                }
            }

            lines.join("\n")
        };

        // Build metadata
        let mut metadata = HashMap::new();
        metadata.insert("match_count".to_string(), match_count.to_string());
        metadata.insert("query".to_string(), query.to_string());
        metadata.insert(
            "search_path".to_string(),
            validated_path.display().to_string(),
        );

        Ok(ToolResult {
            success: true,
            output: Some(output),
            error: None,
            mutations: vec![], // Read-only operation
            execution_time_ms: elapsed,
        })
    }
}

/// ===========================================================================
/// LIST DIRECTORY TOOL
/// ===========================================================================
///
/// Directory entry information
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub path: PathBuf,
    #[allow(dead_code)]
    pub name: String,
    pub entry_type: EntryType,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    File,
    Directory,
    Symlink,
    Other,
}

impl EntryType {
    #[allow(dead_code)]
    fn as_str(&self) -> &'static str {
        match self {
            EntryType::File => "file",
            EntryType::Directory => "dir",
            EntryType::Symlink => "link",
            EntryType::Other => "other",
        }
    }
}

/// List directory tool with recursive and filtering options
///
/// Arguments:
/// - path: Directory to list (required)
/// - recursive: Whether to list subdirectories recursively (default: false)
/// - include_hidden: Whether to include hidden entries (default: false)
/// - file_type: Filter by type - "file", "dir", "all" (default: "all")
/// - max_depth: Maximum recursion depth when recursive (default: 3)
/// - max_entries: Maximum entries to return (default: 100)
///
/// Returns:
/// - List of entries with path, name, type, and size
pub struct ListDirTool;

impl ListDirTool {
    pub fn new() -> Self {
        Self
    }

    /// Check if entry should be included based on filters
    fn should_include(
        name: &str,
        entry_type: EntryType,
        include_hidden: bool,
        file_type_filter: Option<&str>,
    ) -> bool {
        // Hidden file check
        if !include_hidden && name.starts_with('.') {
            return false;
        }

        // Type filter
        if let Some(filter) = file_type_filter {
            let matches = match filter {
                "file" => entry_type == EntryType::File,
                "dir" | "directory" => entry_type == EntryType::Directory,
                "link" | "symlink" => entry_type == EntryType::Symlink,
                "all" => true,
                _ => true,
            };
            if !matches {
                return false;
            }
        }

        true
    }

    /// Recursively list directory
    fn list_recursive(
        &self,
        dir: &Path,
        include_hidden: bool,
        file_type_filter: Option<&str>,
        max_depth: usize,
        current_depth: usize,
        max_entries: &mut usize,
    ) -> Result<Vec<DirEntry>, ForgeError> {
        let mut entries = Vec::new();

        if current_depth > max_depth || *max_entries == 0 {
            return Ok(entries);
        }

        let read_dir = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                return Err(ForgeError::IoError(format!(
                    "Cannot read directory {}: {}",
                    dir.display(),
                    e
                )));
            }
        };

        // Collect and sort entries for consistent ordering
        let mut dir_entries: Vec<_> = read_dir.flatten().collect();
        dir_entries.sort_by(|a, b| {
            let a_name = a.file_name();
            let b_name = b.file_name();
            a_name.cmp(&b_name)
        });

        for entry in dir_entries {
            if *max_entries == 0 {
                break;
            }

            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // Determine entry type
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue, // Skip entries we can't stat
            };

            let entry_type = if metadata.is_dir() {
                EntryType::Directory
            } else if metadata.is_file() {
                EntryType::File
            } else if metadata.is_symlink() {
                EntryType::Symlink
            } else {
                EntryType::Other
            };

            // Check filters
            if !Self::should_include(&name, entry_type, include_hidden, file_type_filter) {
                continue;
            }

            // Add entry
            let size = if metadata.is_file() {
                Some(metadata.len())
            } else {
                None
            };

            entries.push(DirEntry {
                path: path.clone(),
                name: name.clone(),
                entry_type,
                size,
            });
            *max_entries -= 1;

            // Recurse into directories
            if metadata.is_dir()
                && current_depth < max_depth
                && let Ok(mut sub_entries) = self.list_recursive(
                    &path,
                    include_hidden,
                    file_type_filter,
                    max_depth,
                    current_depth + 1,
                    max_entries,
                )
            {
                entries.append(&mut sub_entries);
            }
        }

        Ok(entries)
    }
}

impl Default for ListDirTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ListDirTool {
    fn name(&self) -> crate::types::ToolName {
        crate::types::ToolName::new("list_dir").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "List directory contents with optional recursion and filtering. \
         Returns entries with path, name, type, and size. \
         Allowed in all modes (read-only operation)."
    }

    fn allowed_in_mode(&self, _mode: ExecutionMode) -> bool {
        // list_dir is allowed in all modes (read-only operation)
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

        // Extract required path
        let path_str = args.require("path")?;
        let raw_path = PathBuf::from(path_str);

        // Validate path
        if has_traversal_components(&raw_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components (..)",
                raw_path.display()
            )));
        }

        let validated_path = validate_path_boundary(&raw_path, &ctx.working_dir)?;

        // Ensure it's a directory
        if !validated_path.is_dir() {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' is not a directory",
                validated_path.display()
            )));
        }

        // Parse optional parameters
        let recursive = args
            .get("recursive")
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(false);

        let include_hidden = args
            .get("include_hidden")
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(false);

        let file_type = args.get("file_type");

        let max_depth = args
            .get("max_depth")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(3)
            .min(5); // Hard cap at 5

        let max_entries = args
            .get("max_entries")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100)
            .min(500); // Hard cap at 500

        // List entries
        let entries = if recursive {
            self.list_recursive(
                &validated_path,
                include_hidden,
                file_type,
                max_depth,
                1,
                &mut max_entries.clone(),
            )?
        } else {
            self.list_recursive(
                &validated_path,
                include_hidden,
                file_type,
                0,
                0,
                &mut max_entries.clone(),
            )?
        };

        let elapsed = start.elapsed().as_millis() as u64;

        // Format output
        let entry_count = entries.len();
        let output = if entries.is_empty() {
            format!(
                "Directory {} is empty (or all entries filtered)",
                validated_path.display()
            )
        } else {
            let mut lines = vec![format!(
                "{} entries in {}:",
                entry_count,
                validated_path.display()
            )];

            for entry in &entries {
                let type_indicator = match entry.entry_type {
                    EntryType::Directory => "📁",
                    EntryType::File => "📄",
                    EntryType::Symlink => "🔗",
                    EntryType::Other => "❓",
                };

                let size_str = match entry.size {
                    Some(size) if size < 1024 => format!("{} B", size),
                    Some(size) if size < 1024 * 1024 => format!("{:.1} KB", size as f64 / 1024.0),
                    Some(size) => format!("{:.1} MB", size as f64 / (1024.0 * 1024.0)),
                    None => String::new(),
                };

                let relative_path = entry
                    .path
                    .strip_prefix(&validated_path)
                    .unwrap_or(&entry.path)
                    .display()
                    .to_string();

                lines.push(format!(
                    "{} {} {}{}",
                    type_indicator,
                    relative_path,
                    if size_str.is_empty() { "" } else { " (" },
                    if size_str.is_empty() { "" } else { &size_str }
                ));
            }

            lines.join("\n")
        };

        // Build metadata
        let mut metadata = HashMap::new();
        metadata.insert("entry_count".to_string(), entry_count.to_string());
        metadata.insert("path".to_string(), validated_path.display().to_string());
        if recursive {
            metadata.insert("recursive".to_string(), "true".to_string());
            metadata.insert("max_depth".to_string(), max_depth.to_string());
        }

        Ok(ToolResult {
            success: true,
            output: Some(output),
            error: None,
            mutations: vec![], // Read-only operation
            execution_time_ms: elapsed,
        })
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
    fn test_grep_search_finds_matches() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "line one\nline two\nline three\n").unwrap();

        let tool = GrepSearchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("query", "two"), ("path", ".")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("line two"));
        assert!(result.output.as_ref().unwrap().contains("test.txt:2"));
    }

    #[test]
    fn test_grep_search_case_insensitive() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "Hello World\n").unwrap();

        let tool = GrepSearchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[
            ("query", "hello"),
            ("path", "."),
            ("case_sensitive", "false"),
        ]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("Hello World"));
    }

    #[test]
    fn test_grep_search_file_pattern() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("test.rs"), "rust code\n").unwrap();
        fs::write(temp_dir.path().join("test.txt"), "text content\n").unwrap();

        let tool = GrepSearchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("query", "code"), ("path", "."), ("file_pattern", "*.rs")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("test.rs"));
        assert!(!result.output.as_ref().unwrap().contains("test.txt"));
    }

    #[test]
    fn test_grep_search_no_matches() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("test.txt"), "some content\n").unwrap();

        let tool = GrepSearchTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("query", "nonexistent"), ("path", ".")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("No matches found"));
    }

    #[test]
    fn test_list_dir_basic() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
        fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let tool = ListDirTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", ".")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        let output = result.output.as_ref().unwrap();
        assert!(output.contains("file.txt"));
        assert!(output.contains("subdir"));
    }

    #[test]
    fn test_list_dir_recursive() {
        let temp_dir = TempDir::new().unwrap();
        let nested = temp_dir.path().join("level1/level2");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("deep.txt"), "content").unwrap();

        let tool = ListDirTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "."), ("recursive", "true")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("deep.txt"));
    }

    #[test]
    fn test_list_dir_file_type_filter() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
        fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let tool = ListDirTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "."), ("file_type", "file")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("file.txt"));
        assert!(!result.output.as_ref().unwrap().contains("subdir"));
    }

    #[test]
    fn test_list_dir_hidden_filter() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("visible.txt"), "content").unwrap();
        fs::write(temp_dir.path().join(".hidden"), "secret").unwrap();

        let tool = ListDirTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "."), ("include_hidden", "false")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("visible.txt"));
        assert!(!result.output.as_ref().unwrap().contains(".hidden"));
    }

    #[test]
    fn test_list_dir_traversal_blocked() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ListDirTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("path", "../outside")]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should fail with path validation error
    }
}
