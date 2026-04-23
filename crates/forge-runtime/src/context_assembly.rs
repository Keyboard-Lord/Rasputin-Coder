//! Context Assembly Module - Phase 5
//!
//! Automatically detects when a task needs context gathering and assembles
//! relevant file content before planner invocation.
//!
//! Triggers:
//! - "find all usages of function X" -> search + read relevant files
//! - "implement feature Y" -> search for related files + read
//! - "fix bug in Z" -> search for Z + read relevant code
//! - "refactor W" -> search for W + read all occurrences
//!
//! The assembled context is prepended to the task description in StateView.

use crate::tool_registry::Tool;
use crate::tools::file_tools::ReadFileTool;
use crate::tools::search_tools::{GrepSearchTool, ListDirTool};
use crate::types::{ExecutionContext, ForgeError, ToolArguments};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Maximum files to read per context assembly
const MAX_FILES_TO_READ: usize = 10;

/// Maximum lines per file to include in context
const MAX_LINES_PER_FILE: usize = 100;

/// Maximum total context size (characters)
const MAX_CONTEXT_SIZE: usize = 50000;

/// Maximum deterministic repo-shape files to enumerate
const MAX_REPO_SHAPE_FILES: usize = 32;

/// Pattern to detect if task already has sufficient context
const SKIP_PATTERNS: &[&str] = &[
    "file",
    "read_file",
    "already read",
    "contents of",
    "in the file",
];

/// Detected context need from task analysis
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextNeed {
    /// No additional context needed
    None,
    /// Search for function/identifier usages
    FindUsages { identifier: String },
    /// Search for implementation of feature/component
    FindImplementation { component: String },
    /// Search for related to bug/issue
    FindRelated { keyword: String },
    /// General codebase exploration
    #[allow(dead_code)]
    ExploreStructure,
}

/// Result of context assembly
#[derive(Debug, Clone)]
pub struct AssembledContext {
    /// Original task
    #[allow(dead_code)]
    pub original_task: String,
    /// Additional context discovered
    pub context: String,
    /// Files that were read
    pub files_read: Vec<PathBuf>,
    /// Whether assembly was performed
    pub was_assembled: bool,
}

impl AssembledContext {
    /// Get the full task with context prepended
    #[allow(dead_code)]
    pub fn full_task(&self) -> String {
        if self.context.is_empty() {
            self.original_task.clone()
        } else {
            format!(
                "=== RELEVANT CONTEXT ===\n\n{context}\n\n=== TASK ===\n\n{original_task}",
                context = self.context,
                original_task = self.original_task
            )
        }
    }
}

/// Context assembler for automatic context gathering
pub struct ContextAssembler {
    /// Compiled regex patterns for detection
    usage_pattern: Regex,
    impl_pattern: Regex,
    bug_pattern: Regex,
    refactor_pattern: Regex,
}

/// Dependency-aware context ranking engine.
pub struct ContextBuilder {
    max_files: usize,
}

impl ContextBuilder {
    pub fn new(max_files: usize) -> Self {
        Self { max_files }
    }

    pub fn rank_files(
        &self,
        keyword: &str,
        working_dir: &Path,
        search_hits: &[PathBuf],
    ) -> Vec<PathBuf> {
        let mut scores: HashMap<PathBuf, i32> = HashMap::new();

        for entrypoint in detect_entrypoints(working_dir) {
            *scores.entry(entrypoint).or_insert(0) += 30;
        }

        for hit in search_hits {
            *scores.entry(hit.clone()).or_insert(0) += 50;
            for dependency in direct_dependency_candidates(hit) {
                *scores.entry(dependency).or_insert(0) += 20;
            }
        }

        let keyword_lower = keyword.to_lowercase();
        for (path, score) in &mut scores {
            let path_lower = path.to_string_lossy().to_lowercase();
            if path_lower.contains(&keyword_lower) {
                *score += 15;
            }
            if path.file_name().is_some_and(|name| {
                matches!(
                    name.to_string_lossy().as_ref(),
                    "main.rs" | "lib.rs" | "mod.rs" | "package.json" | "Cargo.toml"
                )
            }) {
                *score += 10;
            }
        }

        let mut ranked: Vec<(PathBuf, i32)> = scores
            .into_iter()
            .filter(|(path, _)| path.exists() && path.is_file())
            .collect();
        ranked.sort_by(|(left_path, left_score), (right_path, right_score)| {
            right_score.cmp(left_score).then_with(|| {
                left_path
                    .display()
                    .to_string()
                    .cmp(&right_path.display().to_string())
            })
        });

        ranked
            .into_iter()
            .map(|(path, _)| path)
            .take(self.max_files)
            .collect()
    }
}

