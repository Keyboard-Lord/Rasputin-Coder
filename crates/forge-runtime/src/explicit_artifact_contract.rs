use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitArtifactContract {
    pub artifact_type: Option<String>,
    pub required_count: Option<usize>,
    pub required_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExplicitArtifactStatus {
    pub present_paths: Vec<String>,
    pub missing_paths: Vec<String>,
    pub empty_paths: Vec<String>,
    pub unexpected_paths: Vec<String>,
}

impl ExplicitArtifactContract {
    pub fn from_task(task: &str) -> Option<Self> {
        let task_scope = contract_task_scope(task);
        let required_paths = extract_explicit_filenames(task_scope);
        let required_count = detect_explicit_artifact_count(task_scope).or_else(|| {
            if required_paths.len() > 1 {
                Some(required_paths.len())
            } else {
                None
            }
        });

        let lower = task_scope.to_lowercase();
        let numbered_filename_lines = task_scope
            .lines()
            .filter(|line| line_contains_numbered_filename(line))
            .count();
        let has_contract_language = lower.contains("exactly")
            || lower.contains("precise filenames")
            || lower.contains("exact filenames")
            || lower.contains("all of these")
            || lower.contains("all of the following")
            || lower.contains("must be produced")
            || lower.contains("must produce")
            || lower.contains("deliverable set")
            || lower.contains("named markdown files")
            || lower.contains("required files")
            || lower.contains("required artifacts");

        let is_explicit_contract = required_paths.len() > 1
            && (required_count.is_some() || has_contract_language || numbered_filename_lines >= 2);
        if !is_explicit_contract {
            return None;
        }

        Some(Self {
            artifact_type: detect_artifact_type(task_scope, &required_paths),
            required_count,
            required_paths,
        })
    }

    pub fn required_deliverable_count(&self) -> usize {
        self.required_count
            .unwrap_or_else(|| self.required_paths.len())
    }

    pub fn evaluate(&self, written_paths: &[PathBuf]) -> ExplicitArtifactStatus {
        let mut status = ExplicitArtifactStatus::default();

        for expected in &self.required_paths {
            match written_paths
                .iter()
                .find(|path| path_matches_expected(path.as_path(), expected))
            {
                Some(path) if path_has_content(path) => status.present_paths.push(expected.clone()),
                Some(_) => status.empty_paths.push(expected.clone()),
                None => status.missing_paths.push(expected.clone()),
            }
        }

        let required_set = self
            .required_paths
            .iter()
            .map(|path| normalize_path_text(path))
            .collect::<HashSet<_>>();
        for written in written_paths {
            let normalized = normalize_path_text(&written.to_string_lossy());
            let matched_required = required_set
                .iter()
                .any(|expected| normalized == *expected || normalized.ends_with(&format!("/{}", expected)));
            if !matched_required && self.matches_artifact_family(written.as_path()) {
                status
                    .unexpected_paths
                    .push(written.display().to_string());
            }
        }

        status.present_paths.sort();
        status.missing_paths.sort();
        status.empty_paths.sort();
        status.unexpected_paths.sort();
        status
    }

    pub fn is_reason_contract_aware(&self, reason: &str) -> bool {
        let normalized = normalize_path_text(reason);
        normalized.contains(&self.required_deliverable_count().to_string())
            || self.required_paths.iter().any(|path| {
                let normalized_path = normalize_path_text(path);
                let basename = Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_lowercase())
                    .unwrap_or_else(|| normalized_path.clone());
                normalized.contains(&normalized_path) || normalized.contains(&basename)
            })
    }

    fn matches_artifact_family(&self, path: &Path) -> bool {
        match self.artifact_type.as_deref() {
            Some("markdown") => {
                matches!(
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .map(|extension| extension.to_lowercase())
                        .as_deref(),
                    Some("md") | Some("markdown")
                )
            }
            Some(expected_extension) => path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.eq_ignore_ascii_case(expected_extension))
                .unwrap_or(false),
            None => true,
        }
    }
}

fn contract_task_scope(task: &str) -> &str {
    task.rsplit("=== TASK ===")
        .next()
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .unwrap_or(task)
}

pub fn path_matches_expected(path: &Path, expected: &str) -> bool {
    let actual = normalize_path_text(&path.to_string_lossy());
    let expected = normalize_path_text(expected);

    actual == expected || actual.ends_with(&format!("/{}", expected))
}

pub fn normalize_path_text(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .replace('\\', "/")
        .to_lowercase()
}

fn path_has_content(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| !content.trim().is_empty())
        .unwrap_or(false)
}

