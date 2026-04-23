//! FORGE PHASE 1.5: Strongly Typed Internal Contracts
//!
//! All core types use newtypes and enums to prevent string-based errors.
//! JSON exists only at system boundaries.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

// ============================================================================
// IDENTIFIER TYPES (Newtypes for type safety)
// ============================================================================

/// Unique session identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        Self(format!("forge-{}-{:08x}", now, counter::next()))
    }

    /// Create from existing string (for persistence)
    #[allow(dead_code)]
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Tool name - validated at construction
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolName(String);

impl ToolName {
    pub fn new(name: &str) -> Result<Self, ForgeError> {
        // Validate tool name: alphanumeric + underscore, starts with letter
        if name.is_empty() {
            return Err(ForgeError::InvalidToolName("empty".to_string()));
        }
        if !name.chars().next().unwrap().is_alphabetic() {
            return Err(ForgeError::InvalidToolName(
                "must start with letter".to_string(),
            ));
        }
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(ForgeError::InvalidToolName(
                "alphanumeric and underscore only".to_string(),
            ));
        }
        Ok(Self(name.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// EXECUTION CONTEXT
// ============================================================================

/// Execution mode for the agent
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Analysis, // Read-only operations
    Edit,     // File modifications allowed
    #[allow(dead_code)]
    Fix, // Automatic error correction
    #[allow(dead_code)]
    Batch, // Multiple files, automated
}

impl ExecutionMode {
    pub fn allows_tool(&self, tool: &ToolName) -> bool {
        match self {
            ExecutionMode::Analysis => matches!(
                tool.as_str(),
                "read_file"
                    | "search"
                    | "grep_search"
                    | "list_dir"
                    | "dependency_graph"
                    | "symbol_index"
                    | "entrypoint_detector"
            ),
            ExecutionMode::Edit | ExecutionMode::Fix | ExecutionMode::Batch => {
                // All tools allowed
                true
            }
        }
    }
}

/// Context passed to tool execution
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    #[allow(dead_code)]
    pub session_id: SessionId,
    pub iteration: u32,
    pub mode: ExecutionMode,
    #[allow(dead_code)]
    pub working_dir: PathBuf,
}

// ============================================================================
// PLANNER TYPES
// ============================================================================

/// Output from the planner - exactly one action per turn
#[derive(Debug, Clone)]
pub enum PlannerOutput {
    ToolCall(ToolCall),
    Completion { reason: CompletionReason },
    Failure { reason: String, recoverable: bool },
}

/// Validated reason for completion
#[derive(Debug, Clone)]
pub struct CompletionReason(String);

impl CompletionReason {
    pub fn new(reason: &str) -> Self {
        Self(reason.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ============================================================================
// TOOL TYPES
// ============================================================================

/// A call to execute a specific tool
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: ToolName,
    pub arguments: ToolArguments,
}

/// Validated tool arguments
#[derive(Debug, Clone, Default)]
pub struct ToolArguments {
    inner: HashMap<String, String>,
}

impl ToolArguments {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.inner.insert(key.to_string(), value.to_string());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key).map(|s| s.as_str())
    }

    pub fn require(&self, key: &str) -> Result<&str, ForgeError> {
        self.get(key)
            .ok_or_else(|| ForgeError::MissingArgument(key.to_string()))
    }

    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.inner
    }
}

/// Result of tool execution
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<ToolError>,
    pub mutations: Vec<Mutation>,
    pub execution_time_ms: u64,
}

impl ToolResult {
    #[allow(dead_code)]
    pub fn success(output: &str, mutations: Vec<Mutation>) -> Self {
        Self {
            success: true,
            output: Some(output.to_string()),
            error: None,
            mutations,
            execution_time_ms: 0,
        }
    }

    #[allow(dead_code)]
    pub fn failure(error: ToolError) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(error),
            mutations: vec![],
            execution_time_ms: 0,
        }
    }
}

/// Specific tool error types
#[derive(Debug, Clone)]
pub enum ToolError {
    #[allow(dead_code)]
    MissingArgument(String),
    #[allow(dead_code)]
    InvalidArgument {
        name: String,
        value: String,
        reason: String,
    },
    IoError(String),
    #[allow(dead_code)]
    NotAllowed {
        tool: ToolName,
        mode: ExecutionMode,
    },
    ExecutionFailed(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::MissingArgument(name) => write!(f, "Missing argument: {}", name),
            ToolError::InvalidArgument {
                name,
                value,
                reason,
            } => {
                write!(f, "Invalid argument '{}={}': {}", name, value, reason)
            }
            ToolError::IoError(msg) => write!(f, "IO error: {}", msg),
            ToolError::NotAllowed { tool, mode } => {
                write!(f, "Tool '{}' not allowed in {:?} mode", tool, mode)
            }
            ToolError::ExecutionFailed(msg) => write!(f, "Execution failed: {}", msg),
        }
    }
}