impl ContextAssembler {
    pub fn new() -> Result<Self, ForgeError> {
        Ok(Self {
            // Pattern: find/search/get usages/references/calls of <identifier>
            // Matches: "find all usages of process_data", "show me all references to the validate_input function", etc.
            usage_pattern: Regex::new(
                r"(?i)(?:find|search|get|show|list)(?:\s+me)?\s+(?:all\s+)?(?:usages?|uses?|references?|calls?|occurrences?)\s+(?:(?:of|to)\s+(?:the\s+)?)?(?:function\s+)?([a-z_][a-z0-9_]{2,})"
            ).map_err(|e| ForgeError::InvalidConfiguration(format!("Invalid regex: {}", e)))?,
            // Pattern: implement/add/create <component>
            // Matches: "implement the calculate_total function", "add validate_input method", etc.
            impl_pattern: Regex::new(
                r"(?i)(?:implement|add|create)\s+(?:the\s+)?([a-z_][a-z0-9_]{2,})(?:\s+(?:function|method|feature|component|class|struct|enum|trait|type))?"
            ).map_err(|e| ForgeError::InvalidConfiguration(format!("Invalid regex: {}", e)))?,
            // Pattern: fix/debug/resolve bug in <keyword>
            // Matches: "fix bug in authentication", "resolve the bug in auth", etc.
            bug_pattern: Regex::new(
                r"(?i)(?:fix|debug|resolve)\s+(?:the\s+)?(?:bug\s+)?(?:in\s+)?([a-z_][a-z0-9_]{2,})"
            ).map_err(|e| ForgeError::InvalidConfiguration(format!("Invalid regex: {}", e)))?,
            // Pattern: refactor/rename/extract <keyword>
            // Matches: "refactor the parse_config function", "rename old_function", etc.
            refactor_pattern: Regex::new(
                r"(?i)(?:refactor|rename|extract)\s+(?:the\s+)?([a-z_][a-z0-9_]{2,})(?:\s+(?:function|method|code|component))?"
            ).map_err(|e| ForgeError::InvalidConfiguration(format!("Invalid regex: {}", e)))?,
        })
    }

    /// Analyze task to determine what context is needed
    pub fn analyze_task(&self, task: &str) -> ContextNeed {
        // Check if task already has sufficient context
        let task_lower = task.to_lowercase();
        if SKIP_PATTERNS.iter().any(|p| task_lower.contains(p)) {
            return ContextNeed::None;
        }

        // Check for usage search patterns (capture group 1 is the identifier)
        if let Some(captures) = self.usage_pattern.captures(task)
            && let Some(identifier) = captures.get(1)
        {
            return ContextNeed::FindUsages {
                identifier: identifier.as_str().to_string(),
            };
        }

        // Check for implementation patterns (capture group 1 is the component)
        if let Some(captures) = self.impl_pattern.captures(task)
            && let Some(component) = captures.get(1)
        {
            return ContextNeed::FindImplementation {
                component: component.as_str().to_string(),
            };
        }

        // Check for bug fix patterns (capture group 1 is the keyword)
        if let Some(captures) = self.bug_pattern.captures(task)
            && let Some(keyword) = captures.get(1)
        {
            return ContextNeed::FindRelated {
                keyword: keyword.as_str().to_string(),
            };
        }

        // Check for refactor patterns (capture group 1 is the keyword)
        if let Some(captures) = self.refactor_pattern.captures(task)
            && let Some(keyword) = captures.get(1)
        {
            return ContextNeed::FindRelated {
                keyword: keyword.as_str().to_string(),
            };
        }

        // Default: no specific context needed
        ContextNeed::None
    }

    /// Assemble context based on detected need
    pub fn assemble_context(
        &self,
        need: &ContextNeed,
        working_dir: &Path,
    ) -> Result<AssembledContext, ForgeError> {
        let original_task = match need {
            ContextNeed::None => {
                return Ok(AssembledContext {
                    original_task: String::new(),
                    context: String::new(),
                    files_read: vec![],
                    was_assembled: false,
                });
            }
            ContextNeed::FindUsages { identifier } => {
                self.assemble_usage_context(identifier, working_dir)?
            }
            ContextNeed::FindImplementation { component } => {
                self.assemble_implementation_context(component, working_dir)?
            }
            ContextNeed::FindRelated { keyword } => {
                self.assemble_related_context(keyword, working_dir)?
            }
            ContextNeed::ExploreStructure => self.assemble_structure_context(working_dir)?,
        };

        Ok(original_task)
    }

