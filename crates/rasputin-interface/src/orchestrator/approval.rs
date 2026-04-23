//! Approval queue and gating
//!
//! Phase 3 implementation

use crate::types::{ApprovalRequest, GrantDuration};

/// Queue for pending approval requests
#[derive(Debug, Clone, Default)]
pub struct ApprovalQueue {
    pending: Vec<ApprovalRequest>,
    granted: Vec<(String, GrantDuration)>, // request_id -> duration
}

impl ApprovalQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, request: ApprovalRequest) {
        self.pending.push(request);
    }

    pub fn peek(&self) -> Option<&ApprovalRequest> {
        self.pending.first()
    }

    pub fn grant_current(&mut self, duration: GrantDuration) {
        if let Some(req) = self.pending.first() {
            self.granted.push((req.request_id.clone(), duration));
            self.pending.remove(0);
        }
    }

    pub fn deny_current(&mut self, _reason: Option<String>) {
        self.pending.remove(0);
    }

    pub fn pending(&self) -> Vec<ApprovalRequest> {
        self.pending.clone()
    }

    pub fn is_approved(&self, request_id: &str) -> bool {
        self.granted.iter().any(|(id, _)| id == request_id)
    }
}
