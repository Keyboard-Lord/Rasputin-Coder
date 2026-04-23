//! Code intelligence tools for bounded planner context.
//!
//! These tools intentionally provide shallow, deterministic repository facts:
//! entry points, direct dependencies, symbol locations, and validation runners.

use crate::tool_registry::Tool;
use crate::types::{
    ExecutionContext, ExecutionMode, ForgeError, ToolArguments, ToolError, ToolName, ToolResult,
};
use regex::Regex;
use serde_json::json;
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_GRAPH_FILES: usize = 64;
const MAX_SYMBOL_RESULTS: usize = 100;
const RUNNER_TIMEOUT_SECONDS: u64 = 300;

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
            .map(Path::to_path_buf)
            .unwrap_or_else(|| canonical_working.clone());
        let canonical_parent = parent_path
            .canonicalize()
            .unwrap_or_else(|_| parent_path.clone());
        match normalized.file_name() {
            Some(name) => canonical_parent.join(name),
            None => canonical_parent,
        }
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

fn has_traversal_components(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn relative_to(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn collect_source_files(root: &Path, max_files: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut queue = VecDeque::from([root.to_path_buf()]);

    while let Some(dir) = queue.pop_front() {
        if files.len() >= max_files {
            break;
        }

        let mut entries: Vec<_> = match fs::read_dir(&dir) {
            Ok(entries) => entries.flatten().collect(),
            Err(_) => continue,
        };
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || matches!(name.as_str(), "target" | "node_modules" | "dist" | "build")
            {
                continue;
            }

            let Ok(metadata) = entry.metadata() else {
                continue;
            };

            if metadata.is_dir() {
                queue.push_back(path);
            } else if metadata.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        matches!(
                            ext,
                            "rs" | "js" | "jsx" | "ts" | "tsx" | "py" | "toml" | "json"
                        )
                    })
            {
                files.push(path);
                if files.len() >= max_files {
                    break;
                }
            }
        }
    }

    files.sort();
    files
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