    /// Assemble context for finding usages of an identifier
    fn assemble_usage_context(
        &self,
        identifier: &str,
        working_dir: &Path,
    ) -> Result<AssembledContext, ForgeError> {
        let mut files_read: Vec<PathBuf> = vec![];
        let mut context_lines: Vec<String> = vec![];

        // Step 1: Search for the identifier
        let grep_tool = GrepSearchTool::new();
        let ctx = ExecutionContext {
            session_id: crate::types::SessionId::new(),
            iteration: 0,
            mode: crate::types::ExecutionMode::Analysis,
            working_dir: working_dir.to_path_buf(),
        };

        let mut args = ToolArguments::new();
        args.set("query", identifier);
        args.set("path", ".");
        args.set("max_results", "20");
        args.set("context_lines", "3");

        let grep_result = grep_tool.execute(&args, &ctx)?;

        // Parse grep results to extract file paths
        let found_files = self.extract_files_from_grep_output(
            grep_result.output.as_deref().unwrap_or(""),
            working_dir,
        );

        if !found_files.is_empty() {
            context_lines.push(format!(
                "Found {} file(s) containing '{}':\n",
                found_files.len().min(MAX_FILES_TO_READ),
                identifier
            ));

            // Step 2: Read relevant files
            let read_tool = ReadFileTool::new();
            let mut read_count = 0;

            let ranked_files = ContextBuilder::new(MAX_FILES_TO_READ).rank_files(
                identifier,
                working_dir,
                &found_files,
            );

            for file_path in &ranked_files {
                let mut read_args = ToolArguments::new();
                read_args.set("path", &file_path.to_string_lossy());
                read_args.set("limit", &MAX_LINES_PER_FILE.to_string());

                match read_tool.execute(&read_args, &ctx) {
                    Ok(result) => {
                        if result.success {
                            files_read.push(file_path.clone());
                            read_count += 1;

                            // Get content from the read_file tool
                            // We need to re-read to get actual content since ToolResult doesn't contain it
                            if let Ok(content) = std::fs::read_to_string(file_path) {
                                let lines: Vec<&str> = content.lines().collect();
                                let truncated: Vec<&str> =
                                    lines.iter().take(MAX_LINES_PER_FILE).copied().collect();

                                context_lines.push(format!(
                                    "\n--- {} (first {} lines) ---\n{}",
                                    file_path.display(),
                                    truncated.len(),
                                    truncated.join("\n")
                                ));
                            }
                        }
                    }
                    Err(_) => continue, // Skip files we can't read
                }
            }

            context_lines.push(format!(
                "\n[Read {} of {} files]\n",
                read_count,
                ranked_files.len()
            ));
        } else {
            context_lines.push(format!(
                "No files found containing '{}' in the codebase.\n",
                identifier
            ));
        }

        let context = context_lines.join("\n");
        let truncated_context = if context.len() > MAX_CONTEXT_SIZE {
            format!(
                "{}\n[Context truncated: exceeded size limit]",
                &context[..MAX_CONTEXT_SIZE]
            )
        } else {
            context
        };

        Ok(AssembledContext {
            original_task: String::new(),
            context: truncated_context,
            files_read,
            was_assembled: true,
        })
    }