// ============================================================================
// MUTATION TYPES
// ============================================================================

/// A file system mutation performed by a tool
#[derive(Debug, Clone)]
pub struct Mutation {
    pub path: PathBuf,
    pub mutation_type: MutationType,
    #[allow(dead_code)]
    pub content_hash_before: Option<String>,
    #[allow(dead_code)]
    pub content_hash_after: Option<String>,
}

impl Mutation {
    #[allow(dead_code)]
    pub fn write(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mutation_type: MutationType::Write,
            content_hash_before: None,
            content_hash_after: None,
        }
    }

    #[allow(dead_code)]
    pub fn delete(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mutation_type: MutationType::Delete,
            content_hash_before: None,
            content_hash_after: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationType {
    Write,
    Patch,
    #[allow(dead_code)]
    Delete,
    #[allow(dead_code)]
    Move,
}

// ============================================================================
// VALIDATION TYPES
// ============================================================================

/// Decision from validation engine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationDecision {
    Accept,
    Reject,
    #[allow(dead_code)]
    Escalate,
}

/// Detailed validation result
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub decision: ValidationDecision,
    pub stage_results: Vec<ValidationStageResult>,
    pub message: String,
    #[allow(dead_code)]
    pub requires_revert: bool,
}

impl ValidationReport {
    #[allow(dead_code)]
    pub fn accept(message: &str) -> Self {
        Self {
            decision: ValidationDecision::Accept,
            stage_results: vec![],
            message: message.to_string(),
            requires_revert: false,
        }
    }

    #[allow(dead_code)]
    pub fn reject(message: &str) -> Self {
        Self {
            decision: ValidationDecision::Reject,
            stage_results: vec![],
            message: message.to_string(),
            requires_revert: true,
        }
    }
}

/// Result from one validation stage
#[derive(Debug, Clone)]
pub struct ValidationStageResult {
    #[allow(dead_code)]
    pub stage: ValidationStage,
    #[allow(dead_code)]
    pub passed: bool,
    #[allow(dead_code)]
    pub message: String,
    #[allow(dead_code)]
    pub execution_time_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStage {
    Syntax,
    Format,
    #[allow(dead_code)]
    Lint,
    #[allow(dead_code)]
    Build,
    #[allow(dead_code)]
    Test,
}

// ============================================================================
// STATE TYPES
// ============================================================================

/// Session status lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Initializing,
    Running,
    Complete,
    Error,
    Halted,
}

/// A recorded change in the change history
#[derive(Debug, Clone)]
pub struct ChangeRecord {
    #[allow(dead_code)]
    pub session_id: SessionId,
    pub iteration: u32,
    #[allow(dead_code)]
    pub timestamp: u64,
    pub mutation: Mutation,
    pub validation_report: ValidationReport,
}

// ============================================================================
// ERROR TYPES
// ============================================================================

/// Top-level Forge error type
#[derive(Debug, Clone)]
pub enum ForgeError {
    // Configuration errors
    InvalidConfiguration(String),

    // Tool errors
    InvalidToolName(String),
    UnknownTool(ToolName),
    ToolNotAllowed {
        tool: ToolName,
        mode: ExecutionMode,
    },
    MissingArgument(String),
    InvalidArgument(String),

    // State errors
    StateCorruption(String),
    InvalidStateTransition {
        from: SessionStatus,
        to: SessionStatus,
    },

    // Validation errors
    ValidationFailed(String),

    // PHASE 3: Planner-specific errors
    /// Planner backend unavailable or unreachable
    PlannerBackendUnavailable(String),
    /// Planner inference timeout
    #[allow(dead_code)]
    PlannerTimeout {
        timeout_seconds: u64,
    },
    /// Planner returned empty response
    #[allow(dead_code)]
    PlannerResponseEmpty,
    /// Failed to normalize planner output
    PlannerNormalizationError(String),
    /// Planner output violates schema
    #[allow(dead_code)]
    PlannerSchemaViolation(String),
    /// Planner attempted multiple actions (contract violation)
    #[allow(dead_code)]
    PlannerMultipleActions,
    /// Planner tried to call unknown tool
    #[allow(dead_code)]
    PlannerUnknownTool {
        tool_name: String,
    },
    /// Planner violated mode restrictions
    #[allow(dead_code)]
    PlannerModeViolation {
        tool: String,
        mode: ExecutionMode,
    },
    /// General planner contract violation
    #[allow(dead_code)]
    PlannerContractViolation(String),

    // System errors
    IoError(String),
    #[allow(dead_code)]
    InternalError(String),

