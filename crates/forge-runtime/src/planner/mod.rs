//! FORGE PHASE 3: Planner Model Integration
//!
//! This module implements the planner boundary for Forge,
//! treating the planner model as a private external intelligence backend.
//!
//! Core principle: The planner proposes, Forge decides.
//!
//! Architecture:
//! - Planner trait: Swappable planner interface
//! - StateView: Constrained view of runtime state for planners
//! - StubPlanner: Deterministic rule-based planner
//! - ModelPlanner: Real model-backed planner with output normalization
//! - PlannerAdapter: Output parsing and validation layer
//! - PlannerBackend: Isolated model communication interface
//!
//! Safety boundaries:
//! - Planner receives StateView, not raw runtime internals
//! - Planner returns PlannerOutput, never executes directly
//! - One-action-per-turn enforced at adapter layer
//! - Output normalization strips prose, extracts JSON
//! - Schema validation fail-closed
//! - Tool availability filtered by mode

#![allow(unused_imports)]

pub mod adapter;
pub mod compressed_planner;
pub mod css_transformer;
pub mod intelligent_stub;
pub mod model;
pub mod model_http;
pub mod output_adapter;
pub mod protocol_validator;
pub mod state_view;
pub mod stub;
pub mod traits;
pub mod validator;

// Re-export main types for convenience
pub use adapter::PlannerAdapter;
pub use compressed_planner::CompressedStatePlanner;
pub use css_transformer::{CompressionContext, CssSnapshot, CssTransformer, distilled_8b_prompt};
pub use intelligent_stub::IntelligentStubPlanner;
pub use model::{HttpPlannerBackend, ModelPlanner};
pub use model_http::{HttpModelPlanner, HttpOllamaBackend};
pub use output_adapter::{AdapterResult, CanonicalOutputAdapter, RepairLoopHandler};
pub use protocol_validator::{
    ReadRecord, ValidationContext, ValidationDecision, ValidationFailureClass,
};
pub use state_view::StateView;
pub use stub::StubPlanner;
pub use traits::Planner;
pub use validator::{
    PlannerValidator, ValidationContext as VContext, ValidationDecision as VDecision,
    ValidationFailureClass as VFailureClass,
};
