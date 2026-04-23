//! Internal clipboard buffer
//!
//! Stores copied text when system clipboard is unavailable

/// Internal buffer for clipboard operations
#[derive(Debug, Clone, Default)]
pub struct ClipboardBuffer {
    content: Option<String>,
}

impl ClipboardBuffer {
    /// Create new empty buffer
    pub fn new() -> Self {
        Self { content: None }
    }

    /// Set buffer content
    pub fn set(&mut self, text: impl Into<String>) {
        self.content = Some(text.into());
    }

    /// Get buffer content
    pub fn get(&self) -> Option<&str> {
        self.content.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_operations() {
        let mut buffer = ClipboardBuffer::new();
        buffer.set("Hello, World!");
        assert_eq!(buffer.get(), Some("Hello, World!"));
    }
}