    // PHASE 5: Command execution errors
    /// Command execution timeout
    ExecutionTimeout {
        command: String,
        timeout_secs: u64,
    },
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgeError::InvalidConfiguration(msg) => write!(f, "Config error: {}", msg),
            ForgeError::InvalidToolName(reason) => write!(f, "Invalid tool name: {}", reason),
            ForgeError::UnknownTool(name) => write!(f, "Unknown tool: {}", name),
            ForgeError::ToolNotAllowed { tool, mode } => {
                write!(f, "Tool '{}' not allowed in {:?} mode", tool, mode)
            }
            ForgeError::MissingArgument(name) => write!(f, "Missing argument: {}", name),
            ForgeError::InvalidArgument(msg) => write!(f, "Invalid argument: {}", msg),
            ForgeError::StateCorruption(msg) => write!(f, "State corruption: {}", msg),
            ForgeError::InvalidStateTransition { from, to } => {
                write!(f, "Invalid state transition: {:?} -> {:?}", from, to)
            }
            ForgeError::ValidationFailed(msg) => write!(f, "Validation failed: {}", msg),
            // PHASE 3: Planner errors
            ForgeError::PlannerBackendUnavailable(msg) => {
                write!(f, "Planner backend unavailable: {}", msg)
            }
            ForgeError::PlannerTimeout { timeout_seconds } => {
                write!(f, "Planner timeout after {} seconds", timeout_seconds)
            }
            ForgeError::PlannerResponseEmpty => write!(f, "Planner returned empty response"),
            ForgeError::PlannerNormalizationError(msg) => {
                write!(f, "Planner output normalization failed: {}", msg)
            }
            ForgeError::PlannerSchemaViolation(msg) => {
                write!(f, "Planner output schema violation: {}", msg)
            }
            ForgeError::PlannerMultipleActions => {
                write!(f, "Planner attempted multiple actions (contract violation)")
            }
            ForgeError::PlannerUnknownTool { tool_name } => {
                write!(f, "Planner called unknown tool: {}", tool_name)
            }
            ForgeError::PlannerModeViolation { tool, mode } => {
                write!(f, "Planner tool '{}' violated mode {:?}", tool, mode)
            }
            ForgeError::PlannerContractViolation(msg) => {
                write!(f, "Planner contract violation: {}", msg)
            }
            ForgeError::IoError(msg) => write!(f, "IO error: {}", msg),
            ForgeError::InternalError(msg) => write!(f, "Internal error: {}", msg),
            ForgeError::ExecutionTimeout {
                command,
                timeout_secs,
            } => {
                write!(
                    f,
                    "Command '{}' timed out after {} seconds",
                    command, timeout_secs
                )
            }
        }
    }
}

impl std::error::Error for ForgeError {}

// ============================================================================
// INTERNAL MODULES
// ============================================================================

mod counter {
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    pub fn next() -> u32 {
        COUNTER.fetch_add(1, Ordering::SeqCst)
    }
}

// ============================================================================
// TIME UTILITIES
// ============================================================================

pub fn timestamp_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

// ============================================================================
// PHASE 2: FILE TRACKING TYPES
// ============================================================================

/// Record of a file read operation
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub path: PathBuf,
    pub content_hash: String,
    pub size_bytes: u64,
    pub total_lines: usize,
    pub lines_read: Option<(usize, usize)>, // (offset, limit) if partial read
    pub is_full_read: bool,
    pub read_at_iteration: u32,
    #[allow(dead_code)]
    pub content: Option<String>, // Full content or the exact observed excerpt from read_file
}

impl FileRecord {
    pub fn new(
        path: impl Into<PathBuf>,
        content: &str,
        lines_read: Option<(usize, usize)>,
        iteration: u32,
    ) -> Self {
        let path = path.into();
        let size_bytes = content.len() as u64;
        let total_lines = content.lines().count();
        let is_full_read = lines_read.is_none();
        // PHASE 4: Use stable SHA-256 instead of DefaultHasher
        let content_hash = crate::crypto_hash::compute_content_hash(content);

        Self {
            path,
            content_hash,
            size_bytes,
            total_lines,
            lines_read,
            is_full_read,
            read_at_iteration: iteration,
            content: if is_full_read {
                Some(content.to_string())
            } else {
                None
            },
        }
    }

    /// Check if this record can be used for patching
    #[allow(dead_code)]
    pub fn can_patch(&self) -> bool {
        // Must be a full read to patch
        self.is_full_read
    }

    pub fn with_observed_content(mut self, observed_content: impl Into<String>) -> Self {
        self.content = Some(observed_content.into());
        self
    }

    /// Get stored content if available
    #[allow(dead_code)]
    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }
}

/// Result type for read_file tool
#[derive(Debug, Clone)]
pub struct ReadFileResult {
    #[allow(dead_code)]
    pub path: PathBuf,
    #[allow(dead_code)]
    pub content: String,
    pub total_lines: usize,
    pub lines_returned: usize,
    #[allow(dead_code)]
    pub content_hash: String,
    #[allow(dead_code)]
    pub is_full_read: bool,
}

/// Result type for apply_patch tool
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ApplyPatchResult {
    pub path: PathBuf,
    pub success: bool,
    pub lines_changed: usize,
    pub old_hash: String,
    pub new_hash: String,
    pub error: Option<String>,
}

