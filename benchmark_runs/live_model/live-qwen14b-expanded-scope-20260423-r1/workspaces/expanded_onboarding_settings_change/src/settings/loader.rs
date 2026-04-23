use super::defaults::{DEFAULT_MAX_RETRIES, DEFAULT_TIMEOUT_MS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettings {
    pub timeout_ms: u64,
    pub max_retries: u8,
}

pub fn load_defaults() -> RuntimeSettings {
    RuntimeSettings {
        timeout_ms: DEFAULT_TIMEOUT_MS,
        max_retries: DEFAULT_MAX_RETRIES,
    }
}
