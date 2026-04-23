//! FORGE Validation Engine
//!
//! Runs a staged validation pipeline with structured results:
//! - Stage 1: Syntax validation
//! - Stage 2: Format validation
//! - Stage 3: Lint validation
//! - Stage 4: Build validation
//! - Stage 5: Test validation

use crate::types::{
    Mutation, ValidationDecision, ValidationReport, ValidationStage as ReportValidationStage,
    ValidationStageResult,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Validation stage identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStage {
    Syntax = 1,
    Format = 2,
    Lint = 3,
    Build = 4,
    Test = 5,
}

impl std::fmt::Display for ValidationStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax => write!(f, "syntax"),
            Self::Format => write!(f, "format"),
            Self::Lint => write!(f, "lint"),
            Self::Build => write!(f, "build"),
            Self::Test => write!(f, "test"),
        }
    }
}

/// Result from a single validation stage
#[derive(Debug, Clone)]
pub struct StageResult {
    pub stage: ValidationStage,
    pub passed: bool,
    pub skipped: bool,
    pub execution_time_ms: u64,
    #[allow(dead_code)]
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    #[allow(dead_code)]
    pub artifacts: HashMap<String, String>,
}

impl StageResult {
    fn passed(stage: ValidationStage, elapsed_ms: u64, stdout: String, stderr: String) -> Self {
        Self {
            stage,
            passed: true,
            skipped: false,
            execution_time_ms: elapsed_ms,
            exit_code: Some(0),
            stdout,
            stderr,
            artifacts: HashMap::new(),
        }
    }

    fn failed(
        stage: ValidationStage,
        elapsed_ms: u64,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    ) -> Self {
        Self {
            stage,
            passed: false,
            skipped: false,
            execution_time_ms: elapsed_ms,
            exit_code,
            stdout,
            stderr,
            artifacts: HashMap::new(),
        }
    }

    fn skipped(stage: ValidationStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            passed: true,
            skipped: true,
            execution_time_ms: 0,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: message.into(),
            artifacts: HashMap::new(),
        }
    }
}