/// Snapshot for revert operations
#[derive(Debug, Clone)]
pub struct FileSnapshot {
    #[allow(dead_code)]
    pub path: PathBuf,
    #[allow(dead_code)]
    pub content: String,
    #[allow(dead_code)]
    pub content_hash: String,
    #[allow(dead_code)]
    pub captured_at: u64,
}

impl FileSnapshot {
    pub fn new(path: &Path, content: &str) -> Self {
        Self {
            path: path.to_path_buf(),
            content: content.to_string(),
            content_hash: compute_content_hash(content),
            captured_at: timestamp_now(),
        }
    }
}

/// Patch model - exact string replacement
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Patch {
    pub old_text: String,
    pub new_text: String,
}

impl Patch {
    #[allow(dead_code)]
    pub fn apply(&self, content: &str) -> Option<String> {
        if content.contains(&self.old_text) {
            Some(content.replace(&self.old_text, &self.new_text))
        } else {
            None
        }
    }
}

/// Compute SHA-256 hash of content
/// PHASE 4: Now delegates to crypto_hash module for stable, portable hashing
fn compute_content_hash(content: &str) -> String {
    crate::crypto_hash::compute_content_hash(content)
}

// ============================================================================
// PHASE 2.5: HARDENED PATCH CONTRACT
// ============================================================================

/// Hardened patch with cardinality enforcement
/// old_text must appear exactly ONCE in target file
#[derive(Debug, Clone)]
pub struct HardenedPatch {
    pub old_text: String,
    pub new_text: String,
}

/// Result of patch application attempt
#[derive(Debug, Clone)]
pub enum PatchApplicationResult {
    Success {
        new_content: String,
        occurrences_found: usize,
        #[allow(dead_code)]
        occurrences_replaced: usize,
    },
    Failed {
        reason: PatchFailureReason,
        occurrences_found: usize,
    },
}

/// Specific reasons for patch failure (machine-distinguishable)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchFailureReason {
    TextNotFound,        // old_text appeared 0 times
    MultipleOccurrences, // old_text appeared >1 times (ambiguous)
    #[allow(dead_code)]
    TargetNotFound, // file doesn't exist
    #[allow(dead_code)]
    IoError(String),
}

impl fmt::Display for PatchFailureReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchFailureReason::TextNotFound => {
                write!(f, "old_text not found in target file")
            }
            PatchFailureReason::MultipleOccurrences => {
                write!(
                    f,
                    "old_text appears multiple times - ambiguous patch target"
                )
            }
            PatchFailureReason::TargetNotFound => {
                write!(f, "target file not found")
            }
            PatchFailureReason::IoError(msg) => {
                write!(f, "IO error during patch: {}", msg)
            }
        }
    }
}

impl HardenedPatch {
    /// Apply patch with cardinality enforcement (exactly one occurrence)
    pub fn apply(&self, content: &str) -> PatchApplicationResult {
        let occurrences = content.matches(&self.old_text).count();

        if occurrences == 0 {
            return PatchApplicationResult::Failed {
                reason: PatchFailureReason::TextNotFound,
                occurrences_found: 0,
            };
        }

        if occurrences > 1 {
            return PatchApplicationResult::Failed {
                reason: PatchFailureReason::MultipleOccurrences,
                occurrences_found: occurrences,
            };
        }

        // Exactly one occurrence - safe to replace
        let new_content = content.replacen(&self.old_text, &self.new_text, 1);

        PatchApplicationResult::Success {
            new_content,
            occurrences_found: 1,
            occurrences_replaced: 1,
        }
    }
}

/// Hardened ApplyPatchResult with full integrity metadata
#[derive(Debug, Clone)]
pub struct HardenedPatchResult {
    #[allow(dead_code)]
    pub path: PathBuf,
    #[allow(dead_code)]
    pub success: bool,
    #[allow(dead_code)]
    pub hash_before: String,
    #[allow(dead_code)]
    pub hash_after: Option<String>,
    pub old_text_occurrences: usize,
    pub lines_changed: usize,
    #[allow(dead_code)]
    pub snapshot_used: bool,
    #[allow(dead_code)]
    pub expected_hash_verified: bool,
    #[allow(dead_code)]
    pub error: Option<PatchFailureReason>,
}

// ============================================================================
// PHASE 2.5: EXPANDED ERROR TAXONOMY
// ============================================================================

/// Patch-specific errors with domain specificity
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PatchIntegrityError {
    ReadBeforeWriteViolation {
        path: PathBuf,
    },
    ExpectedHashMissing,
    ExpectedHashMismatch {
        path: PathBuf,
        expected: String,
        tracked: String,
    },
    TrackedHashMismatch {
        path: PathBuf,
        tracked: String,
        on_disk: String,
    },
    PatchCardinalityViolation {
        path: PathBuf,
        occurrences: usize,
    },
    PatchTextNotFound {
        path: PathBuf,
    },
    SnapshotMissing {
        path: PathBuf,
    },
    RevertVerificationFailed {
        path: PathBuf,
        expected_hash: String,
        actual_hash: String,
    },
}