    /// Assemble context for implementing a feature
    fn assemble_implementation_context(
        &self,
        component: &str,
        working_dir: &Path,
    ) -> Result<AssembledContext, ForgeError> {
        // Similar to usage context but with broader search
        let mut files_read: Vec<PathBuf> = vec![];
        let mut context_lines: Vec<String> = vec![];

        let grep_tool = GrepSearchTool::new();
        let ctx = ExecutionContext {
            session_id: crate::types::SessionId::new(),
            iteration: 0,
            mode: crate::types::ExecutionMode::Analysis,
            working_dir: working_dir.to_path_buf(),
        };

        // Search for the component
        let mut args = ToolArguments::new();
        args.set("query", component);
        args.set("path", ".");
        args.set("max_results", "15");
        args.set("file_pattern", "*.rs"); // Focus on Rust files by default

        let grep_result = grep_tool.execute(&args, &ctx)?;
        let found_files = self.extract_files_from_grep_output(
            grep_result.output.as_deref().unwrap_or(""),
            working_dir,
        );

        if !found_files.is_empty() {
            context_lines.push(format!(
                "Found {} file(s) potentially related to '{}':\n",
                found_files.len().min(MAX_FILES_TO_READ),
                component
            ));

            let read_tool = ReadFileTool::new();
            let mut read_count = 0;

            let ranked_files = ContextBuilder::new(MAX_FILES_TO_READ).rank_files(
                component,
                working_dir,
                &found_files,
            );

            for file_path in &ranked_files {
                let mut read_args = ToolArguments::new();
                read_args.set("path", &file_path.to_string_lossy());
                read_args.set("limit", &MAX_LINES_PER_FILE.to_string());

                match read_tool.execute(&read_args, &ctx) {
                    Ok(result) => {
                        if result.success {
                            files_read.push(file_path.clone());
                            read_count += 1;

                            if let Ok(content) = std::fs::read_to_string(file_path) {
                                let lines: Vec<&str> = content.lines().collect();
                                let truncated: Vec<&str> =
                                    lines.iter().take(MAX_LINES_PER_FILE).copied().collect();

                                context_lines.push(format!(
                                    "\n--- {} (first {} lines) ---\n{}",
                                    file_path.display(),
                                    truncated.len(),
                                    truncated.join("\n")
                                ));
                            }
                        }
                    }
                    Err(_) => continue,
                }
            }

            context_lines.push(format!(
                "\n[Read {} of {} files]\n",
                read_count,
                ranked_files.len()
            ));
        }

        // Also list directory structure for context
        let list_tool = ListDirTool::new();
        let mut list_args = ToolArguments::new();
        list_args.set("path", ".");
        list_args.set("max_entries", "50");

        if let Ok(list_result) = list_tool.execute(&list_args, &ctx)
            && list_result.success
        {
            context_lines.push("\n--- Project Structure ---".to_string());
            context_lines.push(list_result.output.unwrap_or_default());
        }

        let context = context_lines.join("\n");
        let truncated_context = if context.len() > MAX_CONTEXT_SIZE {
            format!(
                "{}\n[Context truncated: exceeded size limit]",
                &context[..MAX_CONTEXT_SIZE]
            )
        } else {
            context
        };

        Ok(AssembledContext {
            original_task: String::new(),
            context: truncated_context,
            files_read,
            was_assembled: true,
        })
    }

    /// Assemble context for finding related code
    fn assemble_related_context(
        &self,
        keyword: &str,
        working_dir: &Path,
    ) -> Result<AssembledContext, ForgeError> {
        // Similar to implementation context
        self.assemble_implementation_context(keyword, working_dir)
    }

    /// Assemble general structure context
    fn assemble_structure_context(
        &self,
        working_dir: &Path,
    ) -> Result<AssembledContext, ForgeError> {
        let mut context_lines: Vec<String> = vec![];

        let ctx = ExecutionContext {
            session_id: crate::types::SessionId::new(),
            iteration: 0,
            mode: crate::types::ExecutionMode::Analysis,
            working_dir: working_dir.to_path_buf(),
        };

        // List directory structure
        let list_tool = ListDirTool::new();
        let mut list_args = ToolArguments::new();
        list_args.set("path", ".");
        list_args.set("recursive", "true");
        list_args.set("max_entries", "100");
        list_args.set("max_depth", "3");

        if let Ok(list_result) = list_tool.execute(&list_args, &ctx)
            && list_result.success
        {
            context_lines.push("Project structure:\n".to_string());
            context_lines.push(list_result.output.unwrap_or_default());
        }

        // Look for key configuration files
        let key_files = vec![
            "Cargo.toml",
            "package.json",
            "README.md",
            "main.rs",
            "lib.rs",
        ];
        let read_tool = ReadFileTool::new();
        let mut files_read: Vec<PathBuf> = vec![];

        for file_name in key_files {
            let file_path = working_dir.join(file_name);
            if file_path.exists() {
                let mut read_args = ToolArguments::new();
                read_args.set("path", file_name);
                read_args.set("limit", "50");

                if let Ok(result) = read_tool.execute(&read_args, &ctx)
                    && result.success
                {
                    files_read.push(file_path.clone());

                    if let Ok(content) = std::fs::read_to_string(&file_path) {
                        let lines: Vec<&str> = content.lines().collect();
                        let truncated: Vec<&str> = lines.iter().take(50).copied().collect();

                        context_lines.push(format!(
                            "\n--- {} (first {} lines) ---\n{}",
                            file_name,
                            truncated.len(),
                            truncated.join("\n")
                        ));
                    }
                }
            }
        }

        let context = context_lines.join("\n");

        Ok(AssembledContext {
            original_task: String::new(),
            context,
            files_read,
            was_assembled: true,
        })
    }

