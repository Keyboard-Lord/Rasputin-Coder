use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceModelConfig {
    pub model: String,
    pub source: &'static str,
}

pub fn discover_workspace_model(dir: &Path) -> Option<WorkspaceModelConfig> {
    read_forge_yaml_model(dir)
        .map(|model| WorkspaceModelConfig {
            model,
            source: ".forge/config.yaml",
        })
        .or_else(|| {
            read_rasputin_model(dir).map(|model| WorkspaceModelConfig {
                model,
                source: "rasputin.json",
            })
        })
}

#[allow(dead_code)]
pub fn detect_git_repository(dir: &Path) -> bool {
    resolve_git_dir(dir).is_some()
}

#[allow(dead_code)]
pub fn is_git_worktree(dir: &Path) -> bool {
    let dot_git = dir.join(".git");
    dot_git.is_file() && parse_gitdir_file(&dot_git).is_some()
}

#[allow(dead_code)]
fn resolve_git_dir(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }

    if dot_git.is_file() {
        return parse_gitdir_file(&dot_git);
    }

    None
}

fn parse_gitdir_file(dot_git_file: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string(dot_git_file).ok()?;
    let raw_gitdir = content
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:").map(str::trim))?;
    let resolved = if Path::new(raw_gitdir).is_absolute() {
        PathBuf::from(raw_gitdir)
    } else {
        dot_git_file.parent()?.join(raw_gitdir)
    };

    if resolved.exists() {
        Some(resolved)
    } else {
        None
    }
}

fn read_rasputin_model(dir: &Path) -> Option<String> {
    let content = fs::read_to_string(dir.join("rasputin.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("ollama_model")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn read_forge_yaml_model(dir: &Path) -> Option<String> {
    for file_name in ["config.yaml", "config.yml"] {
        let path = dir.join(".forge").join(file_name);
        let content = fs::read_to_string(path).ok()?;
        if let Some(model) = extract_yaml_model(&content) {
            return Some(model);
        }
    }

    None
}

fn extract_yaml_model(content: &str) -> Option<String> {
    let mut in_planner_section = false;
    let mut in_ollama_section = false;

    for raw_line in content.lines() {
        let uncommented = raw_line.split('#').next().unwrap_or("").trim_end();
        if uncommented.trim().is_empty() {
            continue;
        }

        let indent = raw_line.chars().take_while(|ch| ch.is_whitespace()).count();
        let trimmed = uncommented.trim();

        if indent == 0 {
            in_planner_section = trimmed == "planner:";
            in_ollama_section = trimmed == "ollama:";

            if let Some(value) = parse_yaml_scalar(trimmed, "planner_model:") {
                return Some(value);
            }
            if let Some(value) = parse_yaml_scalar(trimmed, "ollama_model:") {
                return Some(value);
            }
            continue;
        }

        if (in_planner_section || in_ollama_section)
            && let Some(value) = parse_yaml_scalar(trimmed, "model:")
        {
            return Some(value);
        }

        if in_planner_section && let Some(value) = parse_yaml_scalar(trimmed, "planner_model:") {
            return Some(value);
        }

        if in_ollama_section && let Some(value) = parse_yaml_scalar(trimmed, "ollama_model:") {
            return Some(value);
        }
    }

    None
}

fn parse_yaml_scalar(line: &str, key: &str) -> Option<String> {
    line.strip_prefix(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_matches('"').trim_matches('\'').to_string())
}

#[cfg(test)]
mod tests {
    use super::{detect_git_repository, discover_workspace_model, is_git_worktree};
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rasputin-workspace-config-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn forge_yaml_takes_precedence_over_rasputin_json() {
        let dir = temp_dir("model-precedence");
        fs::create_dir_all(dir.join(".forge")).expect("create forge dir");
        fs::write(
            dir.join(".forge/config.yaml"),
            "planner:\n  model: qwen2.5-coder:14b-q4km\n",
        )
        .expect("write forge yaml");
        fs::write(
            dir.join("rasputin.json"),
            "{ \"ollama_model\": \"qwen3.5:latest\" }\n",
        )
        .expect("write rasputin json");

        let config = discover_workspace_model(&dir).expect("discover workspace model");
        assert_eq!(config.model, "qwen2.5-coder:14b-q4km");
        assert_eq!(config.source, ".forge/config.yaml");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detects_standard_git_repository() {
        let dir = temp_dir("git-dir");
        fs::create_dir_all(dir.join(".git")).expect("create git dir");

        assert!(detect_git_repository(&dir));
        assert!(!is_git_worktree(&dir));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detects_git_worktree_file_reference() {
        let dir = temp_dir("git-worktree");
        let gitdir = dir.join(".actual-git/worktrees/current");
        fs::create_dir_all(&gitdir).expect("create linked git dir");
        fs::write(dir.join(".git"), "gitdir: .actual-git/worktrees/current\n")
            .expect("write gitdir reference");

        assert!(detect_git_repository(&dir));
        assert!(is_git_worktree(&dir));

        let _ = fs::remove_dir_all(dir);
    }
}