impl fmt::Display for PatchIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchIntegrityError::ReadBeforeWriteViolation { path } => {
                write!(
                    f,
                    "Read-before-write violation: {} must be read before patch",
                    path.display()
                )
            }
            PatchIntegrityError::ExpectedHashMissing => {
                write!(f, "expected_hash is mandatory for apply_patch")
            }
            PatchIntegrityError::ExpectedHashMismatch {
                path,
                expected,
                tracked,
            } => {
                write!(
                    f,
                    "Expected hash mismatch for {}: expected {} but tracked hash is {}",
                    path.display(),
                    expected,
                    tracked
                )
            }
            PatchIntegrityError::TrackedHashMismatch {
                path,
                tracked,
                on_disk,
            } => {
                write!(
                    f,
                    "Tracked/on-disk hash mismatch for {}: tracked {} but on-disk is {}",
                    path.display(),
                    tracked,
                    on_disk
                )
            }
            PatchIntegrityError::PatchCardinalityViolation { path, occurrences } => {
                write!(
                    f,
                    "Patch cardinality violation for {}: old_text appears {} times (must be exactly 1)",
                    path.display(),
                    occurrences
                )
            }
            PatchIntegrityError::PatchTextNotFound { path } => {
                write!(f, "Patch text not found in {}", path.display())
            }
            PatchIntegrityError::SnapshotMissing { path } => {
                write!(f, "No snapshot available for revert of {}", path.display())
            }
            PatchIntegrityError::RevertVerificationFailed {
                path,
                expected_hash,
                actual_hash,
            } => {
                write!(
                    f,
                    "Revert verification failed for {}: expected hash {} but got {}",
                    path.display(),
                    expected_hash,
                    actual_hash
                )
            }
        }
    }
}

// ============================================================================
// PHASE 2: READ-BEFORE-WRITE ENFORCEMENT ERRORS (DEPRECATED - use PatchIntegrityError)
// ============================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PatchError {
    FileNotRead {
        path: PathBuf,
    },
    FileNotFullyRead {
        path: PathBuf,
    },
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    PatchFailed {
        path: PathBuf,
        reason: String,
    },
    FileNotFound {
        path: PathBuf,
    },
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::FileNotRead { path } => {
                write!(f, "File not read before patch: {}", path.display())
            }
            PatchError::FileNotFullyRead { path } => {
                write!(
                    f,
                    "File must be fully read before patch: {}",
                    path.display()
                )
            }
            PatchError::HashMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Hash mismatch for {}: expected {}, actual {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            PatchError::PatchFailed { path, reason } => {
                write!(f, "Patch failed for {}: {}", path.display(), reason)
            }
            PatchError::FileNotFound { path } => {
                write!(f, "File not found: {}", path.display())
            }
        }
    }
}

// ============================================================================
// PHASE 2.5: JSONL LOGGING
// ============================================================================

/// Structured JSONL log entry for machine-readable logging
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonlLogEntry {
    pub event_type: String,
    pub session_id: String,
    pub iteration: u32,
    pub timestamp: u64,
    pub severity: String,
    pub tool: Option<String>,
    pub result_code: Option<String>,
    pub affected_paths: Vec<String>,
    pub message: String,
    pub metadata: HashMap<String, String>,
}

impl JsonlLogEntry {
    pub fn new(
        event_type: &str,
        session_id: &SessionId,
        iteration: u32,
        severity: &str,
        message: &str,
    ) -> Self {
        Self {
            event_type: event_type.to_string(),
            session_id: session_id.to_string(),
            iteration,
            timestamp: timestamp_now(),
            severity: severity.to_string(),
            tool: None,
            result_code: None,
            affected_paths: vec![],
            message: message.to_string(),
            metadata: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn with_tool(mut self, tool: &str) -> Self {
        self.tool = Some(tool.to_string());
        self
    }

    #[allow(dead_code)]
    pub fn with_result_code(mut self, code: &str) -> Self {
        self.result_code = Some(code.to_string());
        self
    }

    #[allow(dead_code)]
    pub fn with_path(mut self, path: &Path) -> Self {
        self.affected_paths.push(path.display().to_string());
        self
    }

    #[allow(dead_code)]
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    pub fn to_jsonl(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                "{{\"error\":\"serialization_failed\",\"timestamp\":{}}}",
                timestamp_now()
            )
        })
    }
}

/// Log severity levels for JSONL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSeverity {
    Debug,
    Info,
    Warning,
    Error,
}

impl fmt::Display for LogSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogSeverity::Debug => write!(f, "DEBUG"),
            LogSeverity::Info => write!(f, "INFO"),
            LogSeverity::Warning => write!(f, "WARNING"),
            LogSeverity::Error => write!(f, "ERROR"),
        }
    }
}

