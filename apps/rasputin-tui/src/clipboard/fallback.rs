//! Fallback clipboard for when system clipboard is unavailable.

use super::buffer::ClipboardBuffer;

/// Fallback clipboard using internal buffer
#[derive(Debug, Clone)]
pub struct FallbackClipboard {
    buffer: ClipboardBuffer,
}

impl FallbackClipboard {
    /// Create new fallback clipboard
    pub fn new() -> Self {
        Self {
            buffer: ClipboardBuffer::new(),
        }
    }

    /// Set text in fallback buffer
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.buffer.set(text);
    }

    pub fn cached_len(&self) -> usize {
        self.buffer.get().map_or(0, str::len)
    }
}

impl Default for FallbackClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_clipboard() {
        let mut clipboard = FallbackClipboard::new();
        clipboard.set_text("Hello");
        assert_eq!(clipboard.cached_len(), 5);
    }
}
