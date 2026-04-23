//! Main execution loop controller
//!
//! Phase 3 implementation: Execution orchestrator with approval gating

use crate::orchestrator::{ApprovalQueue, InterruptHandler, SessionManager};
use crate::types::{
    ApprovalRequest, AutoApprovePolicy, IntentSpec, OrchestratorState, UserFacingEvent,
};
use crate::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Controls the multi-step execution loop with approval gating
#[derive(Debug)]
pub struct ExecutionOrchestrator {
    session: SessionManager,
    approval_queue: Arc<Mutex<ApprovalQueue>>,
    interrupt_handler: InterruptHandler,
    event_sender: mpsc::Sender<UserFacingEvent>,
    current_intent: Option<IntentSpec>,
    auto_approve_policy: AutoApprovePolicy,
}

impl ExecutionOrchestrator {
    pub fn new(
        event_sender: mpsc::Sender<UserFacingEvent>,
        auto_approve_policy: AutoApprovePolicy,
    ) -> Self {
        Self {
            session: SessionManager::new(),
            approval_queue: Arc::new(Mutex::new(ApprovalQueue::new())),
            interrupt_handler: InterruptHandler::new(),
            event_sender,
            current_intent: None,
            auto_approve_policy,
        }
    }

    /// Start executing an intent
    pub async fn start_execution(&mut self, intent: IntentSpec) -> Result<()> {
        // Check if we can accept new execution
        if !self.session.current_state().accepts_input() {
            return Err(crate::InterfaceError::NotAcceptingInput(
                self.session.current_state().as_str().to_string(),
            ));
        }

        self.current_intent = Some(intent);
        self.session.transition_to(OrchestratorState::Executing)?;

        // Notify that execution started
        let _ = self
            .event_sender
            .send(UserFacingEvent::ActionStarted {
                description: "Starting execution...".to_string(),
            })
            .await;

        Ok(())
    }

    /// Process a runtime event during execution
    pub async fn process_runtime_event(
        &mut self,
        event: crate::types::OrchestratorEvent,
    ) -> Result<ProcessingResult> {
        // Check for interruption
        if self.interrupt_handler.is_interrupt_requested() {
            self.handle_interruption().await?;
            return Ok(ProcessingResult::Interrupted);
        }

        match event {
            crate::types::OrchestratorEvent::ToolExecuting { name, arguments } => {
                // Check if tool needs approval
                if let Some(approval) = self.check_approval_needed(&name, &arguments).await {
                    if !self.auto_approve_policy.allows(&approval.action_type) {
                        // Need user approval
                        self.session
                            .transition_to(OrchestratorState::AwaitingApproval)?;

                        let request = approval.clone();
                        self.approval_queue.lock().await.push(approval);

                        // Send approval request to UI
                        let _ = self
                            .event_sender
                            .send(UserFacingEvent::ApprovalRequested { request })
                            .await;

                        return Ok(ProcessingResult::AwaitingApproval);
                    }
                }

                // Auto-approved, continue
                let _ = self
                    .event_sender
                    .send(UserFacingEvent::ActionStarted {
                        description: format!("Executing {}...", name),
                    })
                    .await;

                Ok(ProcessingResult::Continue)
            }

            crate::types::OrchestratorEvent::ToolCompleted { name, result } => {
                let event = if result.success {
                    UserFacingEvent::ActionCompleted {
                        description: format!("{} completed", name),
                    }
                } else {
                    UserFacingEvent::ActionFailed {
                        description: format!("{} failed", name),
                        error: result
                            .error_preview
                            .unwrap_or_else(|| "Unknown error".to_string()),
                    }
                };

                let _ = self.event_sender.send(event).await;
                Ok(ProcessingResult::Continue)
            }

            crate::types::OrchestratorEvent::ValidationStarted { .. } => {
                self.session.transition_to(OrchestratorState::Validating)?;
                let _ = self
                    .event_sender
                    .send(UserFacingEvent::ValidationRunning { stage: None })
                    .await;
                Ok(ProcessingResult::Continue)
            }

            crate::types::OrchestratorEvent::ValidationStageCompleted { stage, passed } => {
                if stage == "final" && passed {
                    let _ = self
                        .event_sender
                        .send(UserFacingEvent::ValidationPassed)
                        .await;
                }
                Ok(ProcessingResult::Continue)
            }

            crate::types::OrchestratorEvent::ExecutionCompleted { success, summary } => {
                self.session.transition_to(OrchestratorState::Completed)?;

                let event = if success {
                    UserFacingEvent::WorkCompleted {
                        summary,
                        files_changed: Vec::new(),
                    }
                } else {
                    UserFacingEvent::WorkFailed {
                        reason: summary,
                        suggestion: Some("You can try again or modify the request".to_string()),
                    }
                };

                let _ = self.event_sender.send(event).await;
                self.current_intent = None;

                Ok(ProcessingResult::Complete)
            }

            crate::types::OrchestratorEvent::ExecutionFailed {
                reason,
                recoverable,
            } => {
                if recoverable {
                    self.session.transition_to(OrchestratorState::Recovering)?;
                    let _ = self
                        .event_sender
                        .send(UserFacingEvent::SystemMessage {
                            content: format!("Retrying: {}", reason),
                            level: crate::types::MessageLevel::Warning,
                        })
                        .await;
                    Ok(ProcessingResult::Retry)
                } else {
                    self.session.transition_to(OrchestratorState::Failed)?;
                    let _ = self
                        .event_sender
                        .send(UserFacingEvent::WorkFailed {
                            reason,
                            suggestion: None,
                        })
                        .await;
                    self.current_intent = None;
                    Ok(ProcessingResult::Failed)
                }
            }

            _ => Ok(ProcessingResult::Continue),
        }
    }