// ============================================================================
// TASK CHAIN TYPES (Phase 1: Bounded Multi-Step Execution)
// ============================================================================

/// Unique chain identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct ChainId(String);

#[allow(dead_code)]
impl ChainId {
    pub fn new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        Self(format!("chain-{}-{:08x}", now, counter::next()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ChainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique chain step identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct ChainStepId(String);

#[allow(dead_code)]
impl ChainStepId {
    pub fn new(index: usize) -> Self {
        Self(format!("step-{}", index))
    }

    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ChainStepId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// AUTHORITATIVE EXECUTION OUTCOME
/// Single source of truth for chain execution result.
/// UI renders ONLY from this outcome - no parallel truth sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    /// All steps completed successfully, no warnings
    Success,
    /// Completed but with non-fatal warnings (e.g., force override used)
    SuccessWithWarnings,
    /// Could not complete due to blocking condition (risks, approval needed)
    Blocked,
    /// Execution failed with errors
    Failed,
}

impl ExecutionOutcome {
    /// Check if outcome represents successful completion
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            ExecutionOutcome::Success | ExecutionOutcome::SuccessWithWarnings
        )
    }

    /// Check if outcome represents a failure state
    pub fn is_failure(&self) -> bool {
        matches!(self, ExecutionOutcome::Failed)
    }

    /// Check if outcome represents blocked state
    pub fn is_blocked(&self) -> bool {
        matches!(self, ExecutionOutcome::Blocked)
    }

    /// Get display label for UI
    pub fn label(&self) -> &'static str {
        match self {
            ExecutionOutcome::Success => "SUCCESS",
            ExecutionOutcome::SuccessWithWarnings => "DONE (with warnings)",
            ExecutionOutcome::Blocked => "BLOCKED",
            ExecutionOutcome::Failed => "FAILED",
        }
    }

    /// Get indicator icon for UI
    pub fn icon(&self) -> &'static str {
        match self {
            ExecutionOutcome::Success => "✓",
            ExecutionOutcome::SuccessWithWarnings => "⚡",
            ExecutionOutcome::Blocked => "⏸",
            ExecutionOutcome::Failed => "✗",
        }
    }
}

/// Status of a task chain lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ChainStatus {
    /// Chain created but not yet started
    Pending,
    /// Currently executing a step
    Running,
    /// All steps completed successfully
    Complete,
    /// Chain halted due to step failure
    Failed,
    /// Chain stopped by user/system
    Cancelled,
}

/// Status of an individual chain step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum StepStatus {
    /// Step not yet started
    Pending,
    /// Step currently executing
    Running,
    /// Step completed, outcome validated
    Completed,
    /// Step failed, outcome rejected
    Failed,
    /// Step blocked, cannot proceed
    Blocked,
}

/// Explicit sprint contract name for step status.
#[allow(dead_code)]
pub type ChainStepStatus = StepStatus;

/// Classified outcome of a chain step
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum StepOutcome {
    /// Step resolved successfully with evidence
    Resolved {
        summary: String,
        files_modified: Vec<PathBuf>,
    },
    /// Step failed with error context
    Failed { reason: String, recoverable: bool },
    /// Step blocked pending external action
    Blocked { reason: String },
}

/// A single step in a task chain
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChainStep {
    /// Stable typed step identifier
    pub id: ChainStepId,
    /// Unique step identifier within chain
    pub step_id: String,
    /// Human-readable description
    pub description: String,
    /// Tool calls assigned to this step by a structured plan
    pub tool_calls: Vec<ToolCall>,
    /// Current execution status
    pub status: StepStatus,
    /// Validation result for this step, if it mutated state
    pub validation_result: Option<ValidationReport>,
    /// Outcome once step completes
    pub outcome: Option<StepOutcome>,
    /// Index for ordering
    pub index: usize,
}

#[allow(dead_code)]
impl ChainStep {
    pub fn new(index: usize, description: impl Into<String>) -> Self {
        let id = ChainStepId::new(index);
        Self {
            step_id: id.to_string(),
            id,
            description: description.into(),
            tool_calls: Vec::new(),
            status: StepStatus::Pending,
            validation_result: None,
            outcome: None,
            index,
        }
    }

    pub fn with_tool_calls(
        index: usize,
        description: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        let mut step = Self::new(index, description);
        step.tool_calls = tool_calls;
        step
    }
}

/// Result of chain execution
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChainExecutionResult {
    pub success: bool,
    pub chain_id: String,
    pub completed_steps: usize,
    pub total_steps: usize,
    pub final_status: ChainStatus,
    pub error: Option<String>,
}

