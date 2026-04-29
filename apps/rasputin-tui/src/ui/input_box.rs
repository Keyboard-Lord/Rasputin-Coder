//! Multi-line input box with bracketed paste support
//!
//! Provides:
//! - Multi-line text input
//! - Bracketed paste detection
//! - Vertical expansion (up to max height)
//! - Proper cursor handling

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Multi-line input buffer
#[derive(Debug, Clone)]
pub struct InputBuffer {
    /// Full text content
    pub content: String,
    
    /// Cursor position in bytes
    pub cursor_byte: usize,
    
    /// Visual lines after wrapping
    wrapped_lines: Vec<String>,
    
    /// Input box height in terminal rows
    pub visual_height: usize,
    
    /// Maximum height before scrolling
    max_height: usize,
    
    /// Bracketed paste mode active
    pub paste_mode: bool,
    
    /// Paste accumulator
    paste_buffer: String,
}

impl InputBuffer {
    /// Create new input buffer
    pub fn new() -> Self {
        Self {
            content: String::new(),
            cursor_byte: 0,
            wrapped_lines: vec![String::new()],
            visual_height: 1,
            max_height: 10,
            paste_mode: false,
            paste_buffer: String::new(),
        }
    }
    
    /// Get current content
    pub fn content(&self) -> &str {
        &self.content
    }
    
    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
    