fn resolve_rust_module(file: &Path, module: &str) -> Vec<PathBuf> {
    let Some(parent) = file.parent() else {
        return Vec::new();
    };

    [
        parent.join(format!("{}.rs", module)),
        parent.join(module).join("mod.rs"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

fn resolve_crate_path(file: &Path, raw_path: &str) -> Vec<PathBuf> {
    let Some(root) = find_upward(file, "Cargo.toml") else {
        return Vec::new();
    };
    let src = root.join("src");
    let first = raw_path
        .trim_start_matches("crate::")
        .split("::")
        .next()
        .unwrap_or("");

    if first.is_empty() {
        return Vec::new();
    }

    [
        src.join(format!("{}.rs", first)),
        src.join(first).join("mod.rs"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

fn resolve_script_import(file: &Path, import_path: &str) -> Vec<PathBuf> {
    if !import_path.starts_with('.') {
        return Vec::new();
    }

    let Some(parent) = file.parent() else {
        return Vec::new();
    };
    let base = parent.join(import_path);
    let candidates = [
        base.clone(),
        base.with_extension("ts"),
        base.with_extension("tsx"),
        base.with_extension("js"),
        base.with_extension("jsx"),
        base.join("index.ts"),
        base.join("index.tsx"),
        base.join("index.js"),
        base.join("index.jsx"),
    ];

    candidates
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

fn direct_dependencies(file: &Path) -> Vec<PathBuf> {
    let content = match fs::read_to_string(file) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    let mut deps = BTreeSet::new();
    let ext = file.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    if ext == "rs" {
        let mod_re = Regex::new(r"(?m)^\s*(?:pub\s+)?mod\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*;")
            .expect("valid rust module regex");
        let use_re = Regex::new(r"\buse\s+(crate::[a-zA-Z0-9_:]+)").expect("valid rust use regex");

        for captures in mod_re.captures_iter(&content) {
            for path in resolve_rust_module(file, &captures[1]) {
                deps.insert(path);
            }
        }
        for captures in use_re.captures_iter(&content) {
            for path in resolve_crate_path(file, &captures[1]) {
                deps.insert(path);
            }
        }
    } else if matches!(ext, "js" | "jsx" | "ts" | "tsx") {
        let import_re =
            Regex::new(r#"(?m)(?:import\s+.*?\s+from\s+|require\()\s*['"]([^'"]+)['"]"#)
                .expect("valid import regex");
        for captures in import_re.captures_iter(&content) {
            for path in resolve_script_import(file, &captures[1]) {
                deps.insert(path);
            }
        }
    }

    deps.into_iter().collect()
}

fn command_output_with_timeout(
    program: &str,
    args: &[&str],
    working_dir: &Path,
) -> Result<(bool, String, String, u64), ForgeError> {
    let start = Instant::now();
    let mut child = Command::new(program)
        .args(args)
        .current_dir(working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ForgeError::IoError(format!("Failed to spawn {}: {}", program, error)))?;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().map_err(|error| {
                    ForgeError::IoError(format!("Failed to collect {} output: {}", program, error))
                })?;
                return Ok((
                    output.status.success(),
                    String::from_utf8_lossy(&output.stdout).to_string(),
                    String::from_utf8_lossy(&output.stderr).to_string(),
                    start.elapsed().as_millis() as u64,
                ));
            }
            Ok(None) if start.elapsed() > Duration::from_secs(RUNNER_TIMEOUT_SECONDS) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ForgeError::ExecutionTimeout {
                    command: format!("{} {}", program, args.join(" ")),
                    timeout_secs: RUNNER_TIMEOUT_SECONDS,
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                return Err(ForgeError::IoError(format!(
                    "Failed while waiting for {}: {}",
                    program, error
                )));
            }
        }
    }
}

fn runner_result(success: bool, stdout: String, stderr: String, elapsed_ms: u64) -> ToolResult {
    let output = if stderr.trim().is_empty() {
        stdout
    } else if stdout.trim().is_empty() {
        stderr.clone()
    } else {
        format!("{}\n{}", stdout, stderr)
    };

    ToolResult {
        success,
        output: Some(output),
        error: (!success).then_some(ToolError::ExecutionFailed(stderr)),
        mutations: vec![],
        execution_time_ms: elapsed_ms,
    }
}

pub struct DependencyGraphTool;

impl DependencyGraphTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for DependencyGraphTool {
    fn name(&self) -> ToolName {
        ToolName::new("dependency_graph").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Build a bounded direct dependency graph for a source file."
    }

    fn allowed_in_mode(&self, _mode: ExecutionMode) -> bool {
        true
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        let start = Instant::now();
        let raw_path = PathBuf::from(args.require("path")?);
        if has_traversal_components(&raw_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components",
                raw_path.display()
            )));
        }
        let root = ctx
            .working_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.working_dir.clone());
        let start_path = validate_path_boundary(&raw_path, &ctx.working_dir)?;
        let max_depth = args
            .get("max_depth")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
            .min(3);

        let mut seen = BTreeSet::new();
        let mut edges = Vec::new();
        let mut queue = VecDeque::from([(start_path.clone(), 0_usize)]);

        while let Some((file, depth)) = queue.pop_front() {
            if seen.len() >= MAX_GRAPH_FILES || !seen.insert(file.clone()) || depth > max_depth {
                continue;
            }

            for dep in direct_dependencies(&file) {
                if !dep.starts_with(&root) {
                    continue;
                }
                edges.push(json!({
                    "from": relative_to(&file, &root),
                    "to": relative_to(&dep, &root),
                }));
                queue.push_back((dep, depth + 1));
            }
        }

        let files: Vec<String> = seen.iter().map(|path| relative_to(path, &root)).collect();
        let output = json!({
            "root": relative_to(&start_path, &root),
            "files": files,
            "edges": edges,
        })
        .to_string();

        Ok(ToolResult {
            success: true,
            output: Some(output),
            error: None,
            mutations: vec![],
            execution_time_ms: start.elapsed().as_millis() as u64,
        })
    }
}

pub struct SymbolIndexTool;

impl SymbolIndexTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for SymbolIndexTool {
    fn name(&self) -> ToolName {
        ToolName::new("symbol_index").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Find symbol-like locations matching a query across bounded source files."
    }

    fn allowed_in_mode(&self, _mode: ExecutionMode) -> bool {
        true
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        let start = Instant::now();
        let query = args.require("query")?;
        let raw_path = PathBuf::from(args.get("path").unwrap_or("."));
        if has_traversal_components(&raw_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components",
                raw_path.display()
            )));
        }
        let root = ctx
            .working_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.working_dir.clone());
        let search_root = validate_path_boundary(&raw_path, &ctx.working_dir)?;
        let max_results = args
            .get("max_results")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(50)
            .min(MAX_SYMBOL_RESULTS);

        let symbol_re = Regex::new(
            r"(?x)
            \b(fn|struct|enum|trait|impl|type|const|let|class|function|def|interface)\s+
            ([A-Za-z_][A-Za-z0-9_]*)",
        )
        .expect("valid symbol regex");

        let files = if search_root.is_file() {
            vec![search_root]
        } else {
            collect_source_files(&search_root, 2_000)
        };

        let mut results = Vec::new();
        for file in files {
            if results.len() >= max_results {
                break;
            }
            let Ok(content) = fs::read_to_string(&file) else {
                continue;
            };

            for (idx, line) in content.lines().enumerate() {
                if results.len() >= max_results {
                    break;
                }
                let Some(captures) = symbol_re.captures(line) else {
                    continue;
                };
                let symbol = captures
                    .get(2)
                    .map(|matched| matched.as_str())
                    .unwrap_or("");
                if symbol.contains(query) || line.contains(query) {
                    results.push(json!({
                        "path": relative_to(&file, &root),
                        "line": idx + 1,
                        "kind": captures.get(1).map(|matched| matched.as_str()).unwrap_or("symbol"),
                        "symbol": symbol,
                        "preview": line.trim(),
                    }));
                }
            }
        }

        Ok(ToolResult {
            success: true,
            output: Some(json!({ "query": query, "results": results }).to_string()),
            error: None,
            mutations: vec![],
            execution_time_ms: start.elapsed().as_millis() as u64,
        })
    }
}

