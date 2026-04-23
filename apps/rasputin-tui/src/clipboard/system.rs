//! System clipboard integration
//!
//! Provides access to OS clipboard when available

use clipboard::{ClipboardContext, ClipboardProvider};
use std::error::Error;

/// System clipboard wrapper
pub struct SystemClipboard {
    context: ClipboardContext,
}

impl std::fmt::Debug for SystemClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemClipboard").finish()
    }
}

impl SystemClipboard {
    /// Create new system clipboard
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let context = ClipboardContext::new()?;
        Ok(Self { context })
    }

    /// Set clipboard text
    pub fn set_text(&mut self, text: &str) -> Result<(), Box<dyn Error>> {
        self.context.set_contents(text.to_owned())
    }
}

/// Try to create system clipboard
pub fn try_get_system_clipboard() -> Result<SystemClipboard, Box<dyn Error>> {
    SystemClipboard::new()
}
