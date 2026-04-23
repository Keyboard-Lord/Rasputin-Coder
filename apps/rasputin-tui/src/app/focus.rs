//! Focus management for TUI panes
//!
//! Tracks which UI component has focus and handles focus transitions

/// Which UI component currently has focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// Chat history pane
    ChatPane,
    /// Input text box
    InputBox,
    /// File preview pane
    PreviewPane,
    /// Archive browser modal
    ArchiveBrowser,
}

impl FocusTarget {
    /// Get next focus target in cycle order
    pub fn next(&self) -> Self {
        match self {
            FocusTarget::ChatPane => FocusTarget::InputBox,
            FocusTarget::InputBox => FocusTarget::PreviewPane,
            FocusTarget::PreviewPane => FocusTarget::ArchiveBrowser,
            FocusTarget::ArchiveBrowser => FocusTarget::ChatPane,
        }
    }

    /// Get previous focus target in cycle order
    pub fn prev(&self) -> Self {
        match self {
            FocusTarget::ChatPane => FocusTarget::ArchiveBrowser,
            FocusTarget::InputBox => FocusTarget::ChatPane,
            FocusTarget::PreviewPane => FocusTarget::InputBox,
            FocusTarget::ArchiveBrowser => FocusTarget::PreviewPane,
        }
    }
}

/// Focus state management
#[derive(Debug, Clone)]
pub struct FocusState {
    /// Current focus target
    pub current: FocusTarget,

    /// Previous focus target for cycle context
    pub previous: Option<FocusTarget>,

    /// Currently focused message ID in chat (for keyboard nav)
    pub chat_focused_message: Option<String>,
}

impl FocusState {
    /// Create new focus state starting in chat pane
    pub fn new() -> Self {
        Self {
            current: FocusTarget::ChatPane,
            previous: None,
            chat_focused_message: None,
        }
    }

    /// Cycle to next focus target
    pub fn cycle_next(&mut self) {
        let old = self.current;
        self.current = self.current.next();
        self.previous = Some(old);
    }

    /// Cycle to previous focus target
    pub fn cycle_prev(&mut self) {
        let old = self.current;
        self.current = self.current.prev();
        self.previous = Some(old);
    }

    /// Check if specific target has focus
    pub fn has_focus(&self, target: FocusTarget) -> bool {
        self.current == target
    }
}

impl Default for FocusState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_cycle() {
        let mut focus = FocusState::new();
        assert_eq!(focus.current, FocusTarget::ChatPane);

        focus.cycle_next();
        assert_eq!(focus.current, FocusTarget::InputBox);

        focus.cycle_next();
        assert_eq!(focus.current, FocusTarget::PreviewPane);

        focus.cycle_prev();
        assert_eq!(focus.current, FocusTarget::InputBox);
    }
}