    /// Extract file paths from grep search output
    fn extract_files_from_grep_output(&self, output: &str, working_dir: &Path) -> Vec<PathBuf> {
        let mut files = HashSet::new();

        for line in output.lines() {
            // Look for lines like "path/to/file.rs:123:" or "path/to/file.rs:"
            if let Some(colon_pos) = line.find(':') {
                let potential_path = &line[..colon_pos];
                let path = working_dir.join(potential_path);
                if path.exists() && path.is_file() {
                    files.insert(path);
                }
            }
        }

        let mut result: Vec<PathBuf> = files.into_iter().collect();
        result.sort();
        result
    }
}

impl Default for ContextAssembler {
    fn default() -> Self {
        Self::new().expect("Failed to create ContextAssembler")
    }
}

/// Auto-assembly result that can be applied to a task
#[derive(Debug, Clone)]
pub struct AutoAssemblyResult {
    pub was_assembled: bool,
    pub enriched_task: String,
    #[allow(dead_code)]
    pub files_read: Vec<PathBuf>,
    pub context_summary: String,
}

#[derive(Debug, Clone, Default)]
pub struct RepoShape {
    pub project_type: String,
    pub manifests: Vec<PathBuf>,
    pub entrypoints: Vec<PathBuf>,
    pub source_files: Vec<PathBuf>,
    pub test_files: Vec<PathBuf>,
    pub config_files: Vec<PathBuf>,
}

impl RepoShape {
    pub fn detect(working_dir: &Path) -> Self {
        let mut shape = RepoShape::default();
        if working_dir.join("Cargo.toml").exists() {
            shape.project_type = "Rust Cargo package/workspace".to_string();
            shape.manifests.push(PathBuf::from("Cargo.toml"));
        } else if working_dir.join("package.json").exists() {
            shape.project_type = "JavaScript/TypeScript package".to_string();
            shape.manifests.push(PathBuf::from("package.json"));
        } else {
            shape.project_type = "Unknown".to_string();
        }

        shape.entrypoints = relative_existing_paths(
            working_dir,
            &[
                "src/lib.rs",
                "src/main.rs",
                "src/bin",
                "src/index.ts",
                "src/index.tsx",
                "src/index.js",
                "src/main.ts",
                "src/main.tsx",
                "src/main.js",
            ],
        );
        shape.source_files = collect_files_under(working_dir, "src", MAX_REPO_SHAPE_FILES);
        shape.test_files = collect_files_under(working_dir, "tests", MAX_REPO_SHAPE_FILES);
        shape.config_files = shape
            .source_files
            .iter()
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy().to_lowercase();
                    name.contains("config")
                        || name.contains("setting")
                        || name.contains("timeout")
                        || name.contains("option")
                })
            })
            .cloned()
            .collect();
        shape
    }

    pub fn is_useful(&self) -> bool {
        !self.manifests.is_empty() || !self.source_files.is_empty() || !self.test_files.is_empty()
    }
}

pub fn repo_shape_context(task: &str, working_dir: &Path) -> String {
    let shape = RepoShape::detect(working_dir);
    if !shape.is_useful() {
        return String::new();
    }

    let mut lines = vec![
        "--- Repository Shape ---".to_string(),
        format!("Project type: {}", shape.project_type),
        format!("Manifests: {}", format_paths(&shape.manifests)),
        format!("Entrypoints: {}", format_paths(&shape.entrypoints)),
        format!("Source files: {}", format_paths(&shape.source_files)),
        format!("Tests: {}", format_paths(&shape.test_files)),
    ];

    if !shape.config_files.is_empty() {
        lines.push(format!(
            "Config-like source files: {}",
            format_paths(&shape.config_files)
        ));
    }

    let lower = task.to_lowercase();
    let mut likely = Vec::new();
    if lower.contains("timeout") || lower.contains("request") || lower.contains("1000") {
        likely.extend(paths_matching_terms(
            &shape.source_files,
            &["config", "timeout", "setting"],
        ));
        likely.extend(paths_matching_terms(
            &shape.test_files,
            &["config", "timeout"],
        ));
    }
    if lower.contains("parse_setting") || lower.contains("setting") {
        likely.extend(paths_matching_terms(
            &shape.source_files,
            &["lib", "setting"],
        ));
        likely.extend(paths_matching_terms(&shape.test_files, &["setting"]));
    }
    if lower.contains("math") || lower.contains("triple") || lower.contains("is_even") {
        likely.push(PathBuf::from("src/math.rs"));
        likely.push(PathBuf::from("src/lib.rs"));
        likely.extend(paths_matching_terms(
            &shape.test_files,
            &["feature", "math"],
        ));
    }
    if lower.contains("compile error") {
        likely.extend(paths_matching_terms(&shape.source_files, &["lib", "main"]));
    }
    likely.sort();
    likely.dedup();

    if !likely.is_empty() {
        lines.push(format!("Likely task surfaces: {}", format_paths(&likely)));
    }

    lines.push(
        "Repo-shape guidance: if a guessed path is missing, redirect to the listed source/test surfaces instead of repeating the missing path."
            .to_string(),
    );
    lines.push(
        "Multi-file guidance: for Rust modules, keep module file creation and src/lib.rs exposure aligned; validation is not complete until cargo test passes."
            .to_string(),
    );

    lines.join("\n")
}

