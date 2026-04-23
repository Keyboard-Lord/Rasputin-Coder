//! Pause/resume/cancel handling
//!
//! Phase 3 implementation

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Handles interruption requests (Ctrl+C, pause button)
#[derive(Debug, Clone)]
pub struct InterruptHandler {
    requested: Arc<AtomicBool>,
}

impl InterruptHandler {
    pub fn new() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn request_interrupt(&self) {
        self.requested.store(true, Ordering::SeqCst);
    }

    pub fn clear_interrupt(&self) {
        self.requested.store(false, Ordering::SeqCst);
    }

    pub fn is_interrupt_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

impl Default for InterruptHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of an interrupt operation
#[derive(Debug, Clone)]
pub enum InterruptResult {
    Completed,
    Paused { message: String },
    Cancelled { message: String },
}