fn normalize_intent_text(statement: &str) -> String {
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

fn detect_explicit_artifact_count(statement: &str) -> Option<usize> {
    let normalized = normalize_intent_text(statement);
    let words: Vec<&str> = normalized.split_whitespace().collect();

    for (idx, word) in words.iter().enumerate() {
        let Some(count) = parse_count_word(word) else {
            continue;
        };
        let previous = idx
            .checked_sub(1)
            .and_then(|offset| words.get(offset))
            .copied()
            .unwrap_or_default();
        let next = words.get(idx + 1).copied().unwrap_or_default();
        let next_two = words.get(idx + 2).copied().unwrap_or_default();

        if matches!(previous, "exactly" | "precisely" | "total" | "produce" | "creating")
            || matches!(
                next,
                "artifact" | "artifacts" | "doc" | "docs" | "document" | "documents" | "file"
                    | "files" | "markdown"
            )
            || matches!(
                next_two,
                "artifact" | "artifacts" | "doc" | "docs" | "document" | "documents" | "file"
                    | "files" | "markdown"
            )
        {
            return Some(count);
        }
    }

    None
}

fn parse_count_word(word: &str) -> Option<usize> {
    if let Ok(value) = word.parse::<usize>() {
        return Some(value);
    }

    match word {
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        "eleven" => Some(11),
        "twelve" => Some(12),
        "thirteen" => Some(13),
        "fourteen" => Some(14),
        "fifteen" => Some(15),
        "sixteen" => Some(16),
        "seventeen" => Some(17),
        "eighteen" => Some(18),
        "nineteen" => Some(19),
        "twenty" => Some(20),
        _ => None,
    }
}

fn extract_explicit_filenames(statement: &str) -> Vec<String> {
    let mut filenames = Vec::new();
    let mut seen = HashSet::new();

    for line in statement.lines() {
        for candidate in extract_filename_candidates_from_line(line) {
            if seen.insert(candidate.clone()) {
                filenames.push(candidate);
            }
        }
    }

    filenames
}

fn line_contains_numbered_filename(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some((prefix, remainder)) = trimmed.split_once('.') else {
        return false;
    };
    if !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }

    !extract_filename_candidates_from_line(remainder).is_empty()
}

fn extract_filename_candidates_from_line(line: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut trimmed = line.trim();

    while matches!(trimmed.chars().next(), Some('#' | '-' | '*' | '•')) {
        trimmed = trimmed[1..].trim_start();
    }

    if let Some((prefix, remainder)) = trimmed.split_once('.') {
        if prefix.chars().all(|ch| ch.is_ascii_digit()) {
            trimmed = remainder.trim_start();
        }
    }

    if let Some((prefix, remainder)) = trimmed.split_once(')') {
        if prefix.chars().all(|ch| ch.is_ascii_digit()) {
            trimmed = remainder.trim_start();
        }
    }

    for token in trimmed.split_whitespace() {
        let candidate = normalize_artifact_token(token);
        if looks_like_filename(&candidate) {
            candidates.push(candidate);
        }
    }

    candidates
}

fn normalize_artifact_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '`' | '"' | '\'' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        })
        .trim_start_matches("./")
        .replace('\\', "/")
}

fn looks_like_filename(candidate: &str) -> bool {
    if candidate.is_empty()
        || candidate.starts_with("http://")
        || candidate.starts_with("https://")
        || candidate.starts_with('/')
        || !candidate.contains('.')
    {
        return false;
    }

    let Some((stem, extension)) = candidate.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty()
        || extension.len() < 2
        || extension.len() > 10
        || !extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        || !stem.chars().any(|ch| ch.is_ascii_alphabetic())
    {
        return false;
    }

    candidate
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.'))
}

fn detect_artifact_type(statement: &str, required_paths: &[String]) -> Option<String> {
    let lower = statement.to_lowercase();
    if lower.contains("markdown") {
        return Some("markdown".to_string());
    }

    if required_paths.is_empty() {
        return None;
    }

    if required_paths
        .iter()
        .all(|path| path.ends_with(".md") || path.ends_with(".markdown"))
    {
        Some("markdown".to_string())
    } else if required_paths.iter().all(|path| path.ends_with(".json")) {
        Some("json".to_string())
    } else if required_paths.iter().all(|path| path.ends_with(".txt")) {
        Some("text".to_string())
    } else {
        required_paths[0]
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_explicit_multi_artifact_contract() {
        let task = "Create exactly 3 markdown files with these precise filenames:\n\
1. docs/01_PROJECT_OVERVIEW.md\n\
2. docs/02_ARCHITECTURE.md\n\
3. docs/03_TECHNOLOGY_STACK.md\n\
All of these must be produced.";

        let contract = ExplicitArtifactContract::from_task(task).expect("contract");

        assert_eq!(contract.required_count, Some(3));
        assert_eq!(contract.artifact_type.as_deref(), Some("markdown"));
        assert_eq!(contract.required_paths.len(), 3);
    }

    #[test]
    fn ignores_repo_shape_context_when_extracting_contract() {
        let task = "=== Repository Shape ===\n\
- Cargo.toml\n\
- README.md\n\
- src/lib.rs\n\
\n\
=== TASK ===\n\
Create exactly 2 markdown files with these precise filenames:\n\
1. docs/01_PROJECT_OVERVIEW.md\n\
2. docs/02_ARCHITECTURE.md\n\
All of these must be produced.";

        let contract = ExplicitArtifactContract::from_task(task).expect("contract");

        assert_eq!(
            contract.required_paths,
            vec![
                "docs/01_PROJECT_OVERVIEW.md".to_string(),
                "docs/02_ARCHITECTURE.md".to_string(),
            ]
        );
    }
}
