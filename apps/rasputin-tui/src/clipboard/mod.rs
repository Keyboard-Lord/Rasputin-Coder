//! Clipboard management for copy/paste operations
//!
//! Provides:
//! - System clipboard integration when available
//! - Internal buffer fallback for SSH/terminals without clipboard
//! - Clean text extraction for copy operations

pub mod buffer;
pub mod fallback;
pub mod system;

pub use fallback::FallbackClipboard;
pub use system::{SystemClipboard, try_get_system_clipboard};

use crate::state::Message;

/// Unified clipboard interface
pub struct Clipboard {
    system: Option<SystemClipboard>,
    fallback: FallbackClipboard,
}

impl std::fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clipboard")
            .field("system", &self.system.is_some())
            .field("fallback_cached_bytes", &self.fallback.cached_len())
            .finish()
    }
}

impl Clipboard {
    /// Initialize clipboard (tries system first, falls back to internal)
    pub fn new() -> Self {
        let system = try_get_system_clipboard().ok();
        let fallback = FallbackClipboard::new();

        Self { system, fallback }
    }

    /// Copy text to clipboard
    pub fn copy_text(&mut self, text: &str) -> CopyResult {
        // Try system clipboard first
        if let Some(ref mut sys) = self.system {
            match sys.set_text(text) {
                Ok(_) => return CopyResult::CopiedToSystem,
                Err(_) => {
                    // Fall through to fallback
                }
            }
        }

        // Use fallback
        self.fallback.set_text(text);
        CopyResult::CopiedToInternal
    }

    /// Copy a message's source text (clean, no formatting)
    pub fn copy_message(&mut self, message: &Message) -> CopyResult {
        // Always use source_text for clean copy
        let text_to_copy = if !message.source_text.is_empty() {
            &message.source_text
        } else {
            &message.content
        };

        let cleaned = clean_text_for_copy(text_to_copy);
        self.copy_text(&cleaned)
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a copy operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyResult {
    /// Copied to system clipboard
    CopiedToSystem,
    /// Copied to internal buffer (no system clipboard available)
    CopiedToInternal,
}

impl CopyResult {
    /// Get user-facing message
    pub fn message(&self) -> &'static str {
        match self {
            CopyResult::CopiedToSystem => "Copied",
            CopyResult::CopiedToInternal => "Copied (internal)",
        }
    }
}

/// Utility to clean text for copying
///
/// Removes terminal artifacts, ANSI codes, etc.
pub fn clean_text_for_copy(text: &str) -> String {
    // Normalize line endings before stripping ANSI so lone carriage returns are preserved as
    // logical newlines instead of being swallowed by the escape-stripper.
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");

    // Strip ANSI escape sequences
    strip_ansi_escapes::strip_str(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_new() {
        let _clipboard = Clipboard::new();
    }

    #[test]
    fn test_copy_result_messages() {
        assert_eq!(CopyResult::CopiedToSystem.message(), "Copied");
        assert_eq!(CopyResult::CopiedToInternal.message(), "Copied (internal)");
    }

    #[test]
    fn test_clean_text() {
        let with_ansi = "\x1b[32mHello\x1b[0m World";
        let cleaned = clean_text_for_copy(with_ansi);
        assert_eq!(cleaned, "Hello World");

        let with_crlf = "Line 1\r\nLine 2\rLine 3";
        let cleaned = clean_text_for_copy(with_crlf);
        assert_eq!(cleaned, "Line 1\nLine 2\nLine 3");
    }
}
