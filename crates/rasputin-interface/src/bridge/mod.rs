//! Bridge to Forge runtime
//!
//! Adapts the existing forge_bootstrap_rust Runtime to the interface layer

pub mod runtime_adapter;

pub use runtime_adapter::{RuntimeAdapter, RuntimeEvent};