/// Overall validation outcome
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    Accept,
    Reject {
        reason: String,
        failed_stage: ValidationStage,
    },
    #[allow(dead_code)]
    Escalate {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct ValidationRun {
    pub outcome: ValidationOutcome,
    pub stage_results: Vec<StageResult>,
}

/// The FORGE Validation Engine
pub struct ValidationEngine {
    syntax_checks: HashMap<String, bool>, // extension -> syntax validation enabled
    auto_revert: bool,
}

impl ValidationEngine {
    pub fn new() -> Self {
        let mut syntax_checks = HashMap::new();

        syntax_checks.insert("py".to_string(), true);
        syntax_checks.insert("js".to_string(), true);

        for ext in ["ts", "tsx", "jsx", "mjs", "cjs"] {
            syntax_checks.insert(ext.to_string(), true);
        }

        syntax_checks.insert("rs".to_string(), true);
        syntax_checks.insert("sh".to_string(), true);
        syntax_checks.insert("txt".to_string(), false);

        Self {
            syntax_checks,
            auto_revert: true,
        }
    }

    /// Configure auto-revert behavior
    pub fn with_auto_revert(mut self, auto_revert: bool) -> Self {
        self.auto_revert = auto_revert;
        self
    }

    /// Validate a set of mutations through all configured stages
    pub fn validate_detailed(&self, mutations: &[Mutation], working_dir: &Path) -> ValidationRun {
        let mut stage_results = Vec::new();

        let syntax_result = self.run_syntax_stage(mutations, working_dir);
        let syntax_failure =
            (!syntax_result.skipped && !syntax_result.passed).then(|| syntax_result.stderr.clone());
        stage_results.push(syntax_result);
        if let Some(reason) = syntax_failure {
            return ValidationRun {
                outcome: ValidationOutcome::Reject {
                    reason,
                    failed_stage: ValidationStage::Syntax,
                },
                stage_results,
            };
        }

        let format_result = self.run_format_stage(mutations, working_dir);
        let format_failure =
            (!format_result.skipped && !format_result.passed).then(|| format_result.stderr.clone());
        stage_results.push(format_result);
        if let Some(reason) = format_failure {
            return ValidationRun {
                outcome: ValidationOutcome::Reject {
                    reason,
                    failed_stage: ValidationStage::Format,
                },
                stage_results,
            };
        }

        let lint_result = self.run_lint_stage(mutations, working_dir);
        let lint_failure =
            (!lint_result.skipped && !lint_result.passed).then(|| lint_result.stderr.clone());
        stage_results.push(lint_result);
        if let Some(reason) = lint_failure {
            return ValidationRun {
                outcome: ValidationOutcome::Reject {
                    reason,
                    failed_stage: ValidationStage::Lint,
                },
                stage_results,
            };
        }

        let build_result = self.run_build_stage(mutations, working_dir);
        let build_failure =
            (!build_result.skipped && !build_result.passed).then(|| build_result.stderr.clone());
        stage_results.push(build_result);
        if let Some(reason) = build_failure {
            return ValidationRun {
                outcome: ValidationOutcome::Reject {
                    reason,
                    failed_stage: ValidationStage::Build,
                },
                stage_results,
            };
        }

        let test_result = self.run_test_stage(mutations, working_dir);
        let test_failure =
            (!test_result.skipped && !test_result.passed).then(|| test_result.stderr.clone());
        stage_results.push(test_result);
        if let Some(reason) = test_failure {
            return ValidationRun {
                outcome: ValidationOutcome::Reject {
                    reason,
                    failed_stage: ValidationStage::Test,
                },
                stage_results,
            };
        }

        ValidationRun {
            outcome: ValidationOutcome::Accept,
            stage_results,
        }
    }

    fn run_syntax_stage(&self, mutations: &[Mutation], working_dir: &Path) -> StageResult {
        if mutations.is_empty() {
            return StageResult::skipped(
                ValidationStage::Syntax,
                "Syntax stage skipped: no mutations present",
            );
        }

        let stage_start = Instant::now();
        let mut validated_files = 0_u32;
        let mut skipped_files = 0_u32;
        let mut failure_messages = Vec::new();

        for mutation in mutations {
            let result = self.validate_syntax(mutation, working_dir);
            if result.skipped {
                skipped_files += 1;
                continue;
            }

            validated_files += 1;
            if !result.passed {
                let detail = if result.stderr.trim().is_empty() {
                    result.stdout.trim()
                } else {
                    result.stderr.trim()
                };
                failure_messages.push(format!("{}: {}", mutation.path.display(), detail));
            }
        }

        let elapsed_ms = stage_start.elapsed().as_millis() as u64;
        if !failure_messages.is_empty() {
            return StageResult::failed(
                ValidationStage::Syntax,
                elapsed_ms,
                Some(1),
                String::new(),
                format!("Syntax validation failed: {}", failure_messages.join(" | ")),
            );
        }

        if validated_files == 0 {
            return StageResult::skipped(
                ValidationStage::Syntax,
                "Syntax stage skipped: no supported validators for mutation set",
            );
        }

        StageResult::passed(
            ValidationStage::Syntax,
            elapsed_ms,
            format!("Validated syntax for {} file(s)", validated_files),
            if skipped_files > 0 {
                format!(
                    "Skipped syntax validation for {} unsupported file(s)",
                    skipped_files
                )
            } else {
                "Syntax checks passed".to_string()
            },
        )
    }

    fn run_format_stage(&self, mutations: &[Mutation], working_dir: &Path) -> StageResult {
        if self.contains_extension(mutations, "rs")
            && let Some(project_root) = find_upward(working_dir, "Cargo.toml")
        {
            return self.run_command(
                ValidationStage::Format,
                PathBuf::from("cargo"),
                vec!["fmt".to_string(), "--check".to_string()],
                &project_root,
                "cargo fmt --check passed",
                "Cargo fmt not available, format stage skipped",
            );
        }

        if self.contains_any_extension(mutations, &["js", "ts", "tsx", "jsx"])
            && let Some(project_root) = self.resolve_node_project_root(working_dir, mutations)
            && has_package_script(&project_root, "format:check")
        {
            return self.run_command(
                ValidationStage::Format,
                PathBuf::from("npm"),
                vec![
                    "run".to_string(),
                    "format:check".to_string(),
                    "--if-present".to_string(),
                ],
                &project_root,
                "npm format:check passed",
                "npm not available, format stage skipped",
            );
        }

        StageResult::skipped(
            ValidationStage::Format,
            "Format stage skipped: no supported formatter policy detected",
        )
    }

    fn run_lint_stage(&self, mutations: &[Mutation], working_dir: &Path) -> StageResult {
        if self.contains_extension(mutations, "rs")
            && let Some(project_root) = find_upward(working_dir, "Cargo.toml")
        {
            return self.run_command(
                ValidationStage::Lint,
                PathBuf::from("cargo"),
                vec![
                    "clippy".to_string(),
                    "--all-targets".to_string(),
                    "--all-features".to_string(),
                    "--".to_string(),
                    "-D".to_string(),
                    "warnings".to_string(),
                ],
                &project_root,
                "cargo clippy passed",
                "Cargo clippy not available, lint stage skipped",
            );
        }

        if self.contains_any_extension(mutations, &["js", "ts", "tsx", "jsx"])
            && let Some(project_root) = self.resolve_node_project_root(working_dir, mutations)
        {
            if has_package_script(&project_root, "lint") {
                return self.run_command(
                    ValidationStage::Lint,
                    PathBuf::from("npm"),
                    vec![
                        "run".to_string(),
                        "lint".to_string(),
                        "--if-present".to_string(),
                    ],
                    &project_root,
                    "npm lint passed",
                    "npm not available, lint stage skipped",
                );
            }

            if let Some(eslint) = find_eslint_binary(&project_root) {
                return self.run_command(
                    ValidationStage::Lint,
                    eslint,
                    vec![".".to_string()],
                    &project_root,
                    "eslint passed",
                    "eslint not available, lint stage skipped",
                );
            }
        }

        StageResult::skipped(
            ValidationStage::Lint,
            "Lint stage skipped: no supported linter policy detected",
        )
    }

    fn run_build_stage(&self, mutations: &[Mutation], working_dir: &Path) -> StageResult {
        if self.contains_extension(mutations, "rs")
            && let Some(project_root) = find_upward(working_dir, "Cargo.toml")
        {
            return self.run_command(
                ValidationStage::Build,
                PathBuf::from("cargo"),
                vec!["build".to_string(), "--quiet".to_string()],
                &project_root,
                "cargo build passed",
                "Cargo not available, build stage skipped",
            );
        }

        if self.contains_any_extension(mutations, &["ts", "tsx", "jsx"])
            && let Some(project_root) = self.resolve_node_project_root(working_dir, mutations)
        {
            if let Some(tsconfig_path) = find_upward(&project_root, "tsconfig.json")
                && let Some(tsc) = find_tsc_binary(&project_root)
            {
                return self.run_command(
                    ValidationStage::Build,
                    tsc,
                    vec![
                        "--noEmit".to_string(),
                        "--pretty".to_string(),
                        "false".to_string(),
                        "-p".to_string(),
                        tsconfig_path.display().to_string(),
                    ],
                    &project_root,
                    "TypeScript build validation passed",
                    "tsc not available, build stage skipped",
                );
            }

            if has_package_script(&project_root, "build") {
                return self.run_command(
                    ValidationStage::Build,
                    PathBuf::from("npm"),
                    vec![
                        "run".to_string(),
                        "build".to_string(),
                        "--if-present".to_string(),
                    ],
                    &project_root,
                    "npm build passed",
                    "npm not available, build stage skipped",
                );
            }
        }

        if self.contains_extension(mutations, "py")
            && let Some(project_root) = self.resolve_python_project_root(working_dir, mutations)
        {
            return self.run_command(
                ValidationStage::Build,
                PathBuf::from("python"),
                vec![
                    "-m".to_string(),
                    "compileall".to_string(),
                    "-q".to_string(),
                    ".".to_string(),
                ],
                &project_root,
                "Python compileall passed",
                "Python not available, build stage skipped",
            );
        }

        StageResult::skipped(
            ValidationStage::Build,
            "Build stage skipped: no supported project build detected",
        )
    }

    fn run_test_stage(&self, mutations: &[Mutation], working_dir: &Path) -> StageResult {
        if self.contains_extension(mutations, "rs")
            && let Some(project_root) = find_upward(working_dir, "Cargo.toml")
        {
            return self.run_command(
                ValidationStage::Test,
                PathBuf::from("cargo"),
                vec!["test".to_string(), "--quiet".to_string()],
                &project_root,
                "cargo test passed",
                "Cargo not available, test stage skipped",
            );
        }

        if self.contains_any_extension(mutations, &["js", "ts", "tsx", "jsx"])
            && let Some(project_root) = self.resolve_node_project_root(working_dir, mutations)
            && has_package_script(&project_root, "test")
        {
            return self.run_command(
                ValidationStage::Test,
                PathBuf::from("npm"),
                vec!["test".to_string(), "--if-present".to_string()],
                &project_root,
                "npm test passed",
                "npm not available, test stage skipped",
            );
        }

        if self.contains_extension(mutations, "py")
            && let Some(project_root) = self.resolve_python_project_root(working_dir, mutations)
            && python_pytest_available(&project_root)
        {
            return self.run_command(
                ValidationStage::Test,
                PathBuf::from("python"),
                vec!["-m".to_string(), "pytest".to_string(), "-q".to_string()],
                &project_root,
                "pytest passed",
                "pytest unavailable, test stage skipped",
            );
        }

        StageResult::skipped(
            ValidationStage::Test,
            "Test stage skipped: no supported project tests detected",
        )
    }

    /// Stage 1: Syntax validation using language-specific parsers
    fn validate_syntax(&self, mutation: &Mutation, working_dir: &Path) -> StageResult {
        let path = resolve_mutation_file_path(&mutation.path, working_dir);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if !self.syntax_checks.get(&ext).copied().unwrap_or(false) {
            return StageResult::skipped(
                ValidationStage::Syntax,
                format!("Syntax check skipped for .{} files", ext),
            );
        }

        if !path.exists() {
            return StageResult::failed(
                ValidationStage::Syntax,
                0,
                Some(-1),
                String::new(),
                format!("File not found: {}", path.display()),
            );
        }

        match ext.as_str() {
            "py" => self.run_python_syntax_check(&path),
            "js" | "mjs" | "cjs" => self.run_javascript_syntax_check(&path),
            "ts" | "tsx" | "jsx" => self.run_typescript_syntax_check(&path, working_dir),
            "rs" => self.run_rust_syntax_check(&path, working_dir),
            "sh" => self.run_shell_syntax_check(&path),
            _ => StageResult::skipped(
                ValidationStage::Syntax,
                format!("Unknown extension .{} - syntax check skipped", ext),
            ),
        }
    }

    fn run_python_syntax_check(&self, path: &Path) -> StageResult {
        let start = Instant::now();
        let mut spawn_errors = Vec::new();

        for program in ["python3", "python"] {
            match Command::new(program)
                .arg("-m")
                .arg("py_compile")
                .arg(path)
                .output()
            {
                Ok(output) => {
                    return stage_result_from_output(
                        ValidationStage::Syntax,
                        start,
                        output,
                        "Python syntax valid",
                    );
                }
                Err(error) => {
                    spawn_errors.push(format!("{}: {}", program, error));
                }
            }
        }

        StageResult::failed(
            ValidationStage::Syntax,
            start.elapsed().as_millis() as u64,
            Some(-1),
            String::new(),
            format!(
                "Python interpreter unavailable; cannot validate syntax for {} ({})",
                path.display(),
                spawn_errors.join("; ")
            ),
        )
    }

    fn run_javascript_syntax_check(&self, path: &Path) -> StageResult {
        let start = Instant::now();
        match Command::new("node").arg("--check").arg(path).output() {
            Ok(output) => stage_result_from_output(
                ValidationStage::Syntax,
                start,
                output,
                "JavaScript syntax valid",
            ),
            Err(error) => StageResult::failed(
                ValidationStage::Syntax,
                start.elapsed().as_millis() as u64,
                Some(-1),
                String::new(),
                format!(
                    "Node unavailable; cannot validate syntax for {}: {}",
                    path.display(),
                    error
                ),
            ),
        }
    }

    fn run_typescript_syntax_check(&self, path: &Path, working_dir: &Path) -> StageResult {
        let project_root = self
            .resolve_node_project_root(
                working_dir,
                &[Mutation {
                    path: path.to_path_buf(),
                    mutation_type: crate::types::MutationType::Write,
                    content_hash_before: None,
                    content_hash_after: None,
                }],
            )
            .or_else(|| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| working_dir.to_path_buf());

        let tsc = find_tsc_binary(&project_root).unwrap_or_else(|| PathBuf::from("tsc"));

        let args = if let Some(tsconfig_path) = find_upward(&project_root, "tsconfig.json") {
            vec![
                "--noEmit".to_string(),
                "--pretty".to_string(),
                "false".to_string(),
                "-p".to_string(),
                tsconfig_path.display().to_string(),
            ]
        } else {
            vec![
                "--noEmit".to_string(),
                "--pretty".to_string(),
                "false".to_string(),
                path.display().to_string(),
            ]
        };

        self.run_required_command(
            ValidationStage::Syntax,
            tsc,
            args,
            &project_root,
            "TypeScript syntax valid",
            "tsc unavailable; cannot validate TypeScript syntax",
        )
    }

    fn run_rust_syntax_check(&self, path: &Path, working_dir: &Path) -> StageResult {
        if let Some(project_root) = path
            .parent()
            .and_then(|parent| find_upward(parent, "Cargo.toml"))
            .or_else(|| find_upward(working_dir, "Cargo.toml"))
        {
            return self.run_required_command(
                ValidationStage::Syntax,
                PathBuf::from("cargo"),
                vec!["check".to_string(), "--quiet".to_string()],
                &project_root,
                "cargo check passed",
                "Cargo unavailable; cannot validate Rust syntax",
            );
        }

        let start = Instant::now();
        match Command::new("rustc")
            .args([
                "--crate-type",
                "lib",
                "--emit",
                "metadata",
                path.to_str().unwrap_or(""),
            ])
            .current_dir(path.parent().unwrap_or(working_dir))
            .output()
        {
            Ok(output) => stage_result_from_output(
                ValidationStage::Syntax,
                start,
                output,
                "rustc metadata check passed",
            ),
            Err(error) => StageResult::failed(
                ValidationStage::Syntax,
                start.elapsed().as_millis() as u64,
                Some(-1),
                String::new(),
                format!(
                    "rustc unavailable; cannot validate syntax for {}: {}",
                    path.display(),
                    error
                ),
            ),
        }
    }

    fn run_shell_syntax_check(&self, path: &Path) -> StageResult {
        let start = Instant::now();
        match Command::new("sh").arg("-n").arg(path).output() {
            Ok(output) => stage_result_from_output(
                ValidationStage::Syntax,
                start,
                output,
                "Shell syntax valid",
            ),
            Err(error) => StageResult::failed(
                ValidationStage::Syntax,
                start.elapsed().as_millis() as u64,
                Some(-1),
                String::new(),
                format!(
                    "Shell unavailable; cannot validate syntax for {}: {}",
                    path.display(),
                    error
                ),
            ),
        }
    }

    fn run_required_command(
        &self,
        stage: ValidationStage,
        program: PathBuf,
        args: Vec<String>,
        working_dir: &Path,
        success_message: &str,
        missing_message: &str,
    ) -> StageResult {
        let start = Instant::now();
        match Command::new(&program)
            .args(args.iter().map(String::as_str))
            .current_dir(working_dir)
            .output()
        {
            Ok(output) => stage_result_from_output(stage, start, output, success_message),
            Err(error) => StageResult::failed(
                stage,
                start.elapsed().as_millis() as u64,
                Some(-1),
                String::new(),
                format!("{}: {}", missing_message, error),
            ),
        }
    }

    fn run_command(
        &self,
        stage: ValidationStage,
        program: PathBuf,
        args: Vec<String>,
        working_dir: &Path,
        success_message: &str,
        missing_message: &str,
    ) -> StageResult {
        let start = Instant::now();
        match Command::new(&program)
            .args(args.iter().map(String::as_str))
            .current_dir(working_dir)
            .output()
        {
            Ok(output) => stage_result_from_output(stage, start, output, success_message),
            Err(error) => StageResult::skipped(stage, format!("{}: {}", missing_message, error)),
        }
    }

    fn contains_extension(&self, mutations: &[Mutation], extension: &str) -> bool {
        self.contains_any_extension(mutations, &[extension])
    }

    fn contains_any_extension(&self, mutations: &[Mutation], extensions: &[&str]) -> bool {
        mutations.iter().any(|mutation| {
            mutation
                .path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    extensions
                        .iter()
                        .any(|candidate| ext.eq_ignore_ascii_case(candidate))
                })
        })
    }

    fn resolve_node_project_root(
        &self,
        working_dir: &Path,
        mutations: &[Mutation],
    ) -> Option<PathBuf> {
        for mutation in mutations {
            if let Some(parent) = mutation.path.parent() {
                if let Some(root) = find_upward(parent, "package.json") {
                    return Some(root);
                }
                if let Some(root) = find_upward(parent, "tsconfig.json") {
                    return Some(root);
                }
            }
        }

        find_upward(working_dir, "package.json")
            .or_else(|| find_upward(working_dir, "tsconfig.json"))
    }

    fn resolve_python_project_root(
        &self,
        working_dir: &Path,
        mutations: &[Mutation],
    ) -> Option<PathBuf> {
        for mutation in mutations {
            if let Some(parent) = mutation.path.parent()
                && let Some(root) = find_first_upward(parent, &["pyproject.toml", "setup.py"])
            {
                return Some(root);
            }
        }

        find_first_upward(working_dir, &["pyproject.toml", "setup.py"]).or_else(|| {
            working_dir
                .join("tests")
                .exists()
                .then(|| working_dir.to_path_buf())
        })
    }

    /// Generate ValidationReport for runtime integration
    pub fn generate_report(
        &self,
        run: &ValidationRun,
        _mutations: &[Mutation],
    ) -> ValidationReport {
        let (decision, message) = match &run.outcome {
            ValidationOutcome::Accept => (
                ValidationDecision::Accept,
                "All validation stages passed".to_string(),
            ),
            ValidationOutcome::Reject { reason, .. } => {
                (ValidationDecision::Reject, reason.clone())
            }
            ValidationOutcome::Escalate { reason } => {
                (ValidationDecision::Escalate, reason.clone())
            }
        };

        let stage_results = run
            .stage_results
            .iter()
            .map(|result| ValidationStageResult {
                stage: to_report_stage(result.stage),
                passed: result.passed && !result.skipped,
                message: if result.skipped {
                    result.stderr.clone()
                } else if result.stderr.is_empty() {
                    result.stdout.clone()
                } else {
                    result.stderr.clone()
                },
                execution_time_ms: result.execution_time_ms,
            })
            .collect();

        ValidationReport {
            decision,
            stage_results,
            message,
            requires_revert: matches!(run.outcome, ValidationOutcome::Reject { .. })
                && self.auto_revert,
        }
    }
}

