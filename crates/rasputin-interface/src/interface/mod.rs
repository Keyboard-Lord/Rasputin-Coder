//! Intent refinement and conversational interface
//!
//! Phase 4 implementation: Full conversational interface with clarification

pub mod clarifier;
pub mod conversation;
pub mod intent;

pub use crate::types::{ConversationTurn, IntentSpec, InteractionMode, UserMessage};
pub use clarifier::Clarifier;
pub use conversation::ConversationManager;
pub use intent::IntentRefiner;