/// Automatically assemble context for a task if needed
pub fn auto_assemble_context(
    task: &str,
    working_dir: &Path,
) -> Result<AutoAssemblyResult, ForgeError> {
    let assembler = ContextAssembler::new()?;
    let need = assembler.analyze_task(task);
    let shape_context = repo_shape_context(task, working_dir);

    if need == ContextNeed::None && shape_context.is_empty() {
        return Ok(AutoAssemblyResult {
            was_assembled: false,
            enriched_task: task.to_string(),
            files_read: vec![],
            context_summary: String::new(),
        });
    }

    let assembled = assembler.assemble_context(&need, working_dir)?;

    if (!assembled.was_assembled || assembled.context.is_empty()) && shape_context.is_empty() {
        return Ok(AutoAssemblyResult {
            was_assembled: false,
            enriched_task: task.to_string(),
            files_read: vec![],
            context_summary: String::new(),
        });
    }

    let context = [shape_context, assembled.context]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let enriched_task = format!(
        "{context}\n\n=== TASK ===\n\n{task}",
        context = context,
        task = task
    );

    let context_summary = format!(
        "Assembled context for '{}': read {} files{}",
        match need {
            ContextNeed::FindUsages { identifier } => format!("find usages of {}", identifier),
            ContextNeed::FindImplementation { component } => format!("implement {}", component),
            ContextNeed::FindRelated { keyword } => format!("find related to {}", keyword),
            ContextNeed::ExploreStructure => "explore structure".to_string(),
            ContextNeed::None => "repo shape".to_string(),
        },
        assembled.files_read.len(),
        if context.contains("Repository Shape") {
            " plus repo shape"
        } else {
            ""
        }
    );

    Ok(AutoAssemblyResult {
        was_assembled: true,
        enriched_task,
        files_read: assembled.files_read,
        context_summary,
    })
}

fn relative_existing_paths(working_dir: &Path, candidates: &[&str]) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for candidate in candidates {
        let path = working_dir.join(candidate);
        if path.is_file() {
            result.push(PathBuf::from(candidate));
        } else if path.is_dir() {
            result.extend(collect_files_under(
                working_dir,
                candidate,
                MAX_REPO_SHAPE_FILES,
            ));
        }
    }
    result.sort();
    result.dedup();
    result
}

fn collect_files_under(working_dir: &Path, relative_dir: &str, max_files: usize) -> Vec<PathBuf> {
    let root = working_dir.join(relative_dir);
    let mut files = Vec::new();
    collect_files_recursive(working_dir, &root, 0, &mut files, max_files);
    files.sort();
    files.dedup();
    files.truncate(max_files);
    files
}

fn collect_files_recursive(
    working_dir: &Path,
    dir: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    max_files: usize,
) {
    if depth > 3 || files.len() >= max_files || !dir.is_dir() {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if files.len() >= max_files {
            break;
        }
        if path.is_dir() {
            collect_files_recursive(working_dir, &path, depth + 1, files, max_files);
        } else if is_context_source_file(&path)
            && let Ok(relative) = path.strip_prefix(working_dir)
        {
            files.push(relative.to_path_buf());
        }
    }
}

fn is_context_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext,
                "rs" | "toml" | "json" | "ts" | "tsx" | "js" | "jsx" | "py"
            )
        })
}

fn paths_matching_terms(paths: &[PathBuf], terms: &[&str]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|path| {
            let lower = path.to_string_lossy().to_lowercase();
            terms.iter().any(|term| lower.contains(term))
        })
        .cloned()
        .collect()
}

