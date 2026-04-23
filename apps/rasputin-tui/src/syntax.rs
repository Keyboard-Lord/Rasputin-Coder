//! Syntax highlighting for code blocks in the TUI
//!
//! Uses syntect with embedded syntax definitions for common languages.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use std::sync::OnceLock;
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

/// Global syntax set and theme set (lazy initialization)
static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

/// Initialize syntax highlighting resources
fn init_syntax() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn init_themes() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Detect language from file extension or content
fn detect_language(path: &str, content: &str) -> Option<String> {
    // First try file extension
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some("rs") => Some("rust".to_string()),
        Some("py") => Some("python".to_string()),
        Some("js") => Some("javascript".to_string()),
        Some("ts") => Some("typescript".to_string()),
        Some("jsx") => Some("jsx".to_string()),
        Some("tsx") => Some("tsx".to_string()),
        Some("json") => Some("json".to_string()),
        Some("yaml") | Some("yml") => Some("yaml".to_string()),
        Some("toml") => Some("toml".to_string()),
        Some("md") | Some("markdown") => Some("markdown".to_string()),
        Some("html") | Some("htm") => Some("html".to_string()),
        Some("css") => Some("css".to_string()),
        Some("scss") | Some("sass") => Some("scss".to_string()),
        Some("go") => Some("go".to_string()),
        Some("c") => Some("c".to_string()),
        Some("cpp") | Some("cc") | Some("cxx") => Some("cpp".to_string()),
        Some("h") | Some("hpp") => Some("c".to_string()),
        Some("java") => Some("java".to_string()),
        Some("kt") => Some("kotlin".to_string()),
        Some("swift") => Some("swift".to_string()),
        Some("rb") => Some("ruby".to_string()),
        Some("php") => Some("php".to_string()),
        Some("sh") | Some("bash") | Some("zsh") => Some("bash".to_string()),
        Some("dockerfile") => Some("dockerfile".to_string()),
        Some("sql") => Some("sql".to_string()),
        Some("xml") => Some("xml".to_string()),
        Some("svg") => Some("xml".to_string()),
        _ => {
            // Try to detect from shebang
            if content.starts_with("#!/usr/bin/env python")
                || content.starts_with("#!/usr/bin/python")
            {
                Some("python".to_string())
            } else if content.starts_with("#!/usr/bin/env bash")
                || content.starts_with("#!/bin/bash")
            {
                Some("bash".to_string())
            } else if content.starts_with("#!/usr/bin/env node")
                || content.starts_with("#!/usr/bin/node")
            {
                Some("javascript".to_string())
            } else if content.starts_with("#!/usr/bin/env ruby")
                || content.starts_with("#!/usr/bin/ruby")
            {
                Some("ruby".to_string())
            } else {
                None
            }
        }
    }
}

/// Convert syntect style to ratatui style
fn syntect_to_ratatui(style: SyntectStyle) -> Style {
    let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
    let bg = if style.background.a > 0 {
        Some(Color::Rgb(
            style.background.r,
            style.background.g,
            style.background.b,
        ))
    } else {
        None
    };

    let mut ratatui_style = Style::default().fg(fg);
    if let Some(bg_color) = bg {
        ratatui_style = ratatui_style.bg(bg_color);
    }

    // Handle font style
    if style
        .font_style
        .contains(syntect::highlighting::FontStyle::BOLD)
    {
        ratatui_style = ratatui_style.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if style
        .font_style
        .contains(syntect::highlighting::FontStyle::ITALIC)
    {
        ratatui_style = ratatui_style.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if style
        .font_style
        .contains(syntect::highlighting::FontStyle::UNDERLINE)
    {
        ratatui_style = ratatui_style.add_modifier(ratatui::style::Modifier::UNDERLINED);
    }

    ratatui_style
}

/// Highlight code content and return styled lines
pub fn highlight_code(content: &str, language: Option<&str>) -> Vec<Line<'static>> {
    let syntax_set = init_syntax();
    let theme_set = init_themes();

    // Use a dark theme that works well in terminals
    let theme = &theme_set.themes["base16-ocean.dark"];

    // Find syntax definition
    let syntax = if let Some(lang) = language {
        syntax_set.find_syntax_by_token(lang)
    } else {
        syntax_set.find_syntax_by_first_line(content)
    }
    .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = vec![];

    for line in LinesWithEndings::from(content) {
        let highlighted = highlighter.highlight_line(line, syntax_set);
        match highlighted {
            Ok(regions) => {
                let spans: Vec<Span<'static>> = regions
                    .into_iter()
                    .map(|(style, text)| {
                        let ratatui_style = syntect_to_ratatui(style);
                        Span::styled(text.to_string(), ratatui_style)
                    })
                    .collect();
                lines.push(Line::from(spans));
            }
            Err(_) => {
                // Fallback to plain text if highlighting fails
                let trimmed = line.strip_suffix('\n').unwrap_or(line);
                lines.push(Line::from(trimmed.to_string()));
            }
        }
    }

    lines
}

/// Highlight file content with auto-detected language
pub fn highlight_file(path: &str, content: &str) -> Vec<Line<'static>> {
    let language = detect_language(path, content);
    highlight_code(content, language.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language_from_extension() {
        assert_eq!(detect_language("test.rs", ""), Some("rust".to_string()));
        assert_eq!(detect_language("test.py", ""), Some("python".to_string()));
        assert_eq!(
            detect_language("test.js", ""),
            Some("javascript".to_string())
        );
        assert_eq!(
            detect_language("test.ts", ""),
            Some("typescript".to_string())
        );
    }

    #[test]
    fn test_detect_language_from_shebang() {
        assert_eq!(
            detect_language("script", "#!/usr/bin/env python3"),
            Some("python".to_string())
        );
        assert_eq!(
            detect_language("script", "#!/bin/bash"),
            Some("bash".to_string())
        );
    }

    #[test]
    fn test_highlight_rust_code() {
        let code = "fn main() { println!(\"Hello\"); }";
        let lines = highlight_code(code, Some("rust"));
        assert!(!lines.is_empty());
        // Should have at least one line with styled spans
        assert!(!lines[0].spans.is_empty());
    }

    #[test]
    fn test_highlight_file() {
        let content = "fn main() {}";
        let lines = highlight_file("src/main.rs", content);
        assert!(!lines.is_empty());
    }
}
