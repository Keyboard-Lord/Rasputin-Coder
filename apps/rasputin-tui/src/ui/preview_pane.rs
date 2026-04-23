//! File preview pane for right-side panel
//!
//! Shows file previews with state semantics:
//! - PendingValidation: Uncommitted changes
//! - Validated: Committed and passed validation
//! - Reverted: Failed validation, rolled back
//! - CurrentOnDisk: Current file state

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::path::{Path, PathBuf};

/// State of the preview pane
#[derive(Debug, Clone)]
pub struct PreviewPaneState {
    /// Currently previewed file
    pub source_file: Option<PathBuf>,
    
    /// Validation state of the preview
    pub validation_state: PreviewValidationState,
    
    /// Rendered content ready for display
    pub rendered_content: RenderedPreview,
    
    /// When this preview was generated
    pub last_updated: chrono::DateTime<chrono::Local>,
    
    /// Content hash for staleness detection
    pub content_hash: Option<String>,
    
    /// Whether to follow current file (auto-update)
    pub follow_mode: bool,
}

impl PreviewPaneState {
    /// Create new empty preview pane
    pub fn new() -> Self {
        Self {
            source_file: None,
            validation_state: PreviewValidationState::Empty,
            rendered_content: RenderedPreview::Empty,
            last_updated: chrono::Local::now(),
            content_hash: None,
            follow_mode: true,
        }
    }
    
    /// Set file to preview
    pub fn set_file(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref().to_path_buf();
        self.source_file = Some(path);
        self.refresh();
    }
    
    /// Clear preview
    pub fn clear(&mut self) {
        self.source_file = None;
        self.validation_state = PreviewValidationState::Empty;
        self.rendered_content = RenderedPreview::Empty;
        self.content_hash = None;
    }
    
    /// Update validation state
    pub fn set_validation_state(&mut self, state: PreviewValidationState) {
        self.validation_state = state;
        self.last_updated = chrono::Local::now();
    }
    
    /// Refresh preview content
    pub fn refresh(&mut self) {
        if let Some(ref path) = self.source_file {
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    self.rendered_content = render_content(path, &content);
                    self.content_hash = Some(compute_hash(&content));
                    self.last_updated = chrono::Local::now();
                }
                Err(e) => {
                    self.rendered_content = RenderedPreview::Error(
                        format!("Failed to read file: {}", e)
                    );
                }
            }
        }
    }
    
    /// Check if needs refresh based on hash
    pub fn needs_refresh(&self) -> bool {
        if let Some(ref path) = self.source_file {
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let current_hash = compute_hash(&content);
                    self.content_hash.as_ref() != Some(&current_hash)
                }
                Err(_) => true, // File changed or deleted
            }
        } else {
            false
        }
    }
    
    /// Get state banner text and color
    pub fn state_banner(&self) -> (&str, Color) {
        match self.validation_state {
            PreviewValidationState::Empty => ("No file selected", Color::Gray),
            PreviewValidationState::PendingValidation => {
                ("⚠ PREVIEW: Pending Validation", Color::Yellow)
            }
            PreviewValidationState::Validated { .. } => {
                ("✓ PREVIEW: Current (Validated)", Color::Green)
            }
            PreviewValidationState::Reverted { .. } => {
                ("✗ PREVIEW: Reverted", Color::Red)
            }
            PreviewValidationState::CurrentOnDisk => {
                ("ℹ PREVIEW: Current File on Disk", Color::Blue)
            }
            PreviewValidationState::Error(ref e) => {
                (e.as_str(), Color::Red)
            }
        }
    }
    
    /// Render as ratatui widget
    pub fn render(&self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        let (banner_text, banner_color) = self.state_banner();
        
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(banner_color))
            .title(format!(
                " Preview: {} ",
                self.source_file.as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("None")
            ));
        
        // Render banner line
        let banner = Line::from(vec![
            Span::styled(banner_text, Style::default().fg(banner_color)),
        ]);
        
        let content_lines = match &self.rendered_content {
            RenderedPreview::Empty => vec![Line::from("No preview available")],
            RenderedPreview::Text(lines) => {
                lines.iter().map(|l| Line::from(l.as_str())).collect()
            }
            RenderedPreview::Error(msg) => {
                vec![Line::from(vec![
                    Span::styled("Error: ", Style::default().fg(Color::Red)),
                    Span::raw(msg),
                ])]
            }
            RenderedPreview::Html(_) => vec![Line::from("HTML preview not available in TUI")],
            RenderedPreview::Markdown(_) => vec![Line::from("Markdown preview not available in TUI")],
            RenderedPreview::Code { lines, .. } => {
                lines.iter().map(|l| {
                    Line::from(vec![
                        Span::styled(
                            format!("{:4} ", l.line_number),
                            Style::default().fg(Color::DarkGray)
                        ),
                        Span::raw(&l.text),
                    ])
                }).collect()
            }
            RenderedPreview::Binary => vec![Line::from("Binary file - cannot preview")],
        };
        
        let mut all_lines = vec![banner];
        all_lines.extend(content_lines);
        
        let paragraph = Paragraph::new(all_lines).block(block);
        paragraph.render(area, buf);
    }
}