/// A bounded multi-step task chain
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TaskChain {
    /// Unique chain identifier
    pub chain_id: ChainId,
    /// Overall objective description
    pub objective: String,
    /// Ordered sequence of steps
    pub steps: Vec<ChainStep>,
    /// Index of current step (0-based)
    pub current_step: usize,
    /// Overall chain status
    pub status: ChainStatus,
    /// AUTHORITATIVE EXECUTION OUTCOME
    /// Set once at end of execution lifecycle. UI renders ONLY from this.
    pub execution_outcome: Option<ExecutionOutcome>,
    /// Track if force override was used (maps to SuccessWithWarnings)
    pub force_override_used: bool,
    /// Creation timestamp
    pub created_at: u64,
    /// Last update timestamp
    pub updated_at: u64,
}

/// Explicit sprint contract name for a bounded multi-step task chain.
#[allow(dead_code)]
pub type Chain = TaskChain;

#[allow(dead_code)]
impl TaskChain {
    /// Create a new chain with objective and steps
    pub fn new(objective: impl Into<String>, steps: Vec<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let chain_steps: Vec<ChainStep> = steps
            .into_iter()
            .enumerate()
            .map(|(idx, desc)| ChainStep::new(idx, desc))
            .collect();

        Self {
            chain_id: ChainId::new(),
            objective: objective.into(),
            steps: chain_steps,
            current_step: 0,
            status: ChainStatus::Pending,
            execution_outcome: None,
            force_override_used: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Get the current step if any
    pub fn current_step(&self) -> Option<&ChainStep> {
        self.steps.get(self.current_step)
    }

    /// Get mutable reference to current step
    pub fn current_step_mut(&mut self) -> Option<&mut ChainStep> {
        self.steps.get_mut(self.current_step)
    }

    /// Advance to next step after successful completion
    pub fn advance(&mut self) -> bool {
        if self.current_step + 1 < self.steps.len() {
            self.current_step += 1;
            self.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            true
        } else {
            self.status = ChainStatus::Complete;
            false
        }
    }

    /// Mark chain as failed
    pub fn fail(&mut self) {
        self.status = ChainStatus::Failed;
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// Check if chain is complete
    pub fn is_complete(&self) -> bool {
        matches!(self.status, ChainStatus::Complete)
    }

    /// Check if chain has failed
    pub fn is_failed(&self) -> bool {
        matches!(self.status, ChainStatus::Failed)
    }

    /// Get completed step count
    pub fn completed_steps(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s.status, StepStatus::Completed))
            .count()
    }

    /// AUTHORITATIVE OUTCOME COMPUTATION - SINGLE WRITE POINT
    /// Call this ONCE at end of execution lifecycle to set the outcome.
    /// All UI messaging must derive from this outcome.
    pub fn finalize_outcome(&mut self) -> ExecutionOutcome {
        let outcome = if self.is_failed() {
            ExecutionOutcome::Failed
        } else if self.has_blocked_step() {
            ExecutionOutcome::Blocked
        } else if self.is_complete() {
            // Chain completed - check if force override was used
            if self.force_override_used {
                ExecutionOutcome::SuccessWithWarnings
            } else {
                ExecutionOutcome::Success
            }
        } else {
            // Chain not finished but not failed/blocked - treat as blocked
            ExecutionOutcome::Blocked
        };

        self.execution_outcome = Some(outcome);
        outcome
    }

    /// Check if any step is blocked
    fn has_blocked_step(&self) -> bool {
        self.steps
            .iter()
            .any(|s| matches!(s.status, StepStatus::Blocked))
    }

    /// Get the authoritative outcome if set
    pub fn get_outcome(&self) -> Option<ExecutionOutcome> {
        self.execution_outcome
    }

    /// Mark that force override was used (affects outcome classification)
    pub fn mark_force_override(&mut self) {
        self.force_override_used = true;
    }
}

// ============================================================================
// PHASE A: STATE INTEGRITY + HASH BINDING (PROOF-LEVEL HARDENING)
// ============================================================================

/// Cryptographic digest of chain state at a specific step
/// Enables tamper-evident chain execution and replay verification
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChainStateDigest {
    /// Step index this digest represents
    pub step_index: u32,
    /// SHA3-512 hash of serialized state
    pub state_hash: String,
    /// Hash of previous step (None for step 0)
    pub previous_hash: Option<String>,
    /// Timestamp when digest was computed
    pub computed_at: u64,
}

impl ChainStateDigest {
    /// Create a new state digest linking to previous hash
    pub fn new(step_index: u32, state_hash: String, previous_hash: Option<String>) -> Self {
        Self {
            step_index,
            state_hash,
            previous_hash,
            computed_at: timestamp_now(),
        }
    }

