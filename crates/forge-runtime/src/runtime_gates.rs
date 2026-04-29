//! FORGE Runtime Gates - Hard Enforcement Boundaries
//!
//! Implements the critical enforcement gates per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md:
//! - CompletionGate: State-aware completion validation
//! - ReadBeforeWriteGate: File mutation authority verification
//!
//! These gates are fail-closed and block execution before any mutation occurs.

use crate::explicit_artifact_contract::{
    ExplicitArtifactContract, normalize_path_text as normalize_contract_path_text,
    path_matches_expected as contract_path_matches_expected,
};
use crate::planner::ValidationFailureClass;
use crate::state::AgentState;
use crate::types::{CompletionReason, ValidationDecision, ValidationReport, ValidationStage};
use std::path::{Path, PathBuf};

/// ============================================================================
/// COMPLETION GATE - State-Aware Completion Enforcement
/// ============================================================================
///
/// Result of completion gate evaluation
#[derive(Debug, Clone)]
pub enum CompletionGateResult {
    Accept,
    Reject {
        reason: String,
        failure_class: ValidationFailureClass,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionReadiness {
    Ready { reason: String },
    NotReady { reason: String },
}

/// Validates completion requests against observable state
///
/// Per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 6.2:
/// - Task must be demonstrably satisfied by observable state
/// - At least one tool call must have been executed
/// - No pending validation
/// - No known blocking errors
/// - Reason must be state-justified
pub struct CompletionGate;

impl CompletionGate {
    pub fn readiness(
        state: &AgentState,
        pending_validation: bool,
        known_errors: &[String],
    ) -> CompletionReadiness {
        if pending_validation {
            return CompletionReadiness::NotReady {
                reason: "validation is still pending".to_string(),
            };
        }

        let blocking_errors = known_errors
            .iter()
            .filter(|error| !is_nonessential_post_success_error(error))
            .collect::<Vec<_>>();
        if !blocking_errors.is_empty() {
            return CompletionReadiness::NotReady {
                reason: format!("{} known error(s) remain", blocking_errors.len()),
            };
        }

        if state.change_history.is_empty() && state.files_written.is_empty() {
            return CompletionReadiness::NotReady {
                reason: "no write evidence has been recorded".to_string(),
            };
        }

        let Some(validation_report) = state.last_validation_report.as_ref() else {
            return CompletionReadiness::NotReady {
                reason: "no accepted validation report is available".to_string(),
            };
        };

        if validation_report.decision != ValidationDecision::Accept {
            return CompletionReadiness::NotReady {
                reason: "no accepted validation report is available".to_string(),
            };
        }

        let code_validation_targets = written_code_validation_targets(state);
        if !code_validation_targets.is_empty() && !has_passed_syntax_validation(validation_report) {
            return CompletionReadiness::NotReady {
                reason: format!(
                    "code artifacts require passed syntax validation: {}",
                    code_validation_targets.join(", ")
                ),
            };
        }

        if let Some(contract) = ExplicitArtifactContract::from_task(&state.task) {
            let status = contract.evaluate(&written_paths_buf(state));
            if !status.missing_paths.is_empty() || !status.empty_paths.is_empty() {
                return CompletionReadiness::NotReady {
                    reason: format_explicit_contract_incomplete_reason(&contract, &status),
                };
            }
            if !status.unexpected_paths.is_empty() {
                return CompletionReadiness::NotReady {
                    reason: format!(
                        "explicit artifact contract produced unexpected artifact(s): {}",
                        status.unexpected_paths.join(", ")
                    ),
                };
            }

            return CompletionReadiness::Ready {
                reason: format!(
                    "Created all {} required artifact(s): {}",
                    contract.required_deliverable_count(),
                    contract.required_paths.join(", ")
                ),
            };
        }

        if let Some(spec) = LiteralArtifactSpec::from_task(&state.task) {
            let reason = format!(
                "Created {} with the requested literal artifact content",
                spec.target_path
            );
            if let Some(reject_reason) = spec.rejection_reason(&reason, state) {
                return CompletionReadiness::NotReady {
                    reason: reject_reason,
                };
            }

            return CompletionReadiness::Ready { reason };
        }

        if let Some(spec) = ComplexImplementationSpec::from_task(&state.task) {
            if let Some(reject_reason) = spec.shallow_completion_rejection(state) {
                return CompletionReadiness::NotReady {
                    reason: reject_reason,
                };
            }

            let written = written_paths(state).join(", ");
            let surfaces = spec
                .required_surfaces
                .iter()
                .map(|surface| surface.name)
                .collect::<Vec<_>>()
                .join(", ");
            return CompletionReadiness::Ready {
                reason: format!(
                    "Completed {} with required surfaces [{}] in [{}]",
                    spec.task_kind, surfaces, written
                ),
            };
        }

        if task_requests_validation_finalization(&state.task)
            && has_passed_requested_validation(&state.task, validation_report, state)
            && !state.files_written.is_empty()
        {
            let validation_summary = if task_requests_cargo_check(&state.task) {
                "cargo check and cargo test passed"
            } else {
                "cargo test passed"
            };
            return CompletionReadiness::Ready {
                reason: format!(
                    "Validated requested changes in [{}]; {}",
                    written_paths(state).join(", "),
                    validation_summary
                ),
            };
        }

        CompletionReadiness::NotReady {
            reason: "no deterministic post-success completion rule matched this task".to_string(),
        }
    }

    /// Evaluate completion request
    pub fn evaluate(
        reason: &CompletionReason,
        state: &AgentState,
        pending_validation: bool,
        known_errors: &[String],
    ) -> CompletionGateResult {
        let reason_str = reason.as_str();

        // Check 1: At least one tool call executed (iteration > 0 evidence)
        if state.change_history.is_empty() && state.iteration == 0 {
            return CompletionGateResult::Reject {
                reason: "Completion rejected: no tool execution evidence".to_string(),
                failure_class: ValidationFailureClass::CompletionWithoutEvidence,
            };
        }

        // Check 2: No pending validation
        if pending_validation {
            return CompletionGateResult::Reject {
                reason: "Completion rejected: validation pending".to_string(),
                failure_class: ValidationFailureClass::CompletionWithPendingValidation,
            };
        }

        // Check 3: No known blocking errors
        if !known_errors.is_empty() {
            return CompletionGateResult::Reject {
                reason: format!(
                    "Completion rejected: {} known error(s) remain unaddressed",
                    known_errors.len()
                ),
                failure_class: ValidationFailureClass::CompletionWithKnownErrors,
            };
        }

        // Check 4: Reason must be state-justified (not vague)
        if Self::is_vague_reason(reason_str) {
            return CompletionGateResult::Reject {
                reason: format!(
                    "Completion rejected: reason '{}' is vague/non-state-based",
                    reason_str
                ),
                failure_class: ValidationFailureClass::VagueCompletionReason,
            };
        }

        // Check 5: Reason must reference observable state (files, lines, hashes)
        if !Self::references_observable_state(reason_str) {
            return CompletionGateResult::Reject {
                reason: "Completion rejected: reason does not reference observable state"
                    .to_string(),
                failure_class: ValidationFailureClass::VagueCompletionReason,
            };
        }

        if let Some(contract) = ExplicitArtifactContract::from_task(&state.task) {
            let status = contract.evaluate(&written_paths_buf(state));
            if !status.missing_paths.is_empty() || !status.empty_paths.is_empty() {
                return CompletionGateResult::Reject {
                    reason: format_explicit_contract_incomplete_reason(&contract, &status),
                    failure_class: ValidationFailureClass::VagueCompletionReason,
                };
            }
            if !status.unexpected_paths.is_empty() {
                return CompletionGateResult::Reject {
                    reason: format!(
                        "Completion rejected: explicit artifact contract produced unexpected artifact(s): {}",
                        status.unexpected_paths.join(", ")
                    ),
                    failure_class: ValidationFailureClass::VagueCompletionReason,
                };
            }
            if !contract.is_reason_contract_aware(reason_str) {
                return CompletionGateResult::Reject {
                    reason: format!(
                        "Completion rejected: explicit artifact contract completion reason must cite the required document set or at least one required filename ({})",
                        contract.required_paths.join(", ")
                    ),
                    failure_class: ValidationFailureClass::VagueCompletionReason,
                };
            }

            return CompletionGateResult::Accept;
        }

        if let Some(spec) = LiteralArtifactSpec::from_task(&state.task)
            && let Some(reject_reason) = spec.rejection_reason(reason_str, state)
        {
            return CompletionGateResult::Reject {
                reason: reject_reason,
                failure_class: ValidationFailureClass::VagueCompletionReason,
            };
        }

        if LiteralArtifactSpec::from_task(&state.task).is_none()
            && let Some(spec) = ComplexImplementationSpec::from_task(&state.task)
            && let Some(reject_reason) = spec.shallow_completion_rejection(state)
        {
            return CompletionGateResult::Reject {
                reason: reject_reason,
                failure_class: ValidationFailureClass::VagueCompletionReason,
            };
        }

        CompletionGateResult::Accept
    }

    /// Check if reason is vague/premature
    fn is_vague_reason(reason: &str) -> bool {
        let vague_patterns = [
            "looks good",
            "done",
            "complete",
            "finished",
            "ready",
            "task complete",
            "all done",
            "will be completed",
            "mostly done",
            "should be fine",
            "appears correct",
            "seems ok",
        ];

        let normalized = reason
            .to_lowercase()
            .chars()
            .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        vague_patterns.iter().any(|pattern| {
            normalized == *pattern
                || normalized.starts_with(&format!("{} ", pattern))
                || normalized.ends_with(&format!(" {}", pattern))
                || normalized.contains(&format!(" {} ", pattern))
        })
    }

    /// Check if reason references observable state
    fn references_observable_state(reason: &str) -> bool {
        // Must reference at least one of:
        // - File paths (e.g., "src/main.rs")
        // - Line numbers (e.g., "line 12")
        // - Function names
        // - Specific content changes

        let has_file_path = reason.contains('/') || reason.contains("\\");
        let has_file_name = Self::has_file_name_reference(reason);
        let has_line_number = reason.contains("line ") || reason.contains("Line ");
        let has_function_ref = reason.contains("fn ") || reason.contains("function");
        let has_specific_content = reason.contains("contains") || reason.contains("updated");

        has_file_path
            || has_file_name
            || has_line_number
            || has_function_ref
            || has_specific_content
    }

    fn has_file_name_reference(reason: &str) -> bool {
        const EXTENSIONS: &[&str] = &[
            ".md", ".txt", ".py", ".sh", ".js", ".ts", ".json", ".toml", ".rs", ".html", ".css",
        ];

        reason
            .split_whitespace()
            .map(|token| {
                token.trim_matches(|ch: char| {
                    !(ch.is_alphanumeric() || ch == '.' || ch == '_' || ch == '-' || ch == '/')
                })
            })
            .any(|token| {
                let lower = token.to_lowercase();
                EXTENSIONS
                    .iter()
                    .any(|extension| lower.ends_with(extension))
            })
    }
}

#[derive(Debug, Clone)]
struct LiteralArtifactSpec {
    target_path: String,
    artifact_class: Option<String>,
}

impl LiteralArtifactSpec {
    fn from_task(task: &str) -> Option<Self> {
        let target_path = extract_task_marker(task, "Target artifact:")?;
        let artifact_class = extract_task_marker(task, "Artifact class:");

        Some(Self {
            target_path,
            artifact_class,
        })
    }

    fn rejection_reason(&self, completion_reason: &str, state: &AgentState) -> Option<String> {
        if !self.reason_mentions_target(completion_reason) {
            return Some(format!(
                "Completion rejected: literal creation task must cite target artifact '{}' in the completion reason",
                self.target_path
            ));
        }

        if !self.target_was_written(state) {
            let written = written_paths(state);
            let artifact_class = self.artifact_class.as_deref().unwrap_or("literal artifact");
            return Some(format!(
                "Completion rejected: expected {} at '{}', but written artifact(s) were [{}]",
                artifact_class,
                self.target_path,
                written.join(", ")
            ));
        }

        if let Some(reason) = self.content_rejection_reason(state) {
            return Some(reason);
        }

        None
    }

    fn reason_mentions_target(&self, completion_reason: &str) -> bool {
        let reason = completion_reason.to_lowercase().replace('\\', "/");
        let expected = normalize_path_text(&self.target_path);
        let basename = Path::new(&self.target_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| expected.clone());

        reason.contains(&expected) || reason.contains(&basename)
    }

    fn target_was_written(&self, state: &AgentState) -> bool {
        state
            .files_written
            .iter()
            .any(|path| path_matches_expected(path, &self.target_path))
            || state
                .change_history
                .iter()
                .any(|record| path_matches_expected(&record.mutation.path, &self.target_path))
    }

    fn content_rejection_reason(&self, state: &AgentState) -> Option<String> {
        let artifact_class = self.artifact_class.as_deref()?;
        let content = self.read_written_target_content(state)?;
        let lower = content.to_lowercase();

        match artifact_class {
            "docs note" => {
                let repo_analysis_markers = [
                    "implementation location",
                    "source file",
                    "source files",
                    "src/",
                    "repository structure",
                    "project structure",
                ];
                if repo_analysis_markers
                    .iter()
                    .any(|marker| lower.contains(marker))
                {
                    return Some(format!(
                        "Completion rejected: '{}' must be a generic docs note, not repository implementation analysis",
                        self.target_path
                    ));
                }
            }
            "todo file" => {
                if !lower.contains("todo") && !lower.contains("[ ]") {
                    return Some(format!(
                        "Completion rejected: '{}' must contain TODO-list content",
                        self.target_path
                    ));
                }
            }
            "hello world script" => {
                if !lower.contains("hello, world") && !lower.contains("hello world") {
                    return Some(format!(
                        "Completion rejected: '{}' must contain hello-world script content",
                        self.target_path
                    ));
                }
            }
            _ => {}
        }

        None
    }

    fn read_written_target_content(&self, state: &AgentState) -> Option<String> {
        state
            .files_written
            .iter()
            .chain(
                state
                    .change_history
                    .iter()
                    .map(|record| &record.mutation.path),
            )
            .find(|path| path_matches_expected(path, &self.target_path))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .or_else(|| std::fs::read_to_string(&self.target_path).ok())
    }
}

#[derive(Debug, Clone)]
struct ComplexImplementationSpec {
    task_kind: &'static str,
    required_surfaces: Vec<RequiredSurface>,
}

#[derive(Debug, Clone, Copy)]
struct RequiredSurface {
    name: &'static str,
    kind: SurfaceKind,
}

#[derive(Debug, Clone, Copy)]
enum SurfaceKind {
    CliEntrypoint,
    CliArgumentFlow,
    AuthLogic,
    AuthIntegration,
    ApiRoute,
    ServerWiring,
    SiteStructure,
    SiteBuildOrServe,
    AppRuntime,
    PersistenceData,
    MigrationDefinition,
    MigrationRunner,
}

impl ComplexImplementationSpec {
    fn from_task(task: &str) -> Option<Self> {
        let normalized = normalize_task_text(task);
        if !has_creation_verb(&normalized) {
            return None;
        }

        if contains_any_task_term(
            &normalized,
            &["auth", "authentication", "authorization", "login", "oauth"],
        ) {
            return Some(Self::new(
                "auth system",
                &[
                    RequiredSurface::new("auth logic", SurfaceKind::AuthLogic),
                    RequiredSurface::new("integration surface", SurfaceKind::AuthIntegration),
                ],
            ));
        } else if contains_any_task_term(&normalized, &["rest api", "api", "endpoint", "server"]) {
            return Some(Self::new(
                "API",
                &[
                    RequiredSurface::new("route surface", SurfaceKind::ApiRoute),
                    RequiredSurface::new("server/app wiring", SurfaceKind::ServerWiring),
                ],
            ));
        } else if contains_any_task_term(
            &normalized,
            &["migration", "migrate", "schema", "database", "db"],
        ) {
            return Some(Self::new(
                "migration",
                &[
                    RequiredSurface::new("migration definition", SurfaceKind::MigrationDefinition),
                    RequiredSurface::new("runnable migration flow", SurfaceKind::MigrationRunner),
                ],
            ));
        } else if contains_any_task_term(&normalized, &["docs site", "site", "website"]) {
            return Some(Self::new(
                "site",
                &[
                    RequiredSurface::new("site structure", SurfaceKind::SiteStructure),
                    RequiredSurface::new("serving/build surface", SurfaceKind::SiteBuildOrServe),
                ],
            ));
        } else if contains_any_task_term(&normalized, &["persistence", "persistent", "persist"])
            && contains_any_task_term(&normalized, &["app", "application", "system", "tool"])
        {
            return Some(Self::new(
                "persistence app",
                &[
                    RequiredSurface::new("app/runtime surface", SurfaceKind::AppRuntime),
                    RequiredSurface::new("persistence/data surface", SurfaceKind::PersistenceData),
                ],
            ));
        } else if contains_any_task_term(&normalized, &["cli app", "cli", "command line"]) {
            return Some(Self::new(
                "CLI app",
                &[
                    RequiredSurface::new("entrypoint", SurfaceKind::CliEntrypoint),
                    RequiredSurface::new("runnable argument flow", SurfaceKind::CliArgumentFlow),
                ],
            ));
        } else if contains_any_task_term(
            &normalized,
            &["cli app", "app", "application", "tool", "tooling"],
        ) {
            return Some(Self::new(
                "application/tooling",
                &[RequiredSurface::new(
                    "app/runtime surface",
                    SurfaceKind::AppRuntime,
                )],
            ));
        } else if contains_any_task_term(
            &normalized,
            &[
                "architecture",
                "integrate",
                "integration",
                "persistence",
                "persistent",
                "persist",
                "wire",
                "wiring",
                "multi file",
                "multi-file",
                "multiple files",
                "system",
            ],
        ) {
            return Some(Self::new(
                "working system slice",
                &[RequiredSurface::new(
                    "implementation surface",
                    SurfaceKind::AppRuntime,
                )],
            ));
        } else {
            return None;
        }
    }

    fn new(task_kind: &'static str, required_surfaces: &[RequiredSurface]) -> Self {
        Self {
            task_kind,
            required_surfaces: required_surfaces.to_vec(),
        }
    }

    fn shallow_completion_rejection(&self, state: &AgentState) -> Option<String> {
        let artifacts = written_artifacts(state);
        if artifacts.is_empty() {
            return Some(format!(
                "Completion rejected: {} request requires implementation write evidence, but no written files were recorded",
                self.task_kind
            ));
        }

        if !artifacts
            .iter()
            .any(|artifact| is_implementation_surface(&artifact.path))
        {
            let written = written_paths(state);
            return Some(format!(
                "Completion rejected: {} request cannot be satisfied by shallow artifact(s) only [{}]; continue with source, config, runtime, API, persistence, migration, or site implementation work",
                self.task_kind,
                written.join(", ")
            ));
        }

        let missing_surfaces = self
            .required_surfaces
            .iter()
            .filter(|surface| {
                !artifacts
                    .iter()
                    .any(|artifact| surface.is_satisfied_by(artifact))
            })
            .map(|surface| surface.name)
            .collect::<Vec<_>>();

        if !missing_surfaces.is_empty() {
            let written = written_paths(state);
            let satisfied = self
                .required_surfaces
                .iter()
                .filter(|surface| {
                    artifacts
                        .iter()
                        .any(|artifact| surface.is_satisfied_by(artifact))
                })
                .map(|surface| surface.name)
                .collect::<Vec<_>>();
            let satisfied = if satisfied.is_empty() {
                "none".to_string()
            } else {
                satisfied.join(", ")
            };

            return Some(format!(
                "Completion rejected: {} request requires evidence for [{}], but written artifact(s) [{}] only satisfy [{}]; continue until all required implementation surfaces are present",
                self.task_kind,
                missing_surfaces.join(", "),
                written.join(", "),
                satisfied
            ));
        }

        None
    }
}

impl RequiredSurface {
    const fn new(name: &'static str, kind: SurfaceKind) -> Self {
        Self { name, kind }
    }

    fn is_satisfied_by(&self, artifact: &WrittenArtifact) -> bool {
        let content = artifact.content_lower.as_deref().unwrap_or("");

        match self.kind {
            SurfaceKind::CliEntrypoint => {
                artifact.extension == "py"
                    && (content.contains("if __name__ == \"__main__\"")
                        || content.contains("if __name__ == '__main__'")
                        || (artifact.file_name == "cli.py" && content.contains("def main"))
                        || (artifact.file_name == "main.py" && content.contains("def main")))
            }
            SurfaceKind::CliArgumentFlow => content_contains_any(
                content,
                &[
                    "argparse",
                    "parser.add_argument",
                    "sys.argv",
                    "click.command",
                    "@click",
                    "typer.",
                    "typer(",
                ],
            ),
            SurfaceKind::AuthLogic => {
                path_contains_any(artifact, &["auth", "login", "session"])
                    || content_contains_any(
                        content,
                        &[
                            "password",
                            "hash",
                            "jwt",
                            "token",
                            "session",
                            "bcrypt",
                            "verify_password",
                            "authenticate",
                        ],
                    )
            }
            SurfaceKind::AuthIntegration => {
                path_contains_any(
                    artifact,
                    &["route", "routes", "api", "middleware", "server"],
                ) || content_contains_any(
                    content,
                    &[
                        "@app.",
                        "router.",
                        "app.post",
                        "app.use",
                        "middleware",
                        "login",
                        "logout",
                        "depends(",
                        "route",
                    ],
                )
            }
            SurfaceKind::ApiRoute => {
                path_contains_any(artifact, &["api", "route", "routes"])
                    || content_contains_any(
                        content,
                        &[
                            "@app.get",
                            "@app.post",
                            "@router.",
                            "router.get",
                            "router.post",
                            "app.get(",
                            "app.post(",
                            "route(",
                        ],
                    )
            }
            SurfaceKind::ServerWiring => {
                path_contains_any(artifact, &["main", "server", "app"])
                    && content_contains_any(
                        content,
                        &[
                            "fastapi(",
                            "flask(",
                            "express()",
                            "app.listen",
                            "uvicorn",
                            "http.createserver",
                            "if __name__ == \"__main__\"",
                            "if __name__ == '__main__'",
                        ],
                    )
            }
            SurfaceKind::SiteStructure => {
                matches!(
                    artifact.extension.as_str(),
                    "html" | "css" | "js" | "md" | "mdx"
                ) && (path_contains_any(artifact, &["index", "site", "docs", "pages", "public"])
                    || content_contains_any(
                        content,
                        &["<html", "<main", "<nav", "doctype html", "export default"],
                    ))
            }
            SurfaceKind::SiteBuildOrServe => {
                matches!(
                    artifact.file_name.as_str(),
                    "package.json"
                        | "vite.config.js"
                        | "vite.config.ts"
                        | "mkdocs.yml"
                        | "docusaurus.config.js"
                        | "astro.config.mjs"
                ) || content_contains_any(
                    content,
                    &[
                        "\"scripts\"",
                        "\"build\"",
                        "\"dev\"",
                        "vite",
                        "mkdocs",
                        "serve",
                    ],
                )
            }
            SurfaceKind::AppRuntime => {
                is_implementation_surface(&artifact.path)
                    && (path_contains_any(artifact, &["app", "main", "server", "src"])
                        || content_contains_any(
                            content,
                            &[
                                "def main",
                                "function main",
                                "if __name__",
                                "class ",
                                "fastapi(",
                                "express()",
                                "react",
                                "render(",
                            ],
                        ))
            }
            SurfaceKind::PersistenceData => {
                path_contains_any(
                    artifact,
                    &["data", "db", "database", "storage", "store", "persistence"],
                ) || matches!(artifact.extension.as_str(), "sql" | "sqlite" | "db")
                    || content_contains_any(
                        content,
                        &[
                            "sqlite",
                            "database",
                            "db.",
                            "connect(",
                            "open(",
                            "json.dump",
                            "json.load",
                            "localstorage",
                            "persist",
                            "save_",
                            "load_",
                        ],
                    )
            }
            SurfaceKind::MigrationDefinition => {
                matches!(artifact.extension.as_str(), "sql" | "py" | "js" | "ts")
                    && content.contains("users")
                    && content_contains_any(
                        content,
                        &["create table", "alter table", "migration", "upgrade"],
                    )
            }
            SurfaceKind::MigrationRunner => content_contains_any(
                content,
                &[
                    "if __name__",
                    "def upgrade",
                    "def downgrade",
                    "alembic",
                    "execute(",
                    "cursor.execute",
                    "commit(",
                    "rollback",
                ],
            ),
        }
    }
}

fn extract_task_marker(task: &str, marker: &str) -> Option<String> {
    let start = task.find(marker)? + marker.len();
    let tail = &task[start..];
    let semicolon = tail.find(';').unwrap_or(tail.len());
    let newline = tail.find('\n').unwrap_or(tail.len());
    let end = semicolon.min(newline);
    let value = tail[..end]
        .trim()
        .trim_matches(|ch: char| ch == ',' || ch == '.')
        .trim();

    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn path_matches_expected(path: &Path, expected: &str) -> bool {
    contract_path_matches_expected(path, expected)
}

fn normalize_path_text(path: &str) -> String {
    normalize_contract_path_text(path)
}

fn written_paths_buf(state: &AgentState) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = state.files_written.iter().cloned().collect();
    if paths.is_empty() {
        paths.extend(
            state
                .change_history
                .iter()
                .map(|record| record.mutation.path.clone()),
        );
    }
    paths.sort();
    paths.dedup();
    paths
}

fn format_explicit_contract_incomplete_reason(
    contract: &ExplicitArtifactContract,
    status: &crate::explicit_artifact_contract::ExplicitArtifactStatus,
) -> String {
    let mut parts = vec![format!(
        "explicit artifact contract incomplete: {}/{} required artifact(s) are present",
        status.present_paths.len(),
        contract.required_deliverable_count()
    )];
    if !status.missing_paths.is_empty() {
        parts.push(format!("missing [{}]", status.missing_paths.join(", ")));
    }
    if !status.empty_paths.is_empty() {
        parts.push(format!("empty [{}]", status.empty_paths.join(", ")));
    }
    parts.join("; ")
}

fn written_paths(state: &AgentState) -> Vec<String> {
    let mut paths: Vec<String> = state
        .files_written
        .iter()
        .map(|path| path.display().to_string())
        .collect();

    if paths.is_empty() {
        paths.extend(
            state
                .change_history
                .iter()
                .map(|record| record.mutation.path.display().to_string()),
        );
    }

    if paths.is_empty() {
        paths.push("(none)".to_string());
    }

    paths.sort();
    paths
}

fn written_code_validation_targets(state: &AgentState) -> Vec<String> {
    const VALIDATED_CODE_EXTENSIONS: &[&str] =
        &["py", "rs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "sh"];

    let mut targets = written_artifacts(state)
        .into_iter()
        .filter(|artifact| VALIDATED_CODE_EXTENSIONS.contains(&artifact.extension.as_str()))
        .map(|artifact| artifact.path.display().to_string())
        .collect::<Vec<_>>();
    targets.sort();
    targets
}

fn has_passed_syntax_validation(report: &ValidationReport) -> bool {
    report
        .stage_results
        .iter()
        .any(|result| result.stage == ValidationStage::Syntax && result.passed)
}

fn has_passed_test_validation(report: &ValidationReport) -> bool {
    report
        .stage_results
        .iter()
        .any(|result| result.stage == ValidationStage::Test && result.passed)
}

fn has_passed_check_validation(report: &ValidationReport, state: &AgentState) -> bool {
    report.stage_results.iter().any(|result| {
        if !result.passed {
            return false;
        }

        let message = result.message.to_lowercase();
        let explicit_cargo_check = message.contains("cargo") && message.contains("check");
        let rust_syntax_check =
            result.stage == ValidationStage::Syntax && has_written_rust_artifact(state);

        explicit_cargo_check || rust_syntax_check
    })
}

fn has_passed_requested_validation(
    task: &str,
    report: &ValidationReport,
    state: &AgentState,
) -> bool {
    has_passed_test_validation(report)
        && (!task_requests_cargo_check(task) || has_passed_check_validation(report, state))
}

fn task_requests_validation_finalization(task: &str) -> bool {
    let task = task.to_lowercase();
    task.contains("complete only after cargo test passes")
        || task.contains("complete after cargo test passes")
        || task.contains("only after cargo test passes")
        || task.contains("complete only after cargo check and cargo test pass")
        || task.contains("complete after cargo check and cargo test pass")
        || task.contains("only after cargo check and cargo test pass")
}

fn task_requests_cargo_check(task: &str) -> bool {
    let task = task.to_lowercase();
    task.contains("cargo check")
        && (task.contains("cargo test pass") || task.contains("cargo test passes"))
}

fn has_written_rust_artifact(state: &AgentState) -> bool {
    written_artifacts(state)
        .iter()
        .any(|artifact| artifact.extension == "rs")
}

#[derive(Debug, Clone)]
struct WrittenArtifact {
    path: PathBuf,
    normalized_path: String,
    file_name: String,
    extension: String,
    content_lower: Option<String>,
}

fn written_artifacts(state: &AgentState) -> Vec<WrittenArtifact> {
    let mut paths: Vec<PathBuf> = state.files_written.iter().cloned().collect();

    if paths.is_empty() {
        paths.extend(
            state
                .change_history
                .iter()
                .map(|record| record.mutation.path.clone()),
        );
    }

    paths
        .into_iter()
        .map(|path| WrittenArtifact {
            normalized_path: normalize_path_text(&path.to_string_lossy()),
            file_name: path
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
            extension: path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_lowercase())
                .unwrap_or_default(),
            content_lower: std::fs::read_to_string(&path)
                .ok()
                .map(|content| content.to_lowercase()),
            path,
        })
        .collect()
}

fn is_implementation_surface(path: &Path) -> bool {
    let normalized = normalize_path_text(&path.to_string_lossy());
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if matches!(
        file_name.as_str(),
        "dockerfile" | "makefile" | "justfile" | "procfile"
    ) {
        return true;
    }

    if normalized.contains("/src/")
        || normalized.starts_with("src/")
        || normalized.contains("/app/")
        || normalized.starts_with("app/")
        || normalized.contains("/server/")
        || normalized.starts_with("server/")
        || normalized.contains("/api/")
        || normalized.starts_with("api/")
        || normalized.contains("/auth/")
        || normalized.starts_with("auth/")
        || normalized.contains("/migrations/")
        || normalized.starts_with("migrations/")
    {
        return true;
    }

    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_lowercase().as_str(),
                "py" | "js"
                    | "ts"
                    | "tsx"
                    | "jsx"
                    | "rs"
                    | "go"
                    | "java"
                    | "kt"
                    | "swift"
                    | "rb"
                    | "php"
                    | "cs"
                    | "cpp"
                    | "c"
                    | "h"
                    | "hpp"
                    | "sql"
                    | "sh"
                    | "bash"
                    | "zsh"
                    | "html"
                    | "css"
                    | "json"
                    | "toml"
                    | "yaml"
                    | "yml"
                    | "env"
                    | "ini"
                    | "cfg"
                    | "conf"
            )
        })
        .unwrap_or(false)
}