impl Default for PreviewPaneState {
    fn default() -> Self {
        Self::new()
    }
}

/// Validation state of previewed content
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewValidationState {
    /// Empty, no file selected
    Empty,
    /// Uncommitted changes pending validation
    PendingValidation,
    /// Committed and passed validation
    Validated { commit_id: String },
    /// Failed validation, rolled back
    Reverted { reason: String },
    /// Reading current file on disk
    CurrentOnDisk,
    /// Error reading/rendering
    Error(String),
}

/// Rendered preview content
#[derive(Debug, Clone)]
pub enum RenderedPreview {
    /// Empty preview
    Empty,
    /// Plain text lines
    Text(Vec<String>),
    /// HTML content (may need special rendering)
    Html(String),
    /// Markdown content
    Markdown(String),
    /// Code with syntax highlighting
    Code {
        lines: Vec<HighlightedLine>,
        language: String,
    },
    /// Binary file (cannot preview)
    Binary,
    /// Error message
    Error(String),
}

/// A line of highlighted code
#[derive(Debug, Clone)]
pub struct HighlightedLine {
    pub line_number: usize,
    pub text: String,
}

/// Render file content based on type
fn render_content(path: &Path, content: &str) -> RenderedPreview {
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    
    match ext.as_str() {
        "html" | "htm" => RenderedPreview::Html(content.to_string()),
        "md" | "markdown" => RenderedPreview::Markdown(content.to_string()),
        "rs" | "js" | "ts" | "py" | "go" | "java" | "c" | "cpp" | "h" | "hpp" => {
            render_code(content, &ext)
        }
        "json" | "yaml" | "yml" | "toml" => render_code(content, &ext),
        _ => RenderedPreview::Text(content.lines().map(|l| l.to_string()).collect()),
    }
}

/// Simple code rendering (placeholder for syntect integration)
fn render_code(content: &str, _language: &str) -> RenderedPreview {
    let lines: Vec<HighlightedLine> = content
        .lines()
        .enumerate()
        .map(|(idx, line)| HighlightedLine {
            line_number: idx + 1,
            text: line.to_string(),
        })
        .collect();
    
    RenderedPreview::Code {
        lines,
        language: _language.to_string(),
    }
}

/// Compute simple hash for staleness detection
fn compute_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// File type detection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Html,
    Markdown,
    Code { language: &'static str },
    Text,
    Binary,
}

impl FileType {
    /// Detect file type from path
    pub fn from_path(path: &Path) -> Self {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        
        match ext.as_str() {
            "html" | "htm" => FileType::Html,
            "md" | "markdown" => FileType::Markdown,
            "rs" => FileType::Code { language: "rust" },
            "js" => FileType::Code { language: "javascript" },
            "ts" => FileType::Code { language: "typescript" },
            "py" => FileType::Code { language: "python" },
            "go" => FileType::Code { language: "go" },
            "java" => FileType::Code { language: "java" },
            "c" => FileType::Code { language: "c" },
            "cpp" | "cc" | "cxx" => FileType::Code { language: "cpp" },
            "json" | "yaml" | "yml" | "toml" => FileType::Code { language: &ext },
            _ => FileType::Text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_preview_validation_states() {
        let mut preview = PreviewPaneState::new();
        
        assert!(matches!(preview.validation_state, PreviewValidationState::Empty));
        
        preview.set_validation_state(PreviewValidationState::PendingValidation);
        let (text, color) = preview.state_banner();
        assert!(text.contains("Pending"));
        assert_eq!(color, Color::Yellow);
        
        preview.set_validation_state(PreviewValidationState::Validated { 
            commit_id: "abc123".to_string() 
        });
        let (text, color) = preview.state_banner();
        assert!(text.contains("Validated"));
        assert_eq!(color, Color::Green);
    }
    
    #[test]
    fn test_file_type_detection() {
        assert!(matches!(
            FileType::from_path(Path::new("test.html")),
            FileType::Html
        ));
        assert!(matches!(
            FileType::from_path(Path::new("test.rs")),
            FileType::Code { language: "rust" }
        ));
        assert!(matches!(
            FileType::from_path(Path::new("test.md")),
            FileType::Markdown
        ));
    }
    
    #[test]
    fn test_compute_hash() {
        let h1 = compute_hash("Hello");
        let h2 = compute_hash("Hello");
        let h3 = compute_hash("World");
        
        assert_eq!(h1, h2); // Same content = same hash
        assert_ne!(h1, h3); // Different content = different hash
    }
}