pub struct EntryPointDetectorTool;

impl EntryPointDetectorTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for EntryPointDetectorTool {
    fn name(&self) -> ToolName {
        ToolName::new("entrypoint_detector").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Detect bounded project entry points such as Rust main/lib/bin files and package scripts."
    }

    fn allowed_in_mode(&self, _mode: ExecutionMode) -> bool {
        true
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        let start = Instant::now();
        let raw_path = PathBuf::from(args.get("path").unwrap_or("."));
        if has_traversal_components(&raw_path) {
            return Err(ForgeError::InvalidArgument(format!(
                "Path '{}' contains traversal components",
                raw_path.display()
            )));
        }
        let root = validate_path_boundary(&raw_path, &ctx.working_dir)?;
        let repo_root = ctx
            .working_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.working_dir.clone());
        let mut entries = Vec::new();

        if root.join("Cargo.toml").exists() {
            for path in [root.join("src/main.rs"), root.join("src/lib.rs")] {
                if path.exists() {
                    entries.push(json!({
                        "kind": "rust",
                        "path": relative_to(&path, &repo_root),
                    }));
                }
            }

            let bin_dir = root.join("src/bin");
            if let Ok(read_dir) = fs::read_dir(&bin_dir) {
                let mut bins: Vec<_> = read_dir.flatten().map(|entry| entry.path()).collect();
                bins.sort();
                for path in bins.into_iter().filter(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext == "rs")
                }) {
                    entries.push(json!({
                        "kind": "rust_bin",
                        "path": relative_to(&path, &repo_root),
                    }));
                }
            }
        }

        let package_json = root.join("package.json");
        if package_json.exists()
            && let Ok(content) = fs::read_to_string(&package_json)
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
        {
            if let Some(main) = value.get("main").and_then(|value| value.as_str()) {
                entries.push(json!({
                    "kind": "node_main",
                    "path": main,
                }));
            }
            if let Some(scripts) = value.get("scripts").and_then(|value| value.as_object()) {
                let mut script_names: Vec<_> = scripts.keys().collect();
                script_names.sort();
                for name in script_names {
                    if matches!(name.as_str(), "start" | "dev" | "build" | "test" | "lint") {
                        entries.push(json!({
                            "kind": "npm_script",
                            "name": name,
                            "command": scripts.get(name).and_then(|value| value.as_str()).unwrap_or(""),
                        }));
                    }
                }
            }
        }

        Ok(ToolResult {
            success: true,
            output: Some(json!({ "entrypoints": entries }).to_string()),
            error: None,
            mutations: vec![],
            execution_time_ms: start.elapsed().as_millis() as u64,
        })
    }
}

pub struct LintRunnerTool;

impl LintRunnerTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for LintRunnerTool {
    fn name(&self) -> ToolName {
        ToolName::new("lint_runner").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Run the repository lint policy for Rust or JavaScript projects."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        mode != ExecutionMode::Analysis
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        let root = validate_path_boundary(
            &PathBuf::from(args.get("path").unwrap_or(".")),
            &ctx.working_dir,
        )?;

        if root.join("Cargo.toml").exists() {
            let (success, stdout, stderr, elapsed) = command_output_with_timeout(
                "cargo",
                &[
                    "clippy",
                    "--all-targets",
                    "--all-features",
                    "--",
                    "-D",
                    "warnings",
                ],
                &root,
            )?;
            return Ok(runner_result(success, stdout, stderr, elapsed));
        }

        if root.join("package.json").exists() {
            let (success, stdout, stderr, elapsed) =
                command_output_with_timeout("npm", &["run", "lint", "--if-present"], &root)?;
            return Ok(runner_result(success, stdout, stderr, elapsed));
        }

        Ok(ToolResult {
            success: true,
            output: Some("No supported lint policy detected".to_string()),
            error: None,
            mutations: vec![],
            execution_time_ms: 0,
        })
    }
}

pub struct TestRunnerTool;

impl TestRunnerTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for TestRunnerTool {
    fn name(&self) -> ToolName {
        ToolName::new("test_runner").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Run the repository test policy for Rust or JavaScript projects."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        mode != ExecutionMode::Analysis
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        let root = validate_path_boundary(
            &PathBuf::from(args.get("path").unwrap_or(".")),
            &ctx.working_dir,
        )?;

        if root.join("Cargo.toml").exists() {
            let (success, stdout, stderr, elapsed) =
                command_output_with_timeout("cargo", &["test", "--quiet"], &root)?;
            return Ok(runner_result(success, stdout, stderr, elapsed));
        }

        if root.join("package.json").exists() {
            let (success, stdout, stderr, elapsed) =
                command_output_with_timeout("npm", &["test", "--if-present"], &root)?;
            return Ok(runner_result(success, stdout, stderr, elapsed));
        }

        Ok(ToolResult {
            success: true,
            output: Some("No supported test policy detected".to_string()),
            error: None,
            mutations: vec![],
            execution_time_ms: 0,
        })
    }
}
