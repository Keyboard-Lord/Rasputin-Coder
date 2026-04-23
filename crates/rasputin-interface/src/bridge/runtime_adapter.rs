//! Adapter between existing forge_runtime events and interface types
//!
//! This is the bridge that connects the existing TUI's RuntimeEvent stream
//! to the new transparency layer.

use crate::types::events::{ToolArguments, ToolResultSummary};
use crate::types::OrchestratorEvent;

/// Adapts existing runtime events to orchestrator events
#[derive(Debug, Clone)]
pub struct RuntimeAdapter;

impl RuntimeAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Convert a runtime event from rasputin-tui's forge_runtime module
    /// to our internal OrchestratorEvent
    ///
    /// This is the key bridge function for Phase 1 integration
    pub fn adapt_runtime_event(&self, event: &RuntimeEvent) -> Option<OrchestratorEvent> {
        match event {
            RuntimeEvent::ToolExecuting { name, .. } => {
                Some(OrchestratorEvent::ToolExecuting {
                    name: name.clone(),
                    arguments: ToolArguments::default(), // Phase 1: simplified
                })
            }

            RuntimeEvent::ToolResult {
                name,
                success,
                output,
                error,
            } => Some(OrchestratorEvent::ToolCompleted {
                name: name.clone(),
                result: ToolResultSummary {
                    success: *success,
                    output_preview: output.clone(),
                    error_preview: error.clone(),
                    line_count: output.as_ref().map(|o| o.lines().count()),
                },
            }),

            RuntimeEvent::ValidationRunning => {
                Some(OrchestratorEvent::ValidationStarted { mutations_count: 0 })
            }

            RuntimeEvent::ValidationResult { decision, message } => {
                if decision == "accept" {
                    Some(OrchestratorEvent::ValidationStageCompleted {
                        stage: "final".to_string(),
                        passed: true,
                    })
                } else {
                    Some(OrchestratorEvent::ExecutionFailed {
                        reason: message.clone(),
                        recoverable: true,
                    })
                }
            }

            RuntimeEvent::ValidationStage { stage, status, .. } => match status.as_str() {
                "running" => Some(OrchestratorEvent::ValidationStageRunning {
                    stage: stage.clone(),
                }),
                "passed" => Some(OrchestratorEvent::ValidationStageCompleted {
                    stage: stage.clone(),
                    passed: true,
                }),
                "failed" => Some(OrchestratorEvent::ValidationStageCompleted {
                    stage: stage.clone(),
                    passed: false,
                }),
                _ => None,
            },

            RuntimeEvent::Finished { success, error, .. } => {
                if *success {
                    Some(OrchestratorEvent::ExecutionCompleted {
                        success: true,
                        summary: "Task completed successfully".to_string(),
                    })
                } else {
                    Some(OrchestratorEvent::ExecutionFailed {
                        reason: error.clone().unwrap_or_else(|| "Unknown error".to_string()),
                        recoverable: false,
                    })
                }
            }

            // Events that don't map directly (or are internal)
            RuntimeEvent::Init { .. } => None,
            RuntimeEvent::IterationStart { .. } => None,
            RuntimeEvent::PreflightPassed => None,
            RuntimeEvent::PlannerOutput { .. } => None,
            RuntimeEvent::ProtocolValidation { .. } => None,
            RuntimeEvent::ToolCall { .. } => None,
            RuntimeEvent::MutationsDetected { .. } => None,
            RuntimeEvent::StateCommitting { .. } => None,
            RuntimeEvent::Completion { .. } => None,
            RuntimeEvent::Failure { .. } => None,
            RuntimeEvent::RepairLoop { .. } => None,
            RuntimeEvent::BrowserPreview { .. } => None,
        }
    }
}

impl Default for RuntimeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// Mirror of rasputin-tui's RuntimeEvent for the bridge
// This avoids a direct dependency on the TUI crate
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    Init {
        session_id: String,
        task: String,
        planner: String,
    },
    IterationStart {
        iteration: u32,
    },
    PreflightPassed,
    PlannerOutput {
        raw: String,
        output_type: String,
    },
    ProtocolValidation {
        status: String,
        reason: Option<String>,
    },
    ToolCall {
        name: String,
        arguments: String,
    },
    ToolExecuting {
        name: String,
    },
    ToolResult {
        name: String,
        success: bool,
        output: Option<String>,
        error: Option<String>,
    },
    MutationsDetected {
        count: u32,
    },
    ValidationRunning,
    ValidationResult {
        decision: String,
        message: String,
    },
    ValidationStage {
        stage: String,
        status: String,
        duration_ms: u64,
        summary: Option<String>,
    },
    StateCommitting {
        files_written: Vec<String>,
    },
    Completion {
        reason: String,
    },
    Failure {
        reason: String,
        recoverable: bool,
    },
    RepairLoop {
        attempt: u32,
        max: u32,
        reason: String,
    },
    Finished {
        success: bool,
        iterations: u32,
        error: Option<String>,
    },
    BrowserPreview {
        url: String,
        port: u16,
        directory: String,
    },
}
