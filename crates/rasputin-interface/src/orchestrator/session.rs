//! Session state management for orchestrator
//!
//! Phase 3 implementation

use crate::types::OrchestratorState;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

/// Manages orchestrator session lifecycle
#[derive(Debug, Clone)]
pub struct SessionManager {
    state: Arc<AtomicU32>, // Stores OrchestratorState as u32
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU32::new(OrchestratorState::Idle as u32)),
        }
    }

    pub fn current_state(&self) -> OrchestratorState {
        match self.state.load(Ordering::SeqCst) {
            0 => OrchestratorState::Idle,
            1 => OrchestratorState::RefiningIntent,
            2 => OrchestratorState::AwaitingClarification,
            3 => OrchestratorState::Executing,
            4 => OrchestratorState::AwaitingApproval,
            5 => OrchestratorState::Validating,
            6 => OrchestratorState::Committing,
            7 => OrchestratorState::Recovering,
            8 => OrchestratorState::Completed,
            9 => OrchestratorState::Failed,
            _ => OrchestratorState::Idle,
        }
    }

    pub fn transition_to(&self, new_state: OrchestratorState) -> crate::Result<()> {
        self.state.store(new_state as u32, Ordering::SeqCst);
        Ok(())
    }

    pub fn accepts_input(&self) -> bool {
        self.current_state().accepts_input()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
