//! Validation pipeline - syntax, lint, build, test
//!
//! Each stage emits runtime events and updates the validation panel live.

use crate::host_actions::{HostAction, execute as execute_host_action};
use crate::persistence::PersistentState;
use crate::state::{RuntimeStatus, ValidationStage};
use anyhow::Result;
use std::path::PathBuf;
use tracing::{info, warn};

/// Validation pipeline runner
pub struct ValidationPipeline {
    pub stages: Vec<ValidationStage>,
    repo_path: String,
}

impl ValidationPipeline {
    pub fn new(repo_path: String) -> Self {
        Self {
            stages: vec![
                ValidationStage {
                    name: "syntax".to_string(),
                    status: RuntimeStatus::Idle,
                    detail: None,
                    duration_ms: None,
                },
                ValidationStage {
                    name: "lint".to_string(),
                    status: RuntimeStatus::Idle,
                    detail: None,
                    duration_ms: None,
                },
                ValidationStage {
                    name: "build".to_string(),
                    status: RuntimeStatus::Idle,
                    detail: None,
                    duration_ms: None,
                },
                ValidationStage {
                    name: "test".to_string(),
                    status: RuntimeStatus::Idle,
                    detail: None,
                    duration_ms: None,
                },
            ],
            repo_path,
        }
    }

    /// Run full validation pipeline with event callbacks
    pub async fn run<F>(&mut self, mut on_event: F) -> Result<()>
    where
        F: FnMut(&str, RuntimeStatus, Option<&str>),
    {
        info!("Starting validation pipeline for {}", self.repo_path);

        // Syntax check
        self.run_syntax_stage(&mut on_event).await?;

        // Lint check
        self.run_lint_stage(&mut on_event).await?;

        // Build check
        self.run_build_stage(&mut on_event).await?;

        // Test check
        self.run_test_stage(&mut on_event).await?;

        info!("Validation pipeline complete");
        Ok(())
    }

    async fn run_syntax_stage<F>(&mut self, on_event: &mut F) -> Result<()>
    where
        F: FnMut(&str, RuntimeStatus, Option<&str>),
    {
        let name = "syntax";
        let idx = 0;
        info!("Running validation stage: {}", name);
        on_event(name, RuntimeStatus::Running, None);

        let start = std::time::Instant::now();
        let repo_path = self.repo_path.clone();

        // Run check in blocking task
        let result = tokio::task::spawn_blocking(move || Self::check_syntax_static(&repo_path))
            .await
            .unwrap_or_else(|e| Err(format!("Task panicked: {}", e)));

        match result {
            Ok(detail) => {
                let duration = start.elapsed().as_millis() as u64;
                info!("Validation stage {} passed in {}ms", name, duration);

                self.stages[idx].status = RuntimeStatus::Completed;
                self.stages[idx].detail = Some(detail);
                self.stages[idx].duration_ms = Some(duration);

                on_event(
                    name,
                    RuntimeStatus::Completed,
                    Some(&format!("OK ({}ms)", duration)),
                );
                Ok(())
            }
            Err(error) => {
                let duration = start.elapsed().as_millis() as u64;
                warn!("Validation stage {} failed: {}", name, error);

                self.stages[idx].status = RuntimeStatus::Error;
                self.stages[idx].detail = Some(error.clone());
                self.stages[idx].duration_ms = Some(duration);

                on_event(name, RuntimeStatus::Error, Some(&error));
                Err(anyhow::anyhow!("Validation failed at {}: {}", name, error))
            }
        }
    }

    async fn run_lint_stage<F>(&mut self, on_event: &mut F) -> Result<()>
    where
        F: FnMut(&str, RuntimeStatus, Option<&str>),
    {
        let name = "lint";
        let idx = 1;
        info!("Running validation stage: {}", name);
        on_event(name, RuntimeStatus::Running, None);

        let start = std::time::Instant::now();
        let repo_path = self.repo_path.clone();

        let result = tokio::task::spawn_blocking(move || Self::check_lint_static(&repo_path))
            .await
            .unwrap_or_else(|e| Err(format!("Task panicked: {}", e)));

        match result {
            Ok(detail) => {
                let duration = start.elapsed().as_millis() as u64;
                self.stages[idx].status = RuntimeStatus::Completed;
                self.stages[idx].detail = Some(detail);
                self.stages[idx].duration_ms = Some(duration);
                on_event(
                    name,
                    RuntimeStatus::Completed,
                    Some(&format!("OK ({}ms)", duration)),
                );
                Ok(())
            }
            Err(error) => {
                let duration = start.elapsed().as_millis() as u64;
                self.stages[idx].status = RuntimeStatus::Error;
                self.stages[idx].detail = Some(error.clone());
                self.stages[idx].duration_ms = Some(duration);
                on_event(name, RuntimeStatus::Error, Some(&error));
                Err(anyhow::anyhow!("Validation failed at {}: {}", name, error))
            }
        }
    }

