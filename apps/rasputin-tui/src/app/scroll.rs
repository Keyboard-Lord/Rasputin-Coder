//! Chat scroll state management
//!
//! Provides stable scroll behavior for chat history with:
//! - Explicit offset tracking from bottom
//! - Auto-scroll with user interrupt
//! - Virtualization support for long histories
//! - New message indicators

use ratatui::layout::Rect;

/// Scroll state for chat history view
#[derive(Debug, Clone, Default)]
pub struct ChatScrollState {
    /// Lines from bottom (0 = at latest message)
    pub offset_from_bottom: usize,

    /// Terminal rows available for chat viewport
    pub viewport_height: usize,

    /// Total content lines across all messages (cached)
    pub total_content_lines: usize,

    /// Auto-scroll enabled (disabled when user scrolls up)
    pub auto_scroll: bool,

    /// Messages arrived while user was scrolled up
    pub pending_message_count: usize,

    /// Message ID used as scroll anchor for stability
    pub anchor_message_id: Option<String>,

    /// Buffer lines for virtualization (render extra above/below viewport)
    pub virtualization_buffer: usize,
}

impl ChatScrollState {
    /// Create new scroll state with default buffer
    pub fn new() -> Self {
        Self {
            offset_from_bottom: 0,
            viewport_height: 0,
            total_content_lines: 0,
            auto_scroll: true,
            pending_message_count: 0,
            anchor_message_id: None,
            virtualization_buffer: 5,
        }
    }

    /// Update viewport dimensions
    pub fn update_viewport(&mut self, area: Rect) {
        self.viewport_height = area.height as usize;
    }

    /// Handle user scroll action
    ///
    /// Positive delta = scroll up (older messages)
    /// Negative delta = scroll down (newer messages)
    pub fn scroll(&mut self, delta: i32) {
        if delta < 0 {
            // Scrolling down toward newer messages
            let abs_delta = delta.unsigned_abs() as usize;
            if abs_delta >= self.offset_from_bottom {
                // Reached bottom
                self.jump_to_latest();
            } else {
                self.offset_from_bottom -= abs_delta;
                self.auto_scroll = false;
            }
        } else {
            // Scrolling up toward older messages
            let max_offset = self
                .total_content_lines
                .saturating_sub(self.viewport_height);
            self.offset_from_bottom = (self.offset_from_bottom + delta as usize).min(max_offset);
            self.auto_scroll = false;
        }
    }

    /// Jump to latest message (bottom)
    pub fn jump_to_latest(&mut self) {
        self.offset_from_bottom = 0;
        self.auto_scroll = true;
        self.pending_message_count = 0;
        self.anchor_message_id = None;
    }

    /// Jump to first message (top)
    pub fn jump_to_first(&mut self) {
        let max_offset = self
            .total_content_lines
            .saturating_sub(self.viewport_height);
        self.offset_from_bottom = max_offset;
        self.auto_scroll = false;
        self.pending_message_count = 0;
    }

    /// Page up - scroll one viewport height
    pub fn page_up(&mut self) {
        let page_size = self.viewport_height.saturating_sub(2); // Keep some context
        self.scroll(page_size as i32);
    }

    /// Page down - scroll one viewport height
    pub fn page_down(&mut self) {
        let page_size = self.viewport_height.saturating_sub(2);
        self.scroll(-(page_size as i32));
    }

    /// Calculate visible line range for virtualization
    ///
    /// Returns (start_line, end_line) for rendering
    pub fn visible_range(&self) -> (usize, usize) {
        let total = self.total_content_lines;
        let viewport = self.viewport_height;
        let buffer = self.virtualization_buffer;

        // Lines from top = total - offset_from_bottom - viewport
        let lines_from_top = total.saturating_sub(self.offset_from_bottom + viewport);

        let start = lines_from_top.saturating_sub(buffer);
        let end = (lines_from_top + viewport + buffer).min(total);

        (start, end)
    }

    /// Update total content line count
    ///
    /// Called when messages are added/removed or window resized
    pub fn update_total_lines(&mut self, total_lines: usize) {
        self.total_content_lines = total_lines;

        // Ensure offset is still valid
        let max_offset = total_lines.saturating_sub(self.viewport_height);
        if self.offset_from_bottom > max_offset {
            self.offset_from_bottom = max_offset;
        }
    }

    /// Check if there are pending messages to view
    pub fn has_pending_messages(&self) -> bool {
        self.pending_message_count > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_up_disables_auto_scroll() {
        let mut scroll = ChatScrollState::new();
        scroll.viewport_height = 10;
        scroll.total_content_lines = 100;

        assert!(scroll.auto_scroll);
        scroll.scroll(5); // Scroll up
        assert!(!scroll.auto_scroll);
        assert_eq!(scroll.offset_from_bottom, 5);
    }

    #[test]
    fn test_scroll_down_to_bottom_reenables_auto_scroll() {
        let mut scroll = ChatScrollState::new();
        scroll.viewport_height = 10;
        scroll.total_content_lines = 100;

        scroll.scroll(20); // Scroll up
        assert!(!scroll.auto_scroll);

        scroll.scroll(-20); // Scroll back to bottom
        assert!(scroll.auto_scroll);
        assert_eq!(scroll.offset_from_bottom, 0);
    }

    #[test]
    fn test_visible_range_calculation() {
        let mut scroll = ChatScrollState::new();
        scroll.viewport_height = 10;
        scroll.total_content_lines = 100;
        scroll.offset_from_bottom = 20;
        scroll.virtualization_buffer = 2;

        let (start, end) = scroll.visible_range();

        // With offset 20 from bottom, viewport 10, total 100:
        // Lines from top = 100 - 20 - 10 = 70
        // With buffer 2: start = 68, end = 82
        assert_eq!(start, 68);
        assert_eq!(end, 82);
    }

    #[test]
    fn test_page_up_preserves_context() {
        let mut scroll = ChatScrollState::new();
        scroll.viewport_height = 20;
        scroll.total_content_lines = 100;

        scroll.page_up();

        // Should scroll by height - 2 = 18 lines
        assert_eq!(scroll.offset_from_bottom, 18);
    }
}