fn path_contains_any(artifact: &WrittenArtifact, terms: &[&str]) -> bool {
    terms
        .iter()
        .any(|term| artifact.normalized_path.contains(term) || artifact.file_name.contains(term))
}

fn content_contains_any(content: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| content.contains(term))
}

fn is_nonessential_post_success_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("hash mismatch")
        || lower.contains("text not found")
        || lower.contains("cardinality")
        || lower.contains("apply_patch")
        || lower.contains("patch")
}

fn normalize_task_text(statement: &str) -> String {
    statement
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_creation_verb(statement: &str) -> bool {
    statement.starts_with("build ")
        || statement.starts_with("create ")
        || statement.starts_with("make ")
        || statement.starts_with("write ")
        || statement.starts_with("add ")
        || statement.starts_with("implement ")
        || statement.starts_with("please build ")
        || statement.starts_with("please create ")
        || statement.starts_with("please make ")
        || statement.starts_with("please write ")
        || statement.starts_with("please implement ")
        || statement.starts_with("can you build ")
        || statement.starts_with("can you create ")
        || statement.starts_with("can you make ")
        || statement.starts_with("can you write ")
        || statement.starts_with("can you implement ")
}

fn contains_any_task_term(statement: &str, terms: &[&str]) -> bool {
    terms
        .iter()
        .any(|term| contains_normalized_task_term(statement, term))
}

fn contains_normalized_task_term(statement: &str, term: &str) -> bool {
    let tokens = statement.split_whitespace().collect::<Vec<_>>();
    let term_tokens = term.split_whitespace().collect::<Vec<_>>();

    match term_tokens.as_slice() {
        [] => false,
        [single] => tokens.iter().any(|token| token == single),
        phrase => tokens.windows(phrase.len()).any(|window| window == phrase),
    }
}

/// ============================================================================
/// READ-BEFORE-WRITE GATE - File Mutation Authority
/// ============================================================================
///
/// Result of read-before-write gate evaluation
#[derive(Debug, Clone)]
pub enum ReadBeforeWriteResult {
    Allow,
    Block {
        reason: String,
        failure_class: ValidationFailureClass,
        required_action: String,
    },
}

/// Validates file mutations against read authority
///
/// Per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 6.4:
/// - File must have been read in current session
/// - Read scope must cover mutation target
/// - Tracked hash must match current file hash
/// - Overwrite must not masquerade as create
pub struct ReadBeforeWriteGate;

impl ReadBeforeWriteGate {
    /// Evaluate mutation request
    pub fn evaluate(
        path: &Path,
        is_existing_file: bool,
        state: &AgentState,
        current_content: Option<&str>,
    ) -> ReadBeforeWriteResult {
        // Normalize path for lookup
        let normalized_path = normalize_path(path);

        // New file creation: no prior read required
        if !is_existing_file {
            return ReadBeforeWriteResult::Allow;
        }

        // Existing file mutation: read authority required

        // Check 1: Read record exists
        let file_record = match state.files_read.get(&normalized_path) {
            Some(r) => r,
            None => {
                return ReadBeforeWriteResult::Block {
                    reason: format!(
                        "Read-before-write violation: {} was never read in this session",
                        path.display()
                    ),
                    failure_class: ValidationFailureClass::WriteWithoutRead,
                    required_action: format!("read_file with path '{}'", path.display()),
                };
            }
        };

        // Check 2: Read scope is sufficient (full read required for full writes)
        if !file_record.is_full_read {
            return ReadBeforeWriteResult::Block {
                reason: format!(
                    "Read-before-write violation: {} was only partially read (lines {:?}), \
                     full read required for mutation",
                    path.display(),
                    file_record.lines_read
                ),
                failure_class: ValidationFailureClass::InsufficientReadScope,
                required_action: format!("read_file with path '{}' (full file)", path.display()),
            };
        }

        // Check 3: Hash freshness (if current content provided)
        if let Some(content) = current_content {
            let current_hash = crate::crypto_hash::compute_content_hash(content);
            if current_hash != file_record.content_hash {
                return ReadBeforeWriteResult::Block {
                    reason: format!(
                        "Read-before-write violation: {} changed after read \
                         (hash mismatch: expected {}, current {})",
                        path.display(),
                        &file_record.content_hash[..16.min(file_record.content_hash.len())],
                        &current_hash[..16.min(current_hash.len())]
                    ),
                    failure_class: ValidationFailureClass::StaleRead,
                    required_action: format!("re-read_file with path '{}'", path.display()),
                };
            }
        }

        ReadBeforeWriteResult::Allow
    }

    /// Check if path represents an existing file
    #[allow(dead_code)]
    pub fn is_existing_file(path: &Path) -> bool {
        path.exists()
    }
}

/// ============================================================================
/// PATH NORMALIZATION
/// ============================================================================
///
/// Normalize path for consistent identity comparison
///
/// Handles:
/// - Relative path components (./, ../)
/// - Trailing slashes
/// - Case sensitivity (preserves case, normalizes separators)
pub fn normalize_path(path: &Path) -> PathBuf {
    // Use std::fs::canonicalize if path exists
    if path.exists()
        && let Ok(canonical) = std::fs::canonicalize(path)
    {
        return canonical;
    }

    // Manual normalization for non-existent paths
    let mut components = vec![];
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                components.push(component.as_os_str().to_string_lossy().to_string());
            }
            std::path::Component::CurDir => {
                // Skip ./
            }
            std::path::Component::ParentDir => {
                // Handle ../
                if !components.is_empty() && components.last().unwrap() != "/" {
                    components.pop();
                }
            }
            std::path::Component::Normal(name) => {
                components.push(name.to_string_lossy().to_string());
            }
        }
    }

    if components.is_empty() {
        return PathBuf::from(".");
    }

    // Reconstruct path
    let mut result = PathBuf::new();
    for (i, comp) in components.iter().enumerate() {
        if i == 0 && (comp == "/" || comp.ends_with(':')) {
            // Root or Windows prefix
            result.push(comp);
        } else {
            result.push(comp);
        }
    }

    result
}

