/// Layout state management
///
/// Tracks terminal dimensions and calculates widget positions
#[derive(Debug, Clone)]
pub struct LayoutState {
    pub width: u16,
    pub height: u16,
    pub chat_area: ratatui::layout::Rect,
    pub input_area: ratatui::layout::Rect,
    pub status_area: ratatui::layout::Rect,
}

impl LayoutState {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            chat_area: ratatui::layout::Rect::default(),
            input_area: ratatui::layout::Rect::default(),
            status_area: ratatui::layout::Rect::default(),
        }
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;

        // Recalculate areas based on new dimensions
        let total_height = height;

        // Status bar: 1 line at top
        self.status_area = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width,
            height: 1,
        };

        // Input bar: 3 lines at bottom
        self.input_area = ratatui::layout::Rect {
            x: 0,
            y: total_height - 3,
            width,
            height: 3,
        };

        // Chat thread: everything in between
        let chat_height = total_height.saturating_sub(4); // 1 for status + 3 for input
        self.chat_area = ratatui::layout::Rect {
            x: 0,
            y: 1,
            width,
            height: chat_height,
        };
    }
}

impl Default for LayoutState {
    fn default() -> Self {
        Self::new()
    }
}
