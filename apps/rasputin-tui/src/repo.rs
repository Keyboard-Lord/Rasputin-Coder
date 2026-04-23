//! Repository management module

#[path = "../../../support/workspace_config.rs"]
mod workspace_config;

use crate::forge_runtime::{GitCommitSummary, GitFileStatus, GitGrounding};
use anyhow::Result;
use std::path::Path;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct Repo {
    pub name: String,
    pub path: String,
    pub display_path: String,
    pub git_branch: Option<String>,
    pub git_detected: bool,
    pub ollama_model: Option<String>,
    pub model_source: Option<String>,
}

impl Repo {
    /// Attach to a repo at the given path
    pub async fn attach(path: &str) -> Result<Self> {
        info!("Attaching to repo at: {}", path);

        // Check if path exists
        let path_obj = Path::new(path);
        if !path_obj.exists() {
            return Err(anyhow::anyhow!("Path does not exist: {}", path));
        }

        // Get repo name from path
        let name = path_obj
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Create display path (shorten home dir)
        let home = std::env::var("HOME").unwrap_or_default();
        let display_path = if path.starts_with(&home) {
            path.replacen(&home, "~", 1)
        } else {
            path.to_string()
        };

        // Detect git repository
        let git_detected = Self::detect_git(path);
        let git_branch = if git_detected {
            Self::get_git_branch(path)
        } else {
            None
        };

        let model_config = Self::read_config(path).await?;
        let ollama_model = model_config.as_ref().map(|config| config.model.clone());
        let model_source = model_config
            .as_ref()
            .map(|config| config.source.to_string());

        if let Some(ref model) = ollama_model {
            info!(
                "Repo configured for model: {} ({})",
                model,
                model_source.as_deref().unwrap_or("repo config")
            );
        } else {
            warn!("No planner model configured in repo");
        }

        Ok(Self {
            name,
            path: path.to_string(),
            display_path,
            git_branch,
            git_detected,
            ollama_model,
            model_source,
        })
    }

    /// Detect if path is a git repository
    fn detect_git(path: &str) -> bool {
        workspace_config::detect_git_repository(Path::new(path))
    }

    /// Get current git branch
    fn get_git_branch(path: &str) -> Option<String> {
        let output = std::process::Command::new("git")
            .args(["-C", path, "branch", "--show-current"])
            .output()
            .ok()?;

        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() {
                return Some(branch);
            }
        }
        None
    }

    /// Read repo-level model config from `.forge/config.yaml` or `rasputin.json`
    async fn read_config(path: &str) -> Result<Option<workspace_config::WorkspaceModelConfig>> {
        let repo_path = Path::new(path);
        let config = workspace_config::discover_workspace_model(repo_path);

        if let Some(ref config) = config {
            info!(
                "Loaded planner model from {}: {}",
                config.source, config.model
            );
        } else {
            debug!("No repo planner config found at {}", path);
        }

        Ok(config)
    }
}

/// Capture comprehensive Git grounding state for a repository path
pub fn capture_git_grounding(repo_path: &str) -> GitGrounding {
    // Check if this is actually a git repo
    let is_git_repo = std::process::Command::new("git")
        .args(["-C", repo_path, "rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git_repo {
        return GitGrounding::no_repo();
    }

    // Get branch name (None if detached HEAD)
    let branch_name = std::process::Command::new("git")
        .args(["-C", repo_path, "branch", "--show-current"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        });

    // Get HEAD commit hash
    let head_commit = std::process::Command::new("git")
        .args(["-C", repo_path, "rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        });

    // Check if worktree is dirty
    let is_dirty = std::process::Command::new("git")
        .args(["-C", repo_path, "diff", "--quiet"])
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(false);

    // Get modified files (unstaged changes)
    let modified_files = get_git_status_files(repo_path, "modified");

    // Get staged files
    let staged_files = get_git_status_files(repo_path, "staged");

    // Get untracked files
    let untracked_files = get_git_status_files(repo_path, "untracked");

    // Get recent commits (last 10)
    let recent_commits = get_recent_commits(repo_path, 10);

    GitGrounding::from_repo(
        branch_name,
        head_commit,
        is_dirty,
        modified_files,
        staged_files,
        untracked_files,
        recent_commits,
    )
}

/// Get files with specific Git status type
fn get_git_status_files(repo_path: &str, status_type: &str) -> Vec<GitFileStatus> {
    let args = match status_type {
        "modified" => vec!["-C", repo_path, "diff", "--name-status"],
        "staged" => vec!["-C", repo_path, "diff", "--cached", "--name-status"],
        "untracked" => vec![
            "-C",
            repo_path,
            "ls-files",
            "--others",
            "--exclude-standard",
        ],
        _ => return vec![],
    };

    let output = std::process::Command::new("git").args(&args).output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout
                .lines()
                .filter(|l| !l.is_empty())
                .map(|line| {
                    if status_type == "untracked" {
                        GitFileStatus::new(line, "??")
                    } else {
                        // Parse "M\tfilepath" or "A\tfilepath" format
                        let parts: Vec<&str> = line.split('\t').collect();
                        if parts.len() >= 2 {
                            GitFileStatus::new(parts[1], parts[0])
                        } else {
                            GitFileStatus::new(line, "?")
                        }
                    }
                })
                .collect()
        }
        _ => vec![],
    }
}

/// Get recent commits with summary info
fn get_recent_commits(repo_path: &str, limit: usize) -> Vec<GitCommitSummary> {
    let format_str = format!("%h|%s|%an"); // short hash, subject, author

    let output = std::process::Command::new("git")
        .args([
            "-C",
            repo_path,
            "log",
            &format!("-{}", limit),
            &format!("--format={}", format_str),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout
                .lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| {
                    // Parse "hash|subject|author" format
                    let parts: Vec<&str> = line.split('|').collect();
                    if parts.len() >= 2 {
                        let hash = parts[0].to_string();
                        let subject = parts[1].to_string();
                        let author = parts.get(2).map(|s| s.to_string());
                        let mut commit = GitCommitSummary::new(hash, subject);
                        if let Some(author) = author {
                            commit = commit.with_author(author);
                        }
                        Some(commit)
                    } else {
                        None
                    }
                })
                .collect()
        }
        _ => vec![],
    }
}
