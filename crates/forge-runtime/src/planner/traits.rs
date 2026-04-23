//! Planner Trait Definition
//!
//! Canonical interface for all planner implementations.
//! Runtime depends on this trait, not concrete implementations.

use crate::planner::state_view::StateView;
use crate::types::{ForgeError, PlannerOutput};

/// Planner trait - swappable planner interface
///
/// Implementations:
/// - StubPlanner: Deterministic rule-based (testing, deterministic mode)
/// - ModelPlanner: Real model-backed with output normalization
///
/// Contract:
/// - Receives StateView (constrained, read-only)
/// - Returns exactly one PlannerOutput
/// - Never mutates state directly
/// - Never executes tools directly
/// - Fail-closed on errors
pub trait Planner: Send + Sync {
    /// Generate next planning decision based on current state view
    ///
    /// Returns:
    /// - Ok(PlannerOutput): One of ToolCall, Completion, or Failure
    /// - Err(ForgeError): Planner-specific error (backend unavailable, timeout, etc.)
    ///
    /// The runtime will validate and execute the returned PlannerOutput.
    fn generate(&self, state: &StateView) -> Result<PlannerOutput, ForgeError>;

    /// PHASE 4: Generate raw planner output for hardened validation
    ///
    /// This method returns the raw string output from the planner
    /// (JSON or text) BEFORE any parsing or normalization.
    ///
    /// The runtime's ProtocolValidator will validate this raw output
    /// as the SINGLE CHOKE POINT for all planner output.
    ///
    /// Default implementation delegates to generate() and serializes,
    /// but real implementations should return the actual raw output.
    fn generate_raw(&self, state: &StateView) -> Result<String, ForgeError> {
        // Default: serialize the typed output
        // Real implementations (ModelPlanner) should override this
        // to return the actual raw model output
        let output = self.generate(state)?;
        Ok(serialize_planner_output(&output))
    }

    /// Planner identification for logging/debugging
    fn planner_type(&self) -> &'static str;

    /// Check if planner backend is healthy/available
    /// Default implementation returns Ok(())
    #[allow(dead_code)]
    fn health_check(&self) -> Result<(), ForgeError> {
        Ok(())
    }
}

/// Helper to serialize PlannerOutput to canonical JSON string
fn serialize_planner_output(output: &PlannerOutput) -> String {
    match output {
        PlannerOutput::ToolCall(tc) => {
            let args: serde_json::Map<String, serde_json::Value> = tc
                .arguments
                .as_map()
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();

            format!(
                r#"{{"type":"tool_call","tool_call":{{"name":"{}","arguments":{}}}}}"#,
                tc.name.as_str(),
                serde_json::Value::Object(args)
            )
        }
        PlannerOutput::Completion { reason } => {
            format!(
                r#"{{"type":"completion","reason":"{}"}}"#,
                reason.as_str().replace('"', "\\\"")
            )
        }
        PlannerOutput::Failure {
            reason,
            recoverable,
        } => {
            format!(
                r#"{{"type":"failure","reason":"{}","recoverable":{}}}"#,
                reason.replace('"', "\\\""),
                recoverable
            )
        }
    }
}

/// Boxed planner for dynamic dispatch in runtime
pub type BoxedPlanner = Box<dyn Planner>;

/// Planner factory for creating planners based on configuration
#[allow(dead_code)]
pub trait PlannerFactory: Send + Sync {
    fn create(&self) -> Result<BoxedPlanner, ForgeError>;
}