fn stage_result_from_output(
    stage: ValidationStage,
    start: Instant,
    output: std::process::Output,
    success_message: &str,
) -> StageResult {
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        StageResult::passed(
            stage,
            elapsed_ms,
            if stdout.is_empty() {
                success_message.to_string()
            } else {
                stdout
            },
            if stderr.is_empty() {
                success_message.to_string()
            } else {
                stderr
            },
        )
    } else {
        StageResult::failed(
            stage,
            elapsed_ms,
            output.status.code(),
            stdout,
            if stderr.is_empty() {
                format!("{} failed", stage)
            } else {
                stderr
            },
        )
    }
}

fn to_report_stage(stage: ValidationStage) -> ReportValidationStage {
    match stage {
        ValidationStage::Syntax => ReportValidationStage::Syntax,
        ValidationStage::Format => ReportValidationStage::Format,
        ValidationStage::Lint => ReportValidationStage::Lint,
        ValidationStage::Build => ReportValidationStage::Build,
        ValidationStage::Test => ReportValidationStage::Test,
    }
}

fn resolve_mutation_file_path(path: &Path, working_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    let working_dir_candidate = working_dir.join(path);
    if working_dir_candidate.exists() {
        return working_dir_candidate;
    }

    path.to_path_buf()
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

fn find_first_upward(start: &Path, markers: &[&str]) -> Option<PathBuf> {
    markers.iter().find_map(|marker| find_upward(start, marker))
}

fn find_tsc_binary(project_root: &Path) -> Option<PathBuf> {
    let local_tsc = project_root.join("node_modules/.bin/tsc");
    if local_tsc.exists() {
        return Some(local_tsc);
    }

    Some(PathBuf::from("tsc"))
}

fn find_eslint_binary(project_root: &Path) -> Option<PathBuf> {
    let local_eslint = project_root.join("node_modules/.bin/eslint");
    if local_eslint.exists() {
        return Some(local_eslint);
    }

    None
}

fn has_package_script(project_root: &Path, script_name: &str) -> bool {
    let package_json = project_root.join("package.json");
    let content = match std::fs::read_to_string(package_json) {
        Ok(content) => content,
        Err(_) => return false,
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(json) => json,
        Err(_) => return false,
    };

    json.get("scripts")
        .and_then(|scripts| scripts.get(script_name))
        .and_then(|script| script.as_str())
        .is_some()
}

fn python_pytest_available(project_root: &Path) -> bool {
    if !(project_root.join("tests").exists()
        || project_root.join("pytest.ini").exists()
        || project_root.join("pyproject.toml").exists())
    {
        return false;
    }

    Command::new("python")
        .args(["-m", "pytest", "--version"])
        .current_dir(project_root)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

impl Default for ValidationEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{ValidationEngine, ValidationOutcome, ValidationStage, find_upward};
    use crate::types::{Mutation, MutationType};
    use tempfile::TempDir;

    fn mutation(path: &std::path::Path) -> Mutation {
        Mutation {
            path: path.to_path_buf(),
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: Some("hash".to_string()),
        }
    }

    #[test]
    fn test_python_syntax_valid() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("test.py");
        std::fs::write(&file, "print('hello')\n").expect("write file");

        let engine = ValidationEngine::new();
        let result = engine.validate_syntax(&mutation(&file), temp.path());
        assert!(result.passed);
        assert!(!result.skipped);
    }

    #[test]
    fn test_python_syntax_invalid() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("test.py");
        std::fs::write(&file, "print('hello'\n").expect("write file");

        let engine = ValidationEngine::new();
        let result = engine.validate_syntax(&mutation(&file), temp.path());
        assert!(!result.passed);
        assert!(!result.skipped);
    }

    #[test]
    fn test_text_file_skips_syntax() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("test.txt");
        std::fs::write(&file, "any content here\n").expect("write file");

        let engine = ValidationEngine::new();
        let result = engine.validate_syntax(&mutation(&file), temp.path());
        assert!(result.skipped);
    }

    #[test]
    fn syntax_stage_validates_root_level_python_from_relative_mutation() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("app.py");
        std::fs::write(&file, "print('hello root')\n").expect("write file");

        let engine = ValidationEngine::new();
        let run = engine.validate_detailed(&[Mutation::write("app.py")], temp.path());
        let syntax = run
            .stage_results
            .iter()
            .find(|result| result.stage == ValidationStage::Syntax)
            .expect("syntax stage");

        assert!(matches!(run.outcome, ValidationOutcome::Accept));
        assert!(syntax.passed);
        assert!(!syntax.skipped);
    }

    #[test]
    fn syntax_stage_validates_nested_python_from_relative_mutation() {
        let temp = TempDir::new().expect("tempdir");
        let nested = temp.path().join("src");
        std::fs::create_dir_all(&nested).expect("create nested");
        std::fs::write(nested.join("app.py"), "print('hello nested')\n").expect("write file");

        let engine = ValidationEngine::new();
        let run = engine.validate_detailed(&[Mutation::write("src/app.py")], temp.path());
        let syntax = run
            .stage_results
            .iter()
            .find(|result| result.stage == ValidationStage::Syntax)
            .expect("syntax stage");

        assert!(matches!(run.outcome, ValidationOutcome::Accept));
        assert!(syntax.passed);
        assert!(!syntax.skipped);
    }

    #[test]
    fn syntax_stage_rejects_invalid_python_before_completion() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("broken.py");
        std::fs::write(&file, "def broken(:\n    pass\n").expect("write file");

        let engine = ValidationEngine::new();
        let run = engine.validate_detailed(&[mutation(&file)], temp.path());

        assert!(matches!(
            run.outcome,
            ValidationOutcome::Reject {
                failed_stage: ValidationStage::Syntax,
                ..
            }
        ));
    }

    #[test]
    fn single_file_cli_script_validates_without_project_scaffold() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("cli.py");
        std::fs::write(
            &file,
            concat!(
                "import argparse\n\n",
                "def main():\n",
                "    parser = argparse.ArgumentParser()\n",
                "    parser.add_argument('--name')\n",
                "    args = parser.parse_args()\n",
                "    print(args.name)\n\n",
                "if __name__ == \"__main__\":\n",
                "    main()\n"
            ),
        )
        .expect("write file");

        let engine = ValidationEngine::new();
        let run = engine.validate_detailed(&[mutation(&file)], temp.path());
        let syntax = run
            .stage_results
            .iter()
            .find(|result| result.stage == ValidationStage::Syntax)
            .expect("syntax stage");

        assert!(matches!(run.outcome, ValidationOutcome::Accept));
        assert!(syntax.passed);
        assert!(!syntax.skipped);
    }

    #[test]
    fn shell_script_syntax_is_validated_by_file_type() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("run.sh");
        std::fs::write(&file, "#!/bin/sh\nset -eu\necho hello\n").expect("write file");

        let engine = ValidationEngine::new();
        let run = engine.validate_detailed(&[mutation(&file)], temp.path());
        let syntax = run
            .stage_results
            .iter()
            .find(|result| result.stage == ValidationStage::Syntax)
            .expect("syntax stage");

        assert!(matches!(run.outcome, ValidationOutcome::Accept));
        assert!(syntax.passed);
        assert!(!syntax.skipped);
    }

    #[test]
    fn validation_run_exposes_stage_results() {
        let temp = TempDir::new().expect("tempdir");
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, "hello\n").expect("write file");

        let engine = ValidationEngine::new();
        let run = engine.validate_detailed(&[mutation(&file)], temp.path());

        assert!(matches!(run.outcome, ValidationOutcome::Accept));
        assert_eq!(run.stage_results.len(), 5);
        assert_eq!(run.stage_results[0].stage, ValidationStage::Syntax);
        assert_eq!(run.stage_results[1].stage, ValidationStage::Format);
    }

    #[test]
    fn rust_validation_uses_real_cargo_project_checks() {
        let temp = TempDir::new().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"validation_smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        let rust_file = temp.path().join("src/lib.rs");
        std::fs::write(&rust_file, "pub fn broken( {\n").expect("write bad rust");

        let engine = ValidationEngine::new();
        let run = engine.validate_detailed(&[mutation(&rust_file)], temp.path());

        assert!(matches!(
            run.outcome,
            ValidationOutcome::Reject {
                failed_stage: ValidationStage::Syntax,
                ..
            }
        ));
    }

    #[test]
    fn find_upward_discovers_project_root() {
        let temp = TempDir::new().expect("tempdir");
        let nested = temp.path().join("a/b/c");
        std::fs::create_dir_all(&nested).expect("create nested dirs");
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
        )
        .expect("write cargo");

        assert_eq!(
            find_upward(&nested, "Cargo.toml"),
            Some(temp.path().to_path_buf())
        );
    }
}
