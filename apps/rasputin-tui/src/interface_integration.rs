//! Interface Layer Integration
//!
//! Connects the existing forge_runtime events to the rasputin-interface
//! transparency layer for user-facing output formatting.
//!
//! NOTE: Only event transformation is currently used. Intent processing
//! and clarification features were removed per CF-3 hardening.

use rasputin_interface::bridge::{RuntimeAdapter, RuntimeEvent};
use rasputin_interface::transparency::{RenderingPolicy, TransparencyMapper};
use rasputin_interface::types::UserFacingEvent;

/// Integrates the interface layer with the existing TUI
///
/// This struct provides event transformation for displaying Forge runtime
/// events in a user-friendly format. Other interface features (intent processing,
/// clarification, conversation management) have been removed per hardening CF-3.
#[derive(Debug)]
pub struct InterfaceIntegration {
    mapper: TransparencyMapper,
    adapter: RuntimeAdapter,
}

impl InterfaceIntegration {
    pub fn new() -> Self {
        Self {
            mapper: TransparencyMapper::new().with_policy(RenderingPolicy::concise()),
            adapter: RuntimeAdapter::new(),
        }
    }

    /// Convert a runtime event to user-facing event
    ///
    /// This is the key integration point - intercepts raw runtime events
    /// and transforms them into human-readable messages
    pub fn transform_event(
        &self,
        event: &crate::forge_runtime::RuntimeEvent,
    ) -> Option<UserFacingEvent> {
        // Convert TUI RuntimeEvent to bridge RuntimeEvent
        let bridge_event = self.convert_to_bridge(event);

        // Adapt to OrchestratorEvent
        let orchestrator_event = self.adapter.adapt_runtime_event(&bridge_event)?;

        // Map to user-facing event
        self.mapper.map(&orchestrator_event)
    }

    /// Convert TUI RuntimeEvent to bridge RuntimeEvent format
    fn convert_to_bridge(&self, event: &crate::forge_runtime::RuntimeEvent) -> RuntimeEvent {
        use crate::forge_runtime::RuntimeEvent as TuiEvent;

        match event {
            TuiEvent::Init {
                session_id,
                task,
                planner,
            } => RuntimeEvent::Init {
                session_id: session_id.clone(),
                task: task.clone(),
                planner: planner.clone(),
            },
            TuiEvent::IterationStart { iteration } => RuntimeEvent::IterationStart {
                iteration: *iteration,
            },
            TuiEvent::PreflightPassed => RuntimeEvent::PreflightPassed,
            TuiEvent::PlannerOutput { raw, output_type } => RuntimeEvent::PlannerOutput {
                raw: raw.clone(),
                output_type: output_type.clone(),
            },
            TuiEvent::ProtocolValidation { status, reason } => RuntimeEvent::ProtocolValidation {
                status: status.clone(),
                reason: reason.clone(),
            },
            TuiEvent::ToolCall { name, arguments } => RuntimeEvent::ToolCall {
                name: name.clone(),
                arguments: arguments.clone(),
            },
            TuiEvent::ToolExecuting { name } => RuntimeEvent::ToolExecuting { name: name.clone() },
            TuiEvent::ToolResult {
                name,
                success,
                output,
                error,
            } => RuntimeEvent::ToolResult {
                name: name.clone(),
                success: *success,
                output: output.clone(),
                error: error.clone(),
            },
            TuiEvent::MutationsDetected { count } => {
                RuntimeEvent::MutationsDetected { count: *count }
            }
            TuiEvent::ValidationRunning => RuntimeEvent::ValidationRunning,
            TuiEvent::ValidationResult { decision, message } => RuntimeEvent::ValidationResult {
                decision: decision.clone(),
                message: message.clone(),
            },
            TuiEvent::ValidationStage {
                stage,
                status,
                duration_ms,
                summary,
            } => {
                // Convert ValidationStageStatus enum to string
                let status_str = match status {
                    crate::forge_runtime::ValidationStageStatus::Running => "running",
                    crate::forge_runtime::ValidationStageStatus::Passed => "passed",
                    crate::forge_runtime::ValidationStageStatus::Failed => "failed",
                    crate::forge_runtime::ValidationStageStatus::Skipped => "skipped",
                };
                RuntimeEvent::ValidationStage {
                    stage: stage.clone(),
                    status: status_str.to_string(),
                    duration_ms: *duration_ms,
                    summary: summary.clone(),
                }
            }
            TuiEvent::StateCommitting { files_written } => RuntimeEvent::StateCommitting {
                files_written: files_written.clone(),
            },
            TuiEvent::Completion { reason } => RuntimeEvent::Completion {
                reason: reason.clone(),
            },
            TuiEvent::Failure {
                reason,
                recoverable,
            } => RuntimeEvent::Failure {
                reason: reason.clone(),
                recoverable: *recoverable,
            },
            TuiEvent::RepairLoop {
                attempt,
                max,
                reason,
            } => RuntimeEvent::RepairLoop {
                attempt: *attempt,
                max: *max,
                reason: reason.clone(),
            },
            TuiEvent::Finished {
                success,
                iterations,
                error,
            } => RuntimeEvent::Finished {
                success: *success,
                iterations: *iterations,
                error: error.clone(),
            },
            TuiEvent::BrowserPreview {
                url,
                port,
                directory,
            } => RuntimeEvent::BrowserPreview {
                url: url.clone(),
                port: *port,
                directory: directory.clone(),
            },
            // ContextAssembly is TUI-specific, not sent to bridge
            TuiEvent::ContextAssembly { .. } => {
                return RuntimeEvent::ValidationRunning; // Neutral event placeholder
            }
        }
    }
}

impl Default for InterfaceIntegration {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a user-facing event for display in the chat
pub fn format_user_event(event: &UserFacingEvent) -> String {
    match event {
        UserFacingEvent::ActionStarted { description } => description.to_string(),
        UserFacingEvent::ActionCompleted { description } => {
            format!("✓ {}", description)
        }
        UserFacingEvent::ActionFailed { description, error } => {
            format!("✗ {}: {}", description, error)
        }
        UserFacingEvent::ValidationRunning { stage: None } => "Validating...".to_string(),
        UserFacingEvent::ValidationRunning { stage: Some(stage) } => {
            format!("Validating {}...", stage)
        }
        UserFacingEvent::ValidationPassed => "✓ Validation passed".to_string(),
        UserFacingEvent::ValidationFailed { reason, reverted } => {
            if *reverted {
                format!("✗ {} (changes reverted)", reason)
            } else {
                format!("✗ {}", reason)
            }
        }
        UserFacingEvent::WorkCompleted { summary, .. } => {
            format!("✓ {}", summary)
        }
        UserFacingEvent::WorkFailed { reason, .. } => {
            format!("✗ {}", reason)
        }
        UserFacingEvent::SystemMessage { content, .. } => content.clone(),
        UserFacingEvent::ClarificationQuestion { question, .. } => {
            format!("? {}", question)
        }
        UserFacingEvent::ApprovalRequested { request } => {
            format!("⏸ Approval needed: {}", request.description)
        }
    }
}