/// ============================================================================
/// GATE INTEGRATION HELPERS
/// ============================================================================
///
/// Comprehensive gate check result for runtime integration
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GateCheckReport {
    pub completion_gate: Option<CompletionGateResult>,
    pub read_before_write_gates: Vec<(PathBuf, ReadBeforeWriteResult)>,
    pub overall_decision: GateDecision,
    pub log_entries: Vec<GateLogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum GateDecision {
    Proceed,
    Block {
        reason: String,
        required_action: String,
    },
    Halt {
        reason: String,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GateLogEntry {
    pub gate_type: String,
    pub path: Option<PathBuf>,
    pub decision: String,
    pub reason: String,
}

/// Run all applicable gates for a planner output
#[allow(dead_code)]
pub fn run_gates(
    state: &AgentState,
    is_completion: bool,
    completion_reason: Option<&CompletionReason>,
    mutation_paths: &[PathBuf],
    pending_validation: bool,
    known_errors: &[String],
) -> GateCheckReport {
    let mut log_entries = vec![];
    let mut completion_gate_result = None;
    let mut rbw_results = vec![];

    // Run completion gate if applicable
    if is_completion && let Some(reason) = completion_reason {
        let result = CompletionGate::evaluate(reason, state, pending_validation, known_errors);

        log_entries.push(GateLogEntry {
            gate_type: "CompletionGate".to_string(),
            path: None,
            decision: match &result {
                CompletionGateResult::Accept => "Accept".to_string(),
                CompletionGateResult::Reject { .. } => "Reject".to_string(),
            },
            reason: match &result {
                CompletionGateResult::Accept => "Completion criteria satisfied".to_string(),
                CompletionGateResult::Reject { reason, .. } => reason.clone(),
            },
        });

        completion_gate_result = Some(result);
    }

    // Run read-before-write gate for each mutation path
    for path in mutation_paths {
        let is_existing = ReadBeforeWriteGate::is_existing_file(path);

        // Get current content if file exists (for hash check)
        let current_content = if is_existing {
            std::fs::read_to_string(path).ok()
        } else {
            None
        };

        let result =
            ReadBeforeWriteGate::evaluate(path, is_existing, state, current_content.as_deref());

        log_entries.push(GateLogEntry {
            gate_type: "ReadBeforeWriteGate".to_string(),
            path: Some(path.clone()),
            decision: match &result {
                ReadBeforeWriteResult::Allow => "Allow".to_string(),
                ReadBeforeWriteResult::Block { .. } => "Block".to_string(),
            },
            reason: match &result {
                ReadBeforeWriteResult::Allow => "Read authority confirmed".to_string(),
                ReadBeforeWriteResult::Block { reason, .. } => reason.clone(),
            },
        });

        rbw_results.push((path.clone(), result));
    }

    // Compute overall decision
    let overall_decision = compute_overall_decision(&completion_gate_result, &rbw_results);

    GateCheckReport {
        completion_gate: completion_gate_result,
        read_before_write_gates: rbw_results,
        overall_decision,
        log_entries,
    }
}

#[allow(dead_code)]
fn compute_overall_decision(
    completion: &Option<CompletionGateResult>,
    rbw_results: &[(PathBuf, ReadBeforeWriteResult)],
) -> GateDecision {
    // Check completion gate
    if let Some(CompletionGateResult::Reject { reason, .. }) = completion {
        return GateDecision::Block {
            reason: reason.clone(),
            required_action: "Continue with tool_call to make progress".to_string(),
        };
    }

    // Check read-before-write gates
    for (_path, result) in rbw_results {
        if let ReadBeforeWriteResult::Block {
            reason,
            required_action,
            ..
        } = result
        {
            return GateDecision::Block {
                reason: reason.clone(),
                required_action: required_action.clone(),
            };
        }
    }

    GateDecision::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CompletionReason, ExecutionMode, ValidationReport, ValidationStage, ValidationStageResult,
    };

    fn literal_task(target: &str, artifact_class: &str) -> String {
        format!(
            "create a tiny docs note file: Create the requested literal artifact. Target artifact: {}; Artifact class: {}; Content requirements: generic note.",
            target, artifact_class
        )
    }

    fn state_with_written_file(task: String, written_path: &str) -> AgentState {
        state_with_written_paths(task, &[PathBuf::from(written_path)])
    }

    fn state_with_written_paths(task: String, written_paths: &[PathBuf]) -> AgentState {
        let mut state = AgentState::new(5, task, ExecutionMode::Edit);
        state.iteration = 1;
        for written_path in written_paths {
            state.files_written.insert(written_path.clone());
        }
        state
    }

    fn validated_state_with_written_paths(task: String, written_paths: &[PathBuf]) -> AgentState {
        let mut state = state_with_written_paths(task, written_paths);
        state.last_validation_report = Some(accepted_validation_for_paths(written_paths));
        state
    }

    fn accepted_validation_for_paths(written_paths: &[PathBuf]) -> ValidationReport {
        let mut report = ValidationReport::accept("validation accepted");
        if written_paths.iter().any(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "py" | "rs" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "sh"
                    )
                })
        }) {
            report.stage_results.push(ValidationStageResult {
                stage: ValidationStage::Syntax,
                passed: true,
                message: "Validated syntax for code artifact(s)".to_string(),
                execution_time_ms: 0,
            });
        }
        report
    }

    fn accepted_cargo_check_and_test_validation() -> ValidationReport {
        let mut report = ValidationReport::accept("validation accepted");
        report.stage_results.push(ValidationStageResult {
            stage: ValidationStage::Syntax,
            passed: true,
            message: "cargo check passed".to_string(),
            execution_time_ms: 0,
        });
        report.stage_results.push(ValidationStageResult {
            stage: ValidationStage::Test,
            passed: true,
            message: "cargo test passed".to_string(),
            execution_time_ms: 0,
        });
        report
    }

    fn unique_temp_path(file_name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "forge-literal-{}-{}-{}",
            std::process::id(),
            nanos,
            file_name
        ))
    }

    fn write_temp_artifact(file_name: &str, content: &str) -> PathBuf {
        let path = unique_temp_path(file_name);
        std::fs::write(&path, content).expect("write temp artifact");
        path
    }

    fn write_relative_artifact(relative_path: &str, content: &str) -> PathBuf {
        let path = PathBuf::from(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create relative artifact parent");
        }
        std::fs::write(&path, content).expect("write relative artifact");
        path
    }

    fn unique_relative_dir(prefix: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        format!(
            "target/explicit-contract-tests/{}-{}-{}",
            prefix,
            std::process::id(),
            nanos
        )
    }

    fn explicit_contract_task(paths: &[&str]) -> String {
        let mut task = format!(
            "Create exactly {} markdown files with these precise filenames:\n",
            paths.len()
        );
        for (index, path) in paths.iter().enumerate() {
            task.push_str(&format!("{}. {}\n", index + 1, path));
        }
        task.push_str("All of these must be produced.");
        task
    }

    #[test]
    fn completion_gate_rejects_wrong_literal_artifact_path() {
        let state = state_with_written_file(
            literal_task("docs/tiny-note.md", "docs note"),
            "src/implementation-location.md",
        );
        let reason =
            CompletionReason::new("Created docs/tiny-note.md containing a tiny generic docs note");

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);

        match result {
            CompletionGateResult::Reject { reason, .. } => {
                assert!(reason.contains("docs/tiny-note.md"));
                assert!(reason.contains("src/implementation-location.md"));
            }
            CompletionGateResult::Accept => panic!("wrong artifact path must not complete"),
        }
    }

    #[test]
    fn completion_gate_accepts_expected_literal_artifact_path() {
        let state = state_with_written_file(
            literal_task("docs/tiny-note.md", "docs note"),
            "docs/tiny-note.md",
        );
        let reason =
            CompletionReason::new("Created docs/tiny-note.md containing a tiny generic docs note");

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);

        assert!(matches!(result, CompletionGateResult::Accept));
    }

    #[test]
    fn completion_gate_requires_reason_to_cite_literal_target() {
        let state = state_with_written_file(
            literal_task("docs/tiny-note.md", "docs note"),
            "docs/tiny-note.md",
        );
        let reason = CompletionReason::new("Created docs/note.md containing a generic docs note");

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);

        match result {
            CompletionGateResult::Reject { reason, .. } => {
                assert!(reason.contains("must cite target artifact"));
            }
            CompletionGateResult::Accept => panic!("completion reason must cite target artifact"),
        }
    }

    #[test]
    fn completion_gate_rejects_docs_note_with_repo_analysis_content() {
        let path = unique_temp_path("tiny-note.md");
        std::fs::write(
            &path,
            "# Implementation Location\n\nThis note references src/main.rs and repository structure.",
        )
        .expect("write temp docs note");

        let state = state_with_written_file(
            literal_task(&path.to_string_lossy(), "docs note"),
            &path.to_string_lossy(),
        );
        let reason = CompletionReason::new(&format!(
            "Created {} containing a tiny docs note",
            path.display()
        ));

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);
        let _ = std::fs::remove_file(&path);

        match result {
            CompletionGateResult::Reject { reason, .. } => {
                assert!(reason.contains("generic docs note"));
                assert!(reason.contains("repository implementation analysis"));
            }
            CompletionGateResult::Accept => {
                panic!("docs note with implementation analysis must not complete")
            }
        }
    }

    #[test]
    fn completion_gate_rejects_shallow_artifacts_for_complex_implementation_tasks() {
        for (task, shallow_path) in [
            ("build a Python CLI app", "README.md"),
            ("create an auth system", "docs/auth-system.md"),
            ("make a docs site", "docs/site-plan.md"),
            ("create a REST API", "docs/rest-api.md"),
            ("write a migration script for users table", "TODO.md"),
            (
                "build a small note-taking app with persistence",
                "docs/note-taking-app.md",
            ),
        ] {
            let state = state_with_written_file(task.to_string(), shallow_path);
            let reason = CompletionReason::new(&format!(
                "Created {} containing notes for the requested work",
                shallow_path
            ));

            let result = CompletionGate::evaluate(&reason, &state, false, &[]);

            match result {
                CompletionGateResult::Reject { reason, .. } => {
                    assert!(reason.contains("shallow artifact"));
                    assert!(reason.contains(shallow_path));
                }
                CompletionGateResult::Accept => {
                    panic!("complex task completed with shallow artifact: {}", task)
                }
            }
        }
    }

    #[test]
    fn completion_gate_rejects_single_surface_complex_implementations() {
        for (task, file_name, content, expected_missing) in [
            (
                "build a Python CLI app",
                "cli.py",
                "print('hello')\n",
                "runnable argument flow",
            ),
            (
                "create an auth system",
                "auth.py",
                "def hash_password(password):\n    return password\n",
                "integration surface",
            ),
            (
                "create a REST API",
                "api.py",
                "def list_users():\n    return []\n",
                "server/app wiring",
            ),
            (
                "make a docs site",
                "index.html",
                "<!doctype html><main>Docs</main>\n",
                "serving/build surface",
            ),
            (
                "write a migration script for users table",
                "users_migration.sql",
                "CREATE TABLE users (id INTEGER PRIMARY KEY);\n",
                "runnable migration flow",
            ),
            (
                "build a small note-taking app with persistence",
                "app.py",
                "def main():\n    notes = []\n    print(notes)\n",
                "persistence/data surface",
            ),
        ] {
            let path = write_temp_artifact(file_name, content);
            let state = state_with_written_paths(task.to_string(), std::slice::from_ref(&path));
            let reason = CompletionReason::new(&format!(
                "Created {} containing implementation code",
                path.display()
            ));

            let result = CompletionGate::evaluate(&reason, &state, false, &[]);
            let _ = std::fs::remove_file(&path);

            match result {
                CompletionGateResult::Reject { reason, .. } => {
                    assert!(
                        reason.contains(expected_missing),
                        "expected missing surface '{}' in reject reason: {}",
                        expected_missing,
                        reason
                    );
                }
                CompletionGateResult::Accept => {
                    panic!("complex task completed with one surface only: {}", task)
                }
            }
        }
    }

    #[test]
    fn completion_gate_accepts_complete_cli_surface_for_complex_task() {
        let path = write_temp_artifact(
            "cli.py",
            "import argparse\n\n\
             def main(argv=None):\n\
                 parser = argparse.ArgumentParser()\n\
                 parser.add_argument('--name', default='world')\n\
                 args = parser.parse_args(argv)\n\
                 print(f'Hello {args.name}')\n\n\
             if __name__ == \"__main__\":\n\
                 main()\n",
        );
        let state = state_with_written_paths(
            "build a Python CLI app".to_string(),
            std::slice::from_ref(&path),
        );
        let reason = CompletionReason::new(&format!(
            "Created {} containing an argparse CLI entrypoint and argument flow",
            path.display()
        ));

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);
        let _ = std::fs::remove_file(&path);

        assert!(matches!(result, CompletionGateResult::Accept));
    }

    #[test]
    fn completion_gate_literal_artifact_does_not_require_complex_surfaces() {
        let path = write_temp_artifact("tiny-note.md", "# Tiny Note\nA small generic note.\n");
        let state = state_with_written_paths(
            literal_task(&path.to_string_lossy(), "docs note"),
            std::slice::from_ref(&path),
        );
        let reason = CompletionReason::new(&format!(
            "Created {} containing a tiny generic docs note",
            path.display()
        ));

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);
        let _ = std::fs::remove_file(&path);

        assert!(matches!(result, CompletionGateResult::Accept));
    }

    #[test]
    fn completion_gate_rejects_partial_explicit_artifact_contract() {
        let dir = unique_relative_dir("partial");
        let first = format!("{}/docs/01_PROJECT_OVERVIEW.md", dir);
        let second = format!("{}/docs/02_ARCHITECTURE.md", dir);
        let first_path = write_relative_artifact(&first, "# Overview\n\nContract doc.\n");
        let task = explicit_contract_task(&[&first, &second]);
        let state = validated_state_with_written_paths(task, std::slice::from_ref(&first_path));
        let reason = CompletionReason::new(&format!(
            "Created {} with the requested markdown content",
            first
        ));

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);
        let readiness = CompletionGate::readiness(&state, false, &[]);
        let _ = std::fs::remove_dir_all(PathBuf::from(dir));

        match result {
            CompletionGateResult::Reject { reason, .. } => {
                assert!(reason.contains("explicit artifact contract incomplete"));
                assert!(reason.contains("02_ARCHITECTURE.md"));
            }
            CompletionGateResult::Accept => panic!("partial explicit contract must not complete"),
        }
        match readiness {
            CompletionReadiness::NotReady { reason } => {
                assert!(reason.contains("02_ARCHITECTURE.md"));
            }
            CompletionReadiness::Ready { reason } => {
                panic!("partial explicit contract should stay blocked: {}", reason)
            }
        }
    }

    #[test]
    fn completion_gate_accepts_full_explicit_artifact_contract() {
        let dir = unique_relative_dir("full");
        let first = format!("{}/docs/01_PROJECT_OVERVIEW.md", dir);
        let second = format!("{}/docs/02_ARCHITECTURE.md", dir);
        let first_path = write_relative_artifact(&first, "# Overview\n\nContract doc.\n");
        let second_path = write_relative_artifact(&second, "# Architecture\n\nContract doc.\n");
        let task = explicit_contract_task(&[&first, &second]);
        let state = validated_state_with_written_paths(task, &[first_path.clone(), second_path.clone()]);
        let reason = CompletionReason::new(&format!(
            "Created all 2 required artifact(s): {}, {}",
            first, second
        ));

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);
        let readiness = CompletionGate::readiness(&state, false, &[]);
        let _ = std::fs::remove_dir_all(PathBuf::from(dir));

        assert!(
            matches!(result, CompletionGateResult::Accept),
            "expected acceptance, got {:?}",
            result
        );
        assert!(
            matches!(readiness, CompletionReadiness::Ready { .. }),
            "expected readiness, got {:?}",
            readiness
        );
    }

    #[test]
    fn completion_gate_rejects_generic_reason_for_full_explicit_contract() {
        let dir = unique_relative_dir("reason");
        let first = format!("{}/docs/01_PROJECT_OVERVIEW.md", dir);
        let second = format!("{}/docs/02_ARCHITECTURE.md", dir);
        let first_path = write_relative_artifact(&first, "# Overview\n\nContract doc.\n");
        let second_path = write_relative_artifact(&second, "# Architecture\n\nContract doc.\n");
        let task = explicit_contract_task(&[&first, &second]);
        let state = validated_state_with_written_paths(task, &[first_path.clone(), second_path.clone()]);
        let reason = CompletionReason::new("Created README.md with updated markdown content");

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);
        let _ = std::fs::remove_dir_all(PathBuf::from(dir));

        match result {
            CompletionGateResult::Reject { reason, .. } => {
                assert!(reason.contains("must cite the required document set"));
            }
            CompletionGateResult::Accept => {
                panic!("explicit contract completion reason must remain contract-aware")
            }
        }
    }

    #[test]
    fn completion_readiness_accepts_valid_working_slice_after_validation() {
        let path = write_temp_artifact(
            "cli.py",
            "import argparse\n\n\
             def main():\n\
                 parser = argparse.ArgumentParser()\n\
                 parser.add_argument('--name')\n\
                 args = parser.parse_args()\n\
                 print(args.name)\n\n\
             if __name__ == \"__main__\":\n\
                 main()\n",
        );
        let state = validated_state_with_written_paths(
            "build a Python CLI app".to_string(),
            std::slice::from_ref(&path),
        );

        let result = CompletionGate::readiness(&state, false, &[]);
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(result, CompletionReadiness::Ready { .. }),
            "unexpected readiness result: {:?}",
            result
        );
    }

    #[test]
    fn completion_readiness_accepts_compile_repair_after_check_and_test() {
        let path = write_temp_artifact("lib.rs", "pub fn answer() -> i32 {\n    42\n}\n");
        let mut state = state_with_written_paths(
            "This project has a compile error. Read src/lib.rs, repair the compile error, and complete only after cargo check and cargo test pass.".to_string(),
            std::slice::from_ref(&path),
        );
        state.last_validation_report = Some(accepted_cargo_check_and_test_validation());

        let result = CompletionGate::readiness(&state, false, &[]);
        let _ = std::fs::remove_file(&path);

        match result {
            CompletionReadiness::Ready { reason } => {
                assert!(reason.contains("cargo check and cargo test passed"));
                assert!(reason.contains("lib.rs"));
            }
            CompletionReadiness::NotReady { reason } => {
                panic!(
                    "compile repair should finalize after check and test: {}",
                    reason
                )
            }
        }
    }

    #[test]
    fn completion_readiness_blocks_missing_surface_and_validation_failure() {
        let path = write_temp_artifact(
            "cli.py",
            "def main():\n    print('hello')\n\nif __name__ == \"__main__\":\n    main()\n",
        );
        let mut state = validated_state_with_written_paths(
            "build a Python CLI app".to_string(),
            std::slice::from_ref(&path),
        );

        let missing_surface = CompletionGate::readiness(&state, false, &[]);
        assert!(matches!(
            missing_surface,
            CompletionReadiness::NotReady { .. }
        ));

        state.last_validation_report = Some(ValidationReport::reject("syntax failed"));
        let validation_failed = CompletionGate::readiness(&state, false, &[]);
        let _ = std::fs::remove_file(&path);

        assert!(matches!(
            validation_failed,
            CompletionReadiness::NotReady { .. }
        ));
    }

    #[test]
    fn completion_readiness_blocks_code_artifact_without_syntax_evidence() {
        let path = write_temp_artifact(
            "cli.py",
            "import argparse\n\n\
             def main():\n\
                 parser = argparse.ArgumentParser()\n\
                 parser.add_argument('--name')\n\
                 args = parser.parse_args()\n\
                 print(args.name)\n\n\
             if __name__ == \"__main__\":\n\
                 main()\n",
        );
        let mut state = state_with_written_paths(
            "build a Python CLI app".to_string(),
            std::slice::from_ref(&path),
        );
        state.last_validation_report = Some(ValidationReport::accept("accepted without stages"));

        let result = CompletionGate::readiness(&state, false, &[]);
        let _ = std::fs::remove_file(&path);

        match result {
            CompletionReadiness::NotReady { reason } => {
                assert!(reason.contains("passed syntax validation"));
                assert!(reason.contains("cli.py"));
            }
            CompletionReadiness::Ready { reason } => {
                panic!(
                    "code artifact completed without syntax evidence: {}",
                    reason
                )
            }
        }
    }

    #[test]
    fn completion_readiness_allows_literal_markdown_without_runtime_validation() {
        let path = write_temp_artifact("tiny-note.md", "# Tiny Note\nA small generic note.\n");
        let mut state = state_with_written_paths(
            literal_task(&path.to_string_lossy(), "docs note"),
            std::slice::from_ref(&path),
        );
        state.last_validation_report = Some(ValidationReport::accept("markdown accepted"));

        let result = CompletionGate::readiness(&state, false, &[]);
        let _ = std::fs::remove_file(&path);

        assert!(matches!(result, CompletionReadiness::Ready { .. }));
    }

    #[test]
    fn completion_readiness_ignores_nonessential_patch_error_after_valid_slice() {
        let path = write_temp_artifact(
            "cli.py",
            "import argparse\n\n\
             def main():\n\
                 parser = argparse.ArgumentParser()\n\
                 parser.add_argument('--name')\n\
                 args = parser.parse_args()\n\
                 print(args.name)\n\n\
             if __name__ == \"__main__\":\n\
                 main()\n",
        );
        let state = validated_state_with_written_paths(
            "build a Python CLI app".to_string(),
            std::slice::from_ref(&path),
        );
        let known_errors =
            vec!["Tool failed: apply_patch hash mismatch while changing wording".to_string()];

        let result = CompletionGate::readiness(&state, false, &known_errors);
        let _ = std::fs::remove_file(&path);

        assert!(matches!(result, CompletionReadiness::Ready { .. }));
    }
}
