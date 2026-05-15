//! Rasputin Interface Layer
//!
//! Transitional interface and transparency primitives above the deterministic
//! Forge core.
//!
//! Current product status: the active TUI hot path uses this crate only for
//! runtime-event transformation in `apps/rasputin-tui/src/interface_integration.rs`.
//! The intent, orchestrator, approval, and session modules are retained as
//! experimental public-ish surfaces until they are either promoted into the hot
//! path or retired with docs/tests updated.

#![deny(unused_must_use)]
// See the crate-level status note above. This broad allow is intentionally
// scoped to the transitional interface crate, whose exported modules are kept
// for compatibility and roadmap evaluation rather than active TUI control flow.
#![allow(dead_code)]

pub mod transparency;
pub mod types;

// Phase 2+ modules (will be implemented in subsequent phases)
pub mod bridge;
pub mod interface;
pub mod orchestrator;

pub use interface::{Clarifier, ConversationManager, IntentRefiner};
pub use orchestrator::{
    ApprovalQueue, ExecutionLoop, ExecutionOrchestrator, InterruptHandler, ProcessingResult,
    SessionManager,
};
pub use transparency::TransparencyMapper;
pub use types::*;

use thiserror::Error;

/// Errors from the interface layer
#[derive(Error, Debug)]
pub enum InterfaceError {
    #[error("Orchestrator not accepting input in current state: {0}")]
    NotAcceptingInput(String),

    #[error("No intent available for execution")]
    NoIntent,

    #[error("Failed to resolve reference: {0}")]
    ResolutionFailed(String),

    #[error("Runtime error: {0}")]
    RuntimeError(String),

    #[error("State transition failed: from {from} to {to}")]
    StateTransitionFailed { from: String, to: String },
}

/// Result type for interface operations
pub type Result<T> = std::result::Result<T, InterfaceError>;