    /// Grant approval for current pending action
    pub async fn grant_approval(&mut self, duration: crate::types::GrantDuration) -> Result<()> {
        let mut queue = self.approval_queue.lock().await;
        queue.grant_current(duration);
        drop(queue);

        // Resume execution
        if self.session.current_state() == OrchestratorState::AwaitingApproval {
            self.session.transition_to(OrchestratorState::Executing)?;
        }

        let _ = self
            .event_sender
            .send(UserFacingEvent::ActionStarted {
                description: "Resuming execution...".to_string(),
            })
            .await;

        Ok(())
    }

    /// Deny approval - cancel current action
    pub async fn deny_approval(&mut self, reason: Option<String>) -> Result<()> {
        let mut queue = self.approval_queue.lock().await;
        queue.deny_current(reason.clone());
        drop(queue);

        self.session.transition_to(OrchestratorState::Idle)?;
        self.current_intent = None;

        let _ = self
            .event_sender
            .send(UserFacingEvent::WorkFailed {
                reason: reason.unwrap_or_else(|| "User denied approval".to_string()),
                suggestion: Some("You can modify the request and try again".to_string()),
            })
            .await;

        Ok(())
    }

    /// Request interruption (pause/cancel)
    pub fn request_interrupt(&self) {
        self.interrupt_handler.request_interrupt();
    }

    /// Resume from interruption
    pub async fn resume(&mut self) -> Result<()> {
        self.interrupt_handler.clear_interrupt();

        if self.session.current_state() == OrchestratorState::Idle && self.current_intent.is_some()
        {
            self.session.transition_to(OrchestratorState::Executing)?;
        }

        let _ = self
            .event_sender
            .send(UserFacingEvent::SystemMessage {
                content: "Resuming execution...".to_string(),
                level: crate::types::MessageLevel::Info,
            })
            .await;

        Ok(())
    }

    /// Get current execution state
    pub fn current_state(&self) -> OrchestratorState {
        self.session.current_state()
    }

    /// Check if orchestrator can accept user input
    pub fn accepts_input(&self) -> bool {
        self.session.accepts_input()
    }

    /// Check if execution is interruptible
    pub fn is_interruptible(&self) -> bool {
        self.session.current_state().interruptible()
    }

    /// Check if a tool needs approval
    async fn check_approval_needed(
        &self,
        name: &str,
        _arguments: &crate::types::events::ToolArguments,
    ) -> Option<ApprovalRequest> {
        let action_type = match name {
            "read_file" => crate::types::ActionType::FileRead,
            "write_file" => crate::types::ActionType::FileWrite,
            "apply_patch" => crate::types::ActionType::FilePatch,
            "delete_file" => crate::types::ActionType::FileDelete,
            "execute_command" => crate::types::ActionType::CommandExecute,
            _ => crate::types::ActionType::DestructiveTool,
        };

        // Check if already approved
        let queue = self.approval_queue.lock().await;
        if let Some(current) = queue.peek() {
            if current.action_type == action_type && queue.is_approved(&current.request_id) {
                return None; // Already approved
            }
        }
        drop(queue);

        // Create approval request
        Some(
            ApprovalRequest::new(action_type, format!("Execute {} command", name))
                .with_rationale(format!("This will run the {} tool", name)),
        )
    }

    /// Handle interruption request
    async fn handle_interruption(&mut self) -> Result<()> {
        self.interrupt_handler.clear_interrupt();
        self.session.transition_to(OrchestratorState::Idle)?;

        let _ = self
            .event_sender
            .send(UserFacingEvent::SystemMessage {
                content: "Execution paused. Say 'continue' to resume.".to_string(),
                level: crate::types::MessageLevel::Info,
            })
            .await;

        Ok(())
    }
}

impl Default for ExecutionOrchestrator {
    fn default() -> Self {
        let (sender, _receiver) = mpsc::channel(100);
        Self::new(sender, AutoApprovePolicy::SafeOnly)
    }
}

/// Result of processing a runtime event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingResult {
    Continue,         // Keep executing
    AwaitingApproval, // Paused for user approval
    Interrupted,      // User requested pause
    Retry,            // Recoverable error, will retry
    Complete,         // Execution finished successfully
    Failed,           // Execution failed fatally
}

/// Old name for backward compatibility
pub type ExecutionLoop = ExecutionOrchestrator;