    /// Verify chain continuity: this digest's previous_hash matches given hash
    pub fn verify_chain_continuity(
        &self,
        expected_previous: &str,
    ) -> Result<(), ChainIntegrityError> {
        match &self.previous_hash {
            Some(prev) if prev == expected_previous => Ok(()),
            Some(prev) => Err(ChainIntegrityError::ChainBroken {
                step_index: self.step_index,
                expected_hash: expected_previous.to_string(),
                actual_hash: prev.clone(),
            }),
            None if self.step_index == 0 => Ok(()), // Step 0 has no previous
            None => Err(ChainIntegrityError::MissingPreviousHash {
                step_index: self.step_index,
            }),
        }
    }
}

/// Replay seal for deterministic execution verification
/// Captures initial conditions and expected final state
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub struct ReplaySeal {
    /// Hash of initial input/state
    pub initial_input_hash: String,
    /// Hash of the complete execution chain
    pub chain_hash: String,
    /// Expected final state hash
    pub final_state_hash: String,
    /// Version of runtime that created seal
    pub runtime_version: String,
    /// Timestamp of seal creation
    pub created_at: u64,
}

#[allow(dead_code)]
impl ReplaySeal {
    /// Create a new replay seal
    pub fn new(initial_input_hash: String, chain_hash: String, final_state_hash: String) -> Self {
        Self {
            initial_input_hash,
            chain_hash,
            final_state_hash,
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: timestamp_now(),
        }
    }

    /// Verify replay matches seal expectations
    pub fn verify_replay(
        &self,
        recomputed_chain_hash: &str,
        final_state_hash: &str,
    ) -> Result<(), ReplayVerificationError> {
        if recomputed_chain_hash != self.chain_hash {
            return Err(ReplayVerificationError::ChainHashMismatch {
                expected: self.chain_hash.clone(),
                actual: recomputed_chain_hash.to_string(),
            });
        }
        if final_state_hash != self.final_state_hash {
            return Err(ReplayVerificationError::FinalStateMismatch {
                expected: self.final_state_hash.clone(),
                actual: final_state_hash.to_string(),
            });
        }
        Ok(())
    }
}

/// Chain integrity errors - cryptographic verification failures
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ChainIntegrityError {
    ChainBroken {
        step_index: u32,
        expected_hash: String,
        actual_hash: String,
    },
    MissingPreviousHash {
        step_index: u32,
    },
    StateTampered {
        step_index: u32,
        expected_hash: String,
        actual_hash: String,
    },
    StepReordered {
        step_index: u32,
        expected_step: u32,
    },
}

impl fmt::Display for ChainIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChainBroken {
                step_index,
                expected_hash,
                actual_hash,
            } => {
                write!(
                    f,
                    "Chain broken at step {}: expected hash {} but got {}",
                    step_index, expected_hash, actual_hash
                )
            }
            Self::MissingPreviousHash { step_index } => {
                write!(
                    f,
                    "Missing previous hash at step {} (non-zero step must link)",
                    step_index
                )
            }
            Self::StateTampered {
                step_index,
                expected_hash,
                actual_hash,
            } => {
                write!(
                    f,
                    "State tampered at step {}: expected {} but got {}",
                    step_index, expected_hash, actual_hash
                )
            }
            Self::StepReordered {
                step_index,
                expected_step,
            } => {
                write!(
                    f,
                    "Step reordering detected: got step {} but expected {}",
                    step_index, expected_step
                )
            }
        }
    }
}

impl std::error::Error for ChainIntegrityError {}

/// Replay verification errors
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ReplayVerificationError {
    ChainHashMismatch { expected: String, actual: String },
    FinalStateMismatch { expected: String, actual: String },
    InitialInputMismatch { expected: String, actual: String },
    RuntimeVersionMismatch { expected: String, actual: String },
}

impl fmt::Display for ReplayVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChainHashMismatch { expected, actual } => {
                write!(
                    f,
                    "Chain hash mismatch: expected {} but got {}",
                    expected, actual
                )
            }
            Self::FinalStateMismatch { expected, actual } => {
                write!(
                    f,
                    "Final state mismatch: expected {} but got {}",
                    expected, actual
                )
            }
            Self::InitialInputMismatch { expected, actual } => {
                write!(
                    f,
                    "Initial input mismatch: expected {} but got {}",
                    expected, actual
                )
            }
            Self::RuntimeVersionMismatch { expected, actual } => {
                write!(
                    f,
                    "Runtime version mismatch: seal created with {} but replaying with {}",
                    expected, actual
                )
            }
        }
    }
}

impl std::error::Error for ReplayVerificationError {}

/// Determinism violation detection
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum DeterminismViolation {
    TimeUsageDetected { source: String },
    RandomnessDetected { source: String },
    ExternalIoDetected { operation: String },
    StateMutationAfterCheckpoint { step_index: u32 },
}

impl fmt::Display for DeterminismViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeUsageDetected { source } => {
                write!(f, "Non-deterministic time usage detected in: {}", source)
            }
            Self::RandomnessDetected { source } => {
                write!(f, "Non-deterministic randomness detected in: {}", source)
            }
            Self::ExternalIoDetected { operation } => {
                write!(f, "Non-deterministic external IO detected: {}", operation)
            }
            Self::StateMutationAfterCheckpoint { step_index } => {
                write!(f, "State mutated after checkpoint at step {}", step_index)
            }
        }
    }
}

impl std::error::Error for DeterminismViolation {}