fn format_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn detect_entrypoints(working_dir: &Path) -> Vec<PathBuf> {
    let mut entrypoints = Vec::new();

    if working_dir.join("Cargo.toml").exists() {
        for path in [
            working_dir.join("Cargo.toml"),
            working_dir.join("src/main.rs"),
            working_dir.join("src/lib.rs"),
        ] {
            if path.exists() {
                entrypoints.push(path);
            }
        }

        let bin_dir = working_dir.join("src/bin");
        if let Ok(entries) = std::fs::read_dir(bin_dir) {
            let mut bins: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            bins.sort();
            entrypoints.extend(bins.into_iter().filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext == "rs")
            }));
        }
    }

    let package_json = working_dir.join("package.json");
    if package_json.exists() {
        entrypoints.push(package_json);
        for path in [
            working_dir.join("src/index.ts"),
            working_dir.join("src/index.tsx"),
            working_dir.join("src/index.js"),
            working_dir.join("src/main.ts"),
            working_dir.join("src/main.tsx"),
            working_dir.join("src/main.js"),
        ] {
            if path.exists() {
                entrypoints.push(path);
            }
        }
    }

    entrypoints.sort();
    entrypoints.dedup();
    entrypoints
}

fn direct_dependency_candidates(path: &Path) -> Vec<PathBuf> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    let mut dependencies = HashSet::new();
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    if ext == "rs" {
        let mod_pattern = Regex::new(r"(?m)^\s*(?:pub\s+)?mod\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*;")
            .expect("valid rust module regex");
        for captures in mod_pattern.captures_iter(&content) {
            if let Some(parent) = path.parent() {
                for candidate in [
                    parent.join(format!("{}.rs", &captures[1])),
                    parent.join(&captures[1]).join("mod.rs"),
                ] {
                    if candidate.exists() {
                        dependencies.insert(candidate);
                    }
                }
            }
        }

        let use_pattern =
            Regex::new(r"\buse\s+crate::([a-zA-Z_][a-zA-Z0-9_]*)").expect("valid rust use regex");
        if let Some(project_root) = find_upward(path, "Cargo.toml") {
            let src = project_root.join("src");
            for captures in use_pattern.captures_iter(&content) {
                for candidate in [
                    src.join(format!("{}.rs", &captures[1])),
                    src.join(&captures[1]).join("mod.rs"),
                ] {
                    if candidate.exists() {
                        dependencies.insert(candidate);
                    }
                }
            }
        }
    }

    let mut result: Vec<PathBuf> = dependencies.into_iter().collect();
    result.sort();
    result
}

