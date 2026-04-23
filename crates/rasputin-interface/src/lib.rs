//! Rasputin Interface Layer
//!
//! Provides conversational interface, intent refinement, execution orchestration,
//! and action transparency above the deterministic Forge core.

#![deny(unused_must_use)]
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
