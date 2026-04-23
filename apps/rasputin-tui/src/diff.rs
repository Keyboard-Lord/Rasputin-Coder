//! Diff viewer for file mutations
//!
//! Uses the `similar` crate to compute and display diffs in the TUI.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use similar::{ChangeTag, TextDiff};

/// Represents a single file mutation with before/after content
#[derive(Debug, Clone)]
pub struct FileMutation {
    pub path: String,
    pub before: Option<String>,
    pub after: String,
    pub before_hash: Option<String>,
    pub after_hash: String,
}

/// Generate a unified diff view as styled lines
pub fn unified_diff(mutation: &FileMutation, context_lines: usize) -> Vec<Line<'_>> {
    let mut lines = vec![];

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            format!("--- {} ", mutation.path),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            mutation.before_hash.as_deref().unwrap_or("(new file)"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            format!("+++ {} ", mutation.path),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(&mutation.after_hash, Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(""));

    let before = mutation.before.as_deref().unwrap_or("");
    let after = &mutation.after;

    let diff = TextDiff::from_lines(before, after);

    for group in diff.grouped_ops(context_lines) {
        // Print context header
        let mut header_parts = vec![];
        for op in &group {
            let tag = op.tag();
            let old_range = op.old_range();
            let new_range = op.new_range();

            use similar::DiffTag;
            match tag {
                DiffTag::Delete => {
                    header_parts.push(Span::styled(
                        format!("@@ -{},{} +0,0 @@", old_range.start + 1, old_range.len()),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                DiffTag::Insert => {
                    header_parts.push(Span::styled(
                        format!("@@ -0,0 +{},{} @@", new_range.start + 1, new_range.len()),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                DiffTag::Equal => {
                    if !header_parts.is_empty() {
                        header_parts.push(Span::styled(
                            format!(
                                "@@ -{},{} +{},{} @@",
                                old_range.start + 1,
                                old_range.len(),
                                new_range.start + 1,
                                new_range.len()
                            ),
                            Style::default().fg(Color::Cyan),
                        ));
                    }
                }
                DiffTag::Replace => {
                    // Replace is treated as delete + insert
                    header_parts.push(Span::styled(
                        format!(
                            "@@ -{},{} +{},{} @@",
                            old_range.start + 1,
                            old_range.len(),
                            new_range.start + 1,
                            new_range.len()
                        ),
                        Style::default().fg(Color::Cyan),
                    ));
                }
            }
        }
        if !header_parts.is_empty() {
            lines.push(Line::from(header_parts));
        }

        // Print changes
        for op in &group {
            for change in diff.iter_changes(op) {
                let tag = change.tag();
                let content = change.value();
                // Remove trailing newline for display
                let display = content.strip_suffix('\n').unwrap_or(content);

                match tag {
                    ChangeTag::Delete => {
                        lines.push(Line::from(vec![
                            Span::styled("-", Style::default().fg(Color::Red)),
                            Span::styled(display, Style::default().fg(Color::Red)),
                        ]));
                    }
                    ChangeTag::Insert => {
                        lines.push(Line::from(vec![
                            Span::styled("+", Style::default().fg(Color::Green)),
                            Span::styled(display, Style::default().fg(Color::Green)),
                        ]));
                    }
                    ChangeTag::Equal => {
                        lines.push(Line::from(vec![
                            Span::styled(" ", Style::default().fg(Color::DarkGray)),
                            Span::styled(display, Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                }
            }
        }
        lines.push(Line::from(""));
    }

    lines
}

/// Generate a compact summary of changes (for chat output)
pub fn compact_diff_summary(mutation: &FileMutation) -> String {
    let before = mutation.before.as_deref().unwrap_or("");
    let after = &mutation.after;

    let diff = TextDiff::from_lines(before, after);

    let mut added = 0usize;
    let mut removed = 0usize;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            _ => {}
        }
    }

    if mutation.before.is_none() {
        format!("[NEW] {} (+{} lines)", mutation.path, added)
    } else if after.is_empty() {
        format!("[DEL] {} (-{} lines)", mutation.path, removed)
    } else {
        format!("[MOD] {} (+{} -{} lines)", mutation.path, added, removed)
    }
}

/// Storage for recent file mutations (shown in inspector)
#[derive(Debug, Default, Clone)]
pub struct DiffStore {
    pub mutations: Vec<FileMutation>,
    pub max_entries: usize,
}

impl DiffStore {
    pub fn new(max_entries: usize) -> Self {
        Self {
            mutations: Vec::new(),
            max_entries,
        }
    }

    pub fn add(&mut self, mutation: FileMutation) {
        self.mutations.push(mutation);
        if self.mutations.len() > self.max_entries {
            self.mutations.remove(0);
        }
    }

    pub fn latest(&self) -> Option<&FileMutation> {
        self.mutations.last()
    }

    pub fn get(&self, path: &str) -> Option<&FileMutation> {
        self.mutations.iter().find(|m| m.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_diff_summary_new_file() {
        let mutation = FileMutation {
            path: "src/main.rs".to_string(),
            before: None,
            after: "fn main() {}\n".to_string(),
            before_hash: None,
            after_hash: "abc123".to_string(),
        };
        let summary = compact_diff_summary(&mutation);
        assert!(summary.contains("NEW"));
        assert!(summary.contains("src/main.rs"));
        assert!(summary.contains("+1"));
    }

    #[test]
    fn test_compact_diff_summary_modified() {
        let mutation = FileMutation {
            path: "src/lib.rs".to_string(),
            before: Some("fn old() {}\n".to_string()),
            after: "fn new() {}\n".to_string(),
            before_hash: Some("old123".to_string()),
            after_hash: "new456".to_string(),
        };
        let summary = compact_diff_summary(&mutation);
        assert!(summary.contains("MOD"));
        assert!(summary.contains("src/lib.rs"));
    }

    #[test]
    fn test_unified_diff_basic() {
        let mutation = FileMutation {
            path: "test.txt".to_string(),
            before: Some("line1\nline2\n".to_string()),
            after: "line1\nmodified\n".to_string(),
            before_hash: Some("old".to_string()),
            after_hash: "new".to_string(),
        };
        let lines = unified_diff(&mutation, 3);
        assert!(!lines.is_empty());
        // Check header is present
        let header_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header_text.contains("test.txt"));
    }
}
