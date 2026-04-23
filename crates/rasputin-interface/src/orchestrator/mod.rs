//! Execution orchestration - controls multi-step execution loop
//!
//! Phase 3 implementation

pub mod approval;
pub mod interrupt;
pub mod r#loop;
pub mod session;

pub use crate::types::{OrchestratorState, SessionContext, UncommittedWork};
pub use approval::ApprovalQueue;
pub use interrupt::InterruptHandler;
pub use r#loop::{ExecutionLoop, ExecutionOrchestrator, ProcessingResult};
pub use session::SessionManager;