    async fn run_build_stage<F>(&mut self, on_event: &mut F) -> Result<()>
    where
        F: FnMut(&str, RuntimeStatus, Option<&str>),
    {
        let name = "build";
        let idx = 2;
        info!("Running validation stage: {}", name);
        on_event(name, RuntimeStatus::Running, None);

        let start = std::time::Instant::now();
        let repo_path = self.repo_path.clone();

        let result = tokio::task::spawn_blocking(move || Self::check_build_static(&repo_path))
            .await
            .unwrap_or_else(|e| Err(format!("Task panicked: {}", e)));

        match result {
            Ok(detail) => {
                let duration = start.elapsed().as_millis() as u64;
                self.stages[idx].status = RuntimeStatus::Completed;
                self.stages[idx].detail = Some(detail);
                self.stages[idx].duration_ms = Some(duration);
                on_event(
                    name,
                    RuntimeStatus::Completed,
                    Some(&format!("OK ({}ms)", duration)),
                );
                Ok(())
            }
            Err(error) => {
                let duration = start.elapsed().as_millis() as u64;
                self.stages[idx].status = RuntimeStatus::Error;
                self.stages[idx].detail = Some(error.clone());
                self.stages[idx].duration_ms = Some(duration);
                on_event(name, RuntimeStatus::Error, Some(&error));
                Err(anyhow::anyhow!("Validation failed at {}: {}", name, error))
            }
        }
    }

    async fn run_test_stage<F>(&mut self, on_event: &mut F) -> Result<()>
    where
        F: FnMut(&str, RuntimeStatus, Option<&str>),
    {
        let name = "test";
        let idx = 3;
        info!("Running validation stage: {}", name);
        on_event(name, RuntimeStatus::Running, None);

        let start = std::time::Instant::now();
        let repo_path = self.repo_path.clone();

        let result = tokio::task::spawn_blocking(move || Self::check_test_static(&repo_path))
            .await
            .unwrap_or_else(|e| Err(format!("Task panicked: {}", e)));

        match result {
            Ok(detail) => {
                let duration = start.elapsed().as_millis() as u64;
                self.stages[idx].status = RuntimeStatus::Completed;
                self.stages[idx].detail = Some(detail);
                self.stages[idx].duration_ms = Some(duration);
                on_event(
                    name,
                    RuntimeStatus::Completed,
                    Some(&format!("OK ({}ms)", duration)),
                );
                Ok(())
            }
            Err(error) => {
                let duration = start.elapsed().as_millis() as u64;
                self.stages[idx].status = RuntimeStatus::Error;
                self.stages[idx].detail = Some(error.clone());
                self.stages[idx].duration_ms = Some(duration);
                on_event(name, RuntimeStatus::Error, Some(&error));
                Err(anyhow::anyhow!("Validation failed at {}: {}", name, error))
            }
        }
    }

    /// Static version of syntax check for spawn_blocking
    fn check_syntax_static(repo_path: &str) -> Result<String, String> {
        // Check for Rust
        if std::path::Path::new(&format!("{}/Cargo.toml", repo_path)).exists() {
            return Self::check_rust_syntax_static(repo_path);
        }

        // Check for JavaScript/TypeScript
        if std::path::Path::new(&format!("{}/package.json", repo_path)).exists() {
            return Self::check_js_syntax_static(repo_path);
        }

        // Check for Python
        if std::path::Path::new(&format!("{}/requirements.txt", repo_path)).exists()
            || std::path::Path::new(&format!("{}/pyproject.toml", repo_path)).exists()
        {
            return Self::check_python_syntax_static(repo_path);
        }

        // Generic - just check if files exist
        Ok("No syntax checker available for this project type".to_string())
    }

    fn check_rust_syntax_static(repo_path: &str) -> Result<String, String> {
        match run_host_command(repo_path, "cargo check --message-format=short") {
            Ok(result) => Ok(result
                .output
                .as_deref()
                .filter(|output| !output.trim().is_empty())
                .map(|_| "No syntax errors".to_string())
                .unwrap_or_else(|| "No syntax errors".to_string())),
            Err(error) => Err(format!("Syntax errors: {}", error)),
        }
    }