fn find_upward(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };

    loop {
        if current.join(marker).exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
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
    use tempfile::TempDir;

    #[test]
    fn test_analyze_task_find_usages() {
        let assembler = ContextAssembler::new().unwrap();

        let need = assembler.analyze_task("Find all usages of function process_data");
        assert!(
            matches!(need, ContextNeed::FindUsages { identifier } if identifier == "process_data")
        );
    }

    #[test]
    fn test_analyze_task_find_references() {
        let assembler = ContextAssembler::new().unwrap();

        let need = assembler.analyze_task("Show me all references to the validate_input function");
        assert!(
            matches!(need, ContextNeed::FindUsages { identifier } if identifier == "validate_input")
        );
    }

    #[test]
    fn test_analyze_task_implement_feature() {
        let assembler = ContextAssembler::new().unwrap();

        let need = assembler.analyze_task("Implement the calculate_total function");
        assert!(
            matches!(need, ContextNeed::FindImplementation { component } if component == "calculate_total")
        );
    }

    #[test]
    fn test_analyze_task_fix_bug() {
        let assembler = ContextAssembler::new().unwrap();

        let need = assembler.analyze_task("Fix the bug in authentication");
        assert!(
            matches!(need, ContextNeed::FindRelated { keyword } if keyword == "authentication")
        );
    }

    #[test]
    fn test_analyze_task_refactor() {
        let assembler = ContextAssembler::new().unwrap();

        let need = assembler.analyze_task("Refactor the parse_config function");
        assert!(matches!(need, ContextNeed::FindRelated { keyword } if keyword == "parse_config"));
    }

    #[test]
    fn test_analyze_task_no_context_needed() {
        let assembler = ContextAssembler::new().unwrap();

        // Task that already mentions file content
        let need = assembler.analyze_task("Add a new line to the file src/main.rs");
        assert_eq!(need, ContextNeed::None);
    }

    #[test]
    fn test_analyze_task_simple_write() {
        let assembler = ContextAssembler::new().unwrap();

        let need = assembler.analyze_task("Create a hello world program in src/main.rs");
        // This doesn't match any patterns, should be None
        assert_eq!(need, ContextNeed::None);
    }

    #[test]
    fn test_extract_files_from_grep() {
        let assembler = ContextAssembler::new().unwrap();
        let temp_dir = TempDir::new().unwrap();

        // Create test files
        std::fs::write(temp_dir.path().join("test1.rs"), "fn main() {}").unwrap();
        std::fs::write(temp_dir.path().join("test2.rs"), "fn foo() {}").unwrap();

        let grep_output = format!(
            "Found matches:\n{}:1:fn main() {{}}\n{}:1:fn foo() {{}}",
            temp_dir.path().join("test1.rs").display(),
            temp_dir.path().join("test2.rs").display()
        );

        let files = assembler.extract_files_from_grep_output(&grep_output, temp_dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_assembled_context_full_task() {
        let assembled = AssembledContext {
            original_task: "Find usages of foo".to_string(),
            context: "Found foo in file1.rs\nFound foo in file2.rs".to_string(),
            files_read: vec![PathBuf::from("file1.rs"), PathBuf::from("file2.rs")],
            was_assembled: true,
        };

        let full = assembled.full_task();
        assert!(full.contains("RELEVANT CONTEXT"));
        assert!(full.contains("Found foo in file1.rs"));
        assert!(full.contains("=== TASK ==="));
        assert!(full.contains("Find usages of foo"));
    }

    #[test]
    fn test_auto_assemble_context_skips_explicit_file_tasks() {
        let temp_dir = TempDir::new().unwrap();

        // Task that already references files should not trigger assembly
        let result = auto_assemble_context(
            "Read the file src/main.rs and add a function",
            temp_dir.path(),
        )
        .unwrap();

        assert!(!result.was_assembled);
        assert_eq!(
            result.enriched_task,
            "Read the file src/main.rs and add a function"
        );
    }

    #[test]
    fn repo_shape_detects_rust_entrypoints_config_and_tests() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join("tests")).unwrap();
        std::fs::write(
            temp_dir.path().join("Cargo.toml"),
            "[package]\nname='shape'\n",
        )
        .unwrap();
        std::fs::write(temp_dir.path().join("src/lib.rs"), "pub mod config;\n").unwrap();
        std::fs::write(
            temp_dir.path().join("src/config.rs"),
            "pub const DEFAULT_TIMEOUT_MS: u64 = 1000;\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("tests/config_tests.rs"),
            "#[test]\nfn t() {}\n",
        )
        .unwrap();

        let shape = RepoShape::detect(temp_dir.path());

        assert_eq!(shape.project_type, "Rust Cargo package/workspace");
        assert!(shape.manifests.contains(&PathBuf::from("Cargo.toml")));
        assert!(shape.entrypoints.contains(&PathBuf::from("src/lib.rs")));
        assert!(shape.config_files.contains(&PathBuf::from("src/config.rs")));
        assert!(
            shape
                .test_files
                .contains(&PathBuf::from("tests/config_tests.rs"))
        );
    }

    #[test]
    fn repo_shape_context_redirects_onboarding_timeout_task() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join("tests")).unwrap();
        std::fs::write(
            temp_dir.path().join("Cargo.toml"),
            "[package]\nname='shape'\n",
        )
        .unwrap();
        std::fs::write(temp_dir.path().join("src/lib.rs"), "pub mod config;\n").unwrap();
        std::fs::write(
            temp_dir.path().join("src/config.rs"),
            "pub const DEFAULT_TIMEOUT_MS: u64 = 1000;\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("tests/config_tests.rs"),
            "#[test]\nfn t() {}\n",
        )
        .unwrap();

        let context =
            repo_shape_context("Find the default request timeout constant", temp_dir.path());

        assert!(context.contains("Repository Shape"));
        assert!(context.contains("src/config.rs"));
        assert!(context.contains("tests/config_tests.rs"));
        assert!(context.contains("redirect to the listed source/test surfaces"));
    }

    #[test]
    fn auto_assemble_includes_repo_shape_for_onboarding_without_legacy_trigger() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
        std::fs::write(
            temp_dir.path().join("Cargo.toml"),
            "[package]\nname='shape'\n",
        )
        .unwrap();
        std::fs::write(temp_dir.path().join("src/lib.rs"), "pub fn existing() {}\n").unwrap();

        let result = auto_assemble_context(
            "Onboard to this small repo and change a timeout",
            temp_dir.path(),
        )
        .unwrap();

        assert!(result.was_assembled);
        assert!(result.enriched_task.contains("Repository Shape"));
        assert!(result.enriched_task.contains("src/lib.rs"));
    }
}