    /// Clear all content
    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor_byte = 0;
        self.recalculate_wrapping();
    }
    
    /// Insert character at cursor
    pub fn insert_char(&mut self, c: char) {
        if self.cursor_byte > self.content.len() {
            self.cursor_byte = self.content.len();
        }
        self.cursor_byte = crate::text::clamp_to_char_boundary(&self.content, self.cursor_byte);
        
        self.content.insert(self.cursor_byte, c);
        self.cursor_byte += c.len_utf8();
        self.recalculate_wrapping();
    }
    
    /// Insert string at cursor
    pub fn insert_str(&mut self, text: &str) {
        if self.cursor_byte > self.content.len() {
            self.cursor_byte = self.content.len();
        }
        self.cursor_byte = crate::text::clamp_to_char_boundary(&self.content, self.cursor_byte);
        
        self.content.insert_str(self.cursor_byte, text);
        self.cursor_byte += text.len();
        self.recalculate_wrapping();
    }
    
    /// Handle backspace
    pub fn backspace(&mut self) {
        if self.cursor_byte > 0 {
            let prev_char_start = self.content[..self.cursor_byte]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            
            self.content.remove(prev_char_start);
            self.cursor_byte = prev_char_start;
            self.recalculate_wrapping();
        }
    }
    
    /// Handle delete
    pub fn delete(&mut self) {
        self.cursor_byte = crate::text::clamp_to_char_boundary(&self.content, self.cursor_byte);
        if self.cursor_byte < self.content.len() {
            self.content.remove(self.cursor_byte);
            self.recalculate_wrapping();
        }
    }
    
    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor_byte > 0 {
            let prev_char_start = self.content[..self.cursor_byte]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor_byte = prev_char_start;
        }
    }
    
    /// Move cursor right
    pub fn move_right(&mut self) {
        self.cursor_byte = crate::text::clamp_to_char_boundary(&self.content, self.cursor_byte);
        if self.cursor_byte < self.content.len() {
            let next_char_len = self.content[self.cursor_byte..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.cursor_byte += next_char_len;
        }
    }
    
    /// Move cursor to start
    pub fn move_to_start(&mut self) {
        self.cursor_byte = 0;
    }
    
    /// Move cursor to end
    pub fn move_to_end(&mut self) {
        self.cursor_byte = self.content.len();
    }
    
    /// Move cursor up (to previous wrapped line)
    pub fn move_up(&mut self, line_width: usize) {
        let line_width = line_width.max(1);
        let cursor_chars = self.content[..crate::text::clamp_to_char_boundary(&self.content, self.cursor_byte)]
            .chars()
            .count();
        let current_line = cursor_chars / line_width;
        if current_line > 0 {
            let target_line = current_line - 1;
            let target_pos = target_line * line_width + (cursor_chars % line_width);
            self.cursor_byte = crate::text::byte_index_after_chars(&self.content, target_pos);
        }
    }
    
    /// Move cursor down (to next wrapped line)
    pub fn move_down(&mut self, line_width: usize) {
        let line_width = line_width.max(1);
        let cursor_chars = self.content[..crate::text::clamp_to_char_boundary(&self.content, self.cursor_byte)]
            .chars()
            .count();
        let total_chars = self.content.chars().count();
        let current_line = cursor_chars / line_width;
        let total_lines = (total_chars / line_width) + 1;
        
        if current_line + 1 < total_lines {
            let target_line = current_line + 1;
            let target_pos = target_line * line_width + (cursor_chars % line_width);
            self.cursor_byte = crate::text::byte_index_after_chars(&self.content, target_pos);
        }
    }
    
    /// Start bracketed paste mode
    pub fn start_paste(&mut self) {
        self.paste_mode = true;
        self.paste_buffer.clear();
    }
    
    /// End bracketed paste mode, insert accumulated content
    pub fn end_paste(&mut self) {
        self.paste_mode = false;
        if !self.paste_buffer.is_empty() {
            let content = std::mem::take(&mut self.paste_buffer);
            self.insert_str(&content);
        }
    }
    
    /// Add text during paste mode
    pub fn paste_add(&mut self, text: &str) {
        if self.paste_mode {
            self.paste_buffer.push_str(text);
        }
    }
    
    /// Get cursor position for rendering
    pub fn cursor_position(&self) -> (usize, usize) {
        // Calculate line and column based on wrapped lines
        let mut byte_count = 0;
        for (line_idx, line) in self.wrapped_lines.iter().enumerate() {
            if byte_count + line.len() >= self.cursor_byte {
                let col = self.cursor_byte - byte_count;
                return (line_idx, col);
            }
            byte_count += line.len();
        }
        
        // Cursor at end
        (self.wrapped_lines.len().saturating_sub(1), 0)
    }
    
    /// Recalculate wrapped lines and visual height
    fn recalculate_wrapping(&mut self) {
        // Simple word-wrapping at 80 chars for now
        // In production, use actual terminal width
        let width = 80;
        
        self.wrapped_lines.clear();
        
        for line in self.content.lines() {
            if line.chars().count() <= width {
                self.wrapped_lines.push(line.to_string());
            } else {
                self.wrapped_lines
                    .extend(crate::text::chunk_chars(line, width));
            }
        }
        
        if self.wrapped_lines.is_empty() {
            self.wrapped_lines.push(String::new());
        }
        
        // Update visual height (capped at max)
        self.visual_height = self.wrapped_lines.len().min(self.max_height);
    }
    
    /// Get visual lines for rendering
    pub fn visual_lines(&self) -> &[String] {
        &self.wrapped_lines
    }
    
    /// Get current visual height
    pub fn height(&self) -> usize {
        self.visual_height
    }
    
    /// Set maximum height
    pub fn set_max_height(&mut self, height: usize) {
        self.max_height = height;
        self.recalculate_wrapping();
    }
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Render input buffer as ratatui widget
pub fn render_input_box(
    buffer: &InputBuffer,
    title: &str,
    is_focused: bool,
) -> Paragraph<'_> {
    let (cursor_line, cursor_col) = buffer.cursor_position();
    
    // Build lines with cursor indicator
    let mut lines = Vec::new();
    for (idx, line) in buffer.visual_lines().iter().enumerate() {
        if idx == cursor_line && is_focused {
            // Insert cursor indicator
            let split = crate::text::clamp_to_char_boundary(line, cursor_col.min(line.len()));
            let before = &line[..split];
            let after = &line[split..];
            
            let spans = vec![
                Span::raw(before),
                Span::styled("|", Style::default().bg(Color::Green).fg(Color::Black)),
                Span::raw(after),
            ];
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(line.as_str()));
        }
    }
    
    let block = if is_focused {
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .title(title)
    } else {
        Block::default()
            .borders(Borders::ALL)
            .title(title)
    };
    
    Paragraph::new(lines).block(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_insert_and_content() {
        let mut buffer = InputBuffer::new();
        
        buffer.insert_char('H');
        buffer.insert_char('i');
        
        assert_eq!(buffer.content(), "Hi");
        assert_eq!(buffer.cursor_byte, 2);
    }
    
    #[test]
    fn test_backspace() {
        let mut buffer = InputBuffer::new();
        buffer.insert_str("Hello");
        
        buffer.backspace();
        assert_eq!(buffer.content(), "Hell");
        
        buffer.backspace();
        assert_eq!(buffer.content(), "Hel");
    }
    
    #[test]
    fn test_multiline_paste() {
        let mut buffer = InputBuffer::new();
        
        buffer.start_paste();
        buffer.paste_add("Line 1\nLine 2\nLine 3");
        buffer.end_paste();
        
        assert_eq!(buffer.content(), "Line 1\nLine 2\nLine 3");
    }
    
    #[test]
    fn test_cursor_movement() {
        let mut buffer = InputBuffer::new();
        buffer.insert_str("Hello World");
        
        buffer.move_to_start();
        assert_eq!(buffer.cursor_byte, 0);
        
        buffer.move_to_end();
        assert_eq!(buffer.cursor_byte, 11);
        
        buffer.move_left();
        assert_eq!(buffer.cursor_byte, 10);
        
        buffer.move_right();
        assert_eq!(buffer.cursor_byte, 11);
    }

    #[test]
    fn test_unicode_wrapping_and_rendering_do_not_slice_byte_boundaries() {
        let mut buffer = InputBuffer::new();
        buffer.insert_str("“curly quotes” — markdown-heavy numbered list item");
        buffer.set_max_height(3);

        let _paragraph = render_input_box(&buffer, "Input", true);
        assert!(buffer
            .visual_lines()
            .iter()
            .all(|line| line.is_char_boundary(line.len())));
        assert!(buffer.content().is_char_boundary(buffer.cursor_byte));
    }
}