    fn check_js_syntax_static(repo_path: &str) -> Result<String, String> {
        // Check if node_modules exists, if not just skip
        if !std::path::Path::new(&format!("{}/node_modules", repo_path)).exists() {
            return Ok("node_modules not present, skipping syntax check".to_string());
        }

        match run_host_command(repo_path, "npm run lint --silent") {
            Ok(_) => Ok("Lint passed".to_string()),
            Err(error) if error.contains("missing script") => {
                Ok("No linter configured".to_string())
            }
            Err(error) => Err(format!("Lint errors: {}", error)),
        }
    }

    fn check_python_syntax_static(repo_path: &str) -> Result<String, String> {
        // Try to run python -m py_compile on all .py files
        match run_host_command(
            repo_path,
            "find . -name '*.py' -exec python3 -m py_compile {} +",
        ) {
            Ok(_) => Ok("Python syntax OK".to_string()),
            Err(error) => Err(format!("Python syntax error: {}", error)),
        }
    }

    /// Static version of lint check
    fn check_lint_static(repo_path: &str) -> Result<String, String> {
        // For Rust, cargo check already does linting
        // For JS, npm run lint already does it
        // Check project type and return appropriate message
        if std::path::Path::new(&format!("{}/Cargo.toml", repo_path)).exists() {
            Ok("Rust: cargo check handles linting".to_string())
        } else if std::path::Path::new(&format!("{}/package.json", repo_path)).exists() {
            Ok("JS: npm run lint handles linting".to_string())
        } else {
            Ok("No additional linting configured".to_string())
        }
    }

    /// Static version of build check
    fn check_build_static(repo_path: &str) -> Result<String, String> {
        // Check for Rust
        if std::path::Path::new(&format!("{}/Cargo.toml", repo_path)).exists() {
            return match run_host_command(repo_path, "cargo build --release") {
                Ok(_) => Ok("Build successful".to_string()),
                Err(error) => Err(format!("Build failed: {}", error)),
            };
        }

        // Check for JS
        if std::path::Path::new(&format!("{}/package.json", repo_path)).exists() {
            return match run_host_command(repo_path, "npm run build") {
                Ok(_) => Ok("Build successful".to_string()),
                Err(error) if error.contains("missing script") => {
                    Ok("No build script configured".to_string())
                }
                Err(error) => Err(format!("Build failed: {}", error)),
            };
        }

        Ok("No build system detected".to_string())
    }

    /// Static version of test check
    fn check_test_static(repo_path: &str) -> Result<String, String> {
        // Check for Rust
        if std::path::Path::new(&format!("{}/Cargo.toml", repo_path)).exists() {
            return match run_host_command(repo_path, "cargo test --release") {
                Ok(result) => {
                    let stdout = result.output.unwrap_or_default();
                    let test_line = stdout
                        .lines()
                        .find(|l| l.contains("test result:"))
                        .unwrap_or("Tests passed");
                    Ok(test_line.to_string())
                }
                Err(error) => Err(format!("Tests failed: {}", error)),
            };
        }

        // Check for JS
        if std::path::Path::new(&format!("{}/package.json", repo_path)).exists() {
            return match run_host_command(repo_path, "npm test") {
                Ok(_) => Ok("Tests passed".to_string()),
                Err(error) if error.contains("missing script") => {
                    Ok("No test script configured".to_string())
                }
                Err(error) => Err(format!("Tests failed: {}", error)),
            };
        }

        // Check for Python
        if std::path::Path::new(&format!("{}/requirements.txt", repo_path)).exists()
            || std::path::Path::new(&format!("{}/pyproject.toml", repo_path)).exists()
        {
            return match run_host_command(repo_path, "python3 -m pytest -v") {
                Ok(result) => {
                    let stdout = result.output.unwrap_or_default();
                    let summary = stdout
                        .lines()
                        .find(|l| l.contains("passed") || l.contains("failed"))
                        .unwrap_or("Tests complete");
                    Ok(summary.to_string())
                }
                Err(error) if error.contains("No module named pytest") => {
                    Ok("pytest not available".to_string())
                }
                Err(error) => Err(format!("Tests failed: {}", error)),
            };
        }

        Ok("No test system detected".to_string())
    }
}

fn run_host_command(
    repo_path: &str,
    command: &str,
) -> Result<crate::host_actions::HostActionResult, String> {
    let mut persistence = PersistentState::new();
    let result = execute_host_action(
        HostAction::RunCommand {
            project_root: PathBuf::from(repo_path),
            command: command.to_string(),
        },
        &mut persistence,
    );

    if result.success {
        Ok(result)
    } else {
        Err(result.error.unwrap_or(result.summary))
    }
}
