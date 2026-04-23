//! FORGE PHASE 1.5: Hardened State Model
//!
//! Implements AgentState with strong typing and integrity guarantees.
//!
//! Key improvements:
//! - SessionId newtype prevents session confusion
//! - State transitions validated
//! - Hash chain for integrity verification
//! - Immutable change history (append-only)

use crate::types::{
    ChangeRecord, CompletionReason, FileRecord, FileSnapshot, ForgeError, Mutation, MutationType,
    SessionId, SessionStatus, ValidationDecision, ValidationReport, ValidationStage,
    ValidationStageResult,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// ===========================================================================
/// AGENT STATE - Complete session state
/// ===========================================================================
/// Core agent state structure
#[derive(Debug, Clone)]
pub struct AgentState {
    pub session_id: SessionId,
    pub iteration: u32,
    pub max_iterations: u32,
    pub task: String,
    pub mode: crate::types::ExecutionMode,
    pub status: SessionStatus,
    pub files_written: HashSet<PathBuf>,
    pub files_read: HashMap<PathBuf, FileRecord>,
    pub change_history: Vec<ChangeRecord>,
    pub completion_reason: Option<CompletionReason>,
    /// Most recent validation report, including read-only or step-level validation.
    pub last_validation_report: Option<ValidationReport>,
    pub state_hash: String,
    pub previous_hash: Option<String>,
    /// Snapshots for revert operations
    pub snapshots: HashMap<PathBuf, FileSnapshot>,
    /// Pending validation items (files awaiting validation)
    pub pending_validations: Vec<String>,
    /// Cardinality violations count (patch failures due to multiple matches)
    pub cardinality_violations: u32,
    /// Whether state hash mismatch was detected during deserialization
    pub has_hash_mismatch: bool,
}

impl AgentState {
    /// Create initial state for a new session
    pub fn new(max_iterations: u32, task: String, mode: crate::types::ExecutionMode) -> Self {
        let mut state = Self {
            session_id: SessionId::new(),
            iteration: 0,
            max_iterations,
            task,
            mode,
            status: SessionStatus::Initializing,
            files_written: HashSet::new(),
            files_read: HashMap::new(),
            change_history: Vec::new(),
            completion_reason: None,
            last_validation_report: None,
            state_hash: String::new(),
            previous_hash: None,
            snapshots: HashMap::new(),
            pending_validations: Vec::new(),
            cardinality_violations: 0,
            has_hash_mismatch: false,
        };
        state.state_hash = compute_hash(&state);
        state
    }

    /// Transition to a new status with validation
    pub fn transition(&mut self, new_status: SessionStatus) -> Result<(), ForgeError> {
        // Validate transition is allowed
        if !is_valid_transition(self.status, new_status) {
            return Err(ForgeError::InvalidStateTransition {
                from: self.status,
                to: new_status,
            });
        }

        self.status = new_status;
        self.update_hash_chain();
        Ok(())
    }

    /// Start the session (Initializing -> Running)
    pub fn start(&mut self) -> Result<(), ForgeError> {
        self.transition(SessionStatus::Running)
    }

    /// Complete the session
    pub fn complete(&mut self, reason: CompletionReason) -> Result<(), ForgeError> {
        self.completion_reason = Some(reason);
        self.transition(SessionStatus::Complete)
    }

    /// Mark session as error
    pub fn error(&mut self) -> Result<(), ForgeError> {
        self.transition(SessionStatus::Error)
    }

    /// Halt the session (fatal error)
    pub fn halt(&mut self) -> Result<(), ForgeError> {
        self.transition(SessionStatus::Halted)
    }

    /// Increment iteration counter
    pub fn next_iteration(&mut self) -> Result<(), ForgeError> {
        if self.iteration >= self.max_iterations {
            return Err(ForgeError::InvalidConfiguration(format!(
                "Max iterations ({}) exceeded",
                self.max_iterations
            )));
        }
        self.iteration += 1;
        self.update_hash_chain();
        Ok(())
    }

    /// Commit mutations to state after successful validation
    pub fn commit(
        &mut self,
        report: &ValidationReport,
        mutations: &[Mutation],
    ) -> Result<(), ForgeError> {
        if report.decision != ValidationDecision::Accept {
            return Err(ForgeError::ValidationFailed(
                "Cannot commit rejected mutations".to_string(),
            ));
        }

        let timestamp = crate::types::timestamp_now();
        self.last_validation_report = Some(report.clone());
        for mutation in mutations {
            if matches!(
                mutation.mutation_type,
                MutationType::Write | MutationType::Patch
            ) {
                self.files_written.insert(mutation.path.clone());
            }

            self.change_history.push(ChangeRecord {
                session_id: self.session_id.clone(),
                iteration: self.iteration,
                timestamp,
                mutation: mutation.clone(),
                validation_report: report.clone(),
            });
        }

        self.update_hash_chain();
        Ok(())
    }

    /// Add a file to the written set
    #[allow(dead_code)]
    pub fn record_file_written(&mut self, path: PathBuf) {
        self.files_written.insert(path);
        self.update_hash_chain();
    }

    /// Record a file read operation
    pub fn record_file_read(&mut self, record: FileRecord) {
        self.files_read.insert(record.path.clone(), record);
        self.update_hash_chain();
    }

    /// Get a file record if it was previously read
    #[allow(dead_code)]
    pub fn get_file_record(&self, path: &Path) -> Option<&FileRecord> {
        self.files_read.get(path)
    }

    /// Check if a file was fully read (required for patching)
    #[allow(dead_code)]
    pub fn is_file_fully_read(&self, path: &std::path::Path) -> bool {
        self.files_read
            .get(path)
            .map(|r| r.is_full_read)
            .unwrap_or(false)
    }

    /// Capture a snapshot for revert
    pub fn capture_snapshot(&mut self, path: &Path, content: &str) {
        let snapshot = FileSnapshot::new(path, content);
        self.snapshots.insert(path.to_path_buf(), snapshot);
        self.update_hash_chain();
    }

    /// Get a snapshot for revert
    #[allow(dead_code)]
    pub fn get_snapshot(&self, path: &Path) -> Option<&FileSnapshot> {
        self.snapshots.get(path)
    }

    /// Remove a snapshot after successful commit
    #[allow(dead_code)]
    pub fn clear_snapshot(&mut self, path: &Path) {
        self.snapshots.remove(path);
        self.update_hash_chain();
    }

    /// Verify state integrity
    pub fn verify_integrity(&self) -> Result<(), ForgeError> {
        let expected_hash = compute_hash(self);
        if self.state_hash != expected_hash {
            return Err(ForgeError::StateCorruption(
                "State hash mismatch - possible tampering".to_string(),
            ));
        }
        Ok(())
    }

    fn update_hash_chain(&mut self) {
        self.previous_hash = Some(self.state_hash.clone());
        self.state_hash = compute_hash(self);
    }
}

/// ===========================================================================
/// STATE TRANSITION VALIDATION
/// ===========================================================================
///
fn is_valid_transition(from: SessionStatus, to: SessionStatus) -> bool {
    use SessionStatus::*;

    match (from, to) {
        // Initializing can go to Running or Error
        (Initializing, Running) => true,
        (Initializing, Error) => true,
        (Initializing, Halted) => true,

        // Running can go to Complete, Error, or Halted
        (Running, Complete) => true,
        (Running, Error) => true,
        (Running, Halted) => true,

        // Terminal states - no transitions out
        (Complete, _) => false,
        (Error, _) => false,
        (Halted, _) => false,

        // All other transitions invalid
        _ => false,
    }
}

/// ===========================================================================
/// HASH CHAIN
/// ===========================================================================
///
fn compute_hash(state: &AgentState) -> String {
    let input = serde_json::to_string(&state_hash_material(state))
        .expect("agent state hash material must serialize");
    crate::crypto_hash::compute_content_hash(&input)
}

fn state_hash_material(state: &AgentState) -> Value {
    let mut files_written: Vec<String> = state
        .files_written
        .iter()
        .map(|path| path_to_string(path.as_path()))
        .collect();
    files_written.sort();

    let mut files_read: Vec<&FileRecord> = state.files_read.values().collect();
    files_read.sort_by_key(|record| path_to_string(&record.path));

    let mut snapshots: Vec<&FileSnapshot> = state.snapshots.values().collect();
    snapshots.sort_by_key(|snapshot| path_to_string(&snapshot.path));

    json!({
        "session_id": state.session_id.to_string(),
        "iteration": state.iteration,
        "max_iterations": state.max_iterations,
        "task": state.task,
        "mode": execution_mode_name(state.mode),
        "status": session_status_name(state.status),
        "files_written": files_written,
        "files_read": files_read.into_iter().map(file_record_to_value).collect::<Vec<_>>(),
        "change_history": state.change_history.iter().map(change_record_to_value).collect::<Vec<_>>(),
        "completion_reason": state.completion_reason.as_ref().map(|reason| reason.as_str().to_string()),
        "last_validation_report": state.last_validation_report.as_ref().map(validation_report_to_value),
        "previous_hash": state.previous_hash.clone(),
        "snapshots": snapshots.into_iter().map(file_snapshot_to_value).collect::<Vec<_>>(),
        "pending_validations": state.pending_validations,
        "cardinality_violations": state.cardinality_violations,
    })
}

/// ===========================================================================
/// SESSION PERSISTENCE (Phase 3)
/// ===========================================================================
///
#[allow(dead_code)]
impl AgentState {
    /// Export state to JSON for persistence
    pub fn to_json(&self) -> String {
        let mut value = state_hash_material(self);
        let object = value
            .as_object_mut()
            .expect("state hash material must be an object");
        object.insert(
            "state_hash".to_string(),
            Value::String(self.state_hash.clone()),
        );
        serde_json::to_string_pretty(&value).expect("agent state must serialize")
    }

    /// Save state to file (JSON format)
    pub fn save(&self, path: &std::path::Path) -> Result<(), ForgeError> {
        let json = self.to_json();
        std::fs::write(path, json)
            .map_err(|e| ForgeError::IoError(format!("Failed to save state: {}", e)))?;
        Ok(())
    }

    /// Load state from file (JSON format)
    pub fn load(path: &std::path::Path) -> Result<Self, ForgeError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ForgeError::IoError(format!("Failed to load state: {}", e)))?;

        let parsed: Value = serde_json::from_str(&content)
            .map_err(|e| ForgeError::InvalidConfiguration(format!("Invalid state file: {}", e)))?;

        let object = parsed.as_object().ok_or_else(|| {
            ForgeError::InvalidConfiguration("State file must contain a JSON object".to_string())
        })?;

        let session_id = object
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ForgeError::InvalidConfiguration("Missing session_id".to_string()))?;

        let iteration = object
            .get("iteration")
            .and_then(Value::as_u64)
            .ok_or_else(|| ForgeError::InvalidConfiguration("Missing iteration".to_string()))?
            as u32;

        let max_iterations = object
            .get("max_iterations")
            .and_then(Value::as_u64)
            .unwrap_or(10) as u32;

        let status = parse_session_status(
            object
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("Initializing"),
        )?;

        let files_written = object
            .get("files_written")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();

        let files_read = object
            .get("files_read")
            .and_then(Value::as_array)
            .map(|items| parse_file_records(items))
            .transpose()?
            .unwrap_or_default();

        let change_history = object
            .get("change_history")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| parse_change_record(entry, session_id))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        let completion_reason = object
            .get("completion_reason")
            .and_then(Value::as_str)
            .map(CompletionReason::new);

        let last_validation_report = object
            .get("last_validation_report")
            .map(parse_validation_report)
            .transpose()?;

        let previous_hash = object
            .get("previous_hash")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        let snapshots = object
            .get("snapshots")
            .and_then(Value::as_array)
            .map(|items| parse_snapshots(items))
            .transpose()?
            .unwrap_or_default();

        let task = object
            .get("task")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_default();

        let mode = object
            .get("mode")
            .and_then(Value::as_str)
            .and_then(|s| match s {
                "analysis" => Some(crate::types::ExecutionMode::Analysis),
                "edit" => Some(crate::types::ExecutionMode::Edit),
                "fix" => Some(crate::types::ExecutionMode::Fix),
                "batch" => Some(crate::types::ExecutionMode::Batch),
                _ => None,
            })
            .unwrap_or(crate::types::ExecutionMode::Edit);

        let pending_validations = object
            .get("pending_validations")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let cardinality_violations = object
            .get("cardinality_violations")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(0);

        let mut state = Self {
            session_id: SessionId::from_string(session_id.to_string()),
            iteration,
            max_iterations,
            task,
            mode,
            status,
            files_written,
            files_read,
            change_history,
            completion_reason,
            last_validation_report,
            state_hash: String::new(),
            previous_hash,
            snapshots,
            pending_validations,
            cardinality_violations,
            has_hash_mismatch: false, // Will be set to true if mismatch detected below
        };

        let expected_hash = compute_hash(&state);
        let stored_hash = object
            .get("state_hash")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        state.state_hash = stored_hash.clone().unwrap_or_else(|| expected_hash.clone());

        // Detect hash mismatch for CSS metadata (before returning error)
        if stored_hash.is_some() && state.state_hash != expected_hash {
            state.has_hash_mismatch = true;
            return Err(ForgeError::StateCorruption(
                "Persisted state hash does not match reconstructed state".to_string(),
            ));
        }

        Ok(state)
    }

    /// Get data directory for persistence
    pub fn data_dir() -> std::path::PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("forge")
    }

    /// Get default state file path
    pub fn state_path() -> std::path::PathBuf {
        Self::data_dir().join("session.json")
    }

    /// Save to default location
    pub fn save_default(&self) -> Result<(), ForgeError> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ForgeError::IoError(format!("Failed to create data dir: {}", e)))?;
        }
        self.save(&path)
    }

    /// Load from default location
    pub fn load_default() -> Result<Self, ForgeError> {
        Self::load(&Self::state_path())
    }

    /// Record a successful step-level validation even when no mutation occurred.
    pub fn mark_validated(&mut self, report: ValidationReport) -> Result<(), ForgeError> {
        if report.decision != ValidationDecision::Accept {
            return Err(ForgeError::ValidationFailed(
                "Only accepted validation reports can mark state validated".to_string(),
            ));
        }

        self.last_validation_report = Some(report);
        self.update_hash_chain();
        Ok(())
    }

    /// Get default checkpoint directory.
    pub fn checkpoint_dir() -> std::path::PathBuf {
        Self::data_dir().join("checkpoints")
    }

    /// Get default checkpoint file path for a chain step.
    pub fn checkpoint_path(step_index: usize) -> std::path::PathBuf {
        Self::checkpoint_dir().join(format!("checkpoint_{}.json", step_index))
    }

    /// Check whether this state is acceptable as a resume checkpoint.
    pub fn is_validated_checkpoint(&self) -> Result<bool, ForgeError> {
        self.verify_integrity()?;
        Ok(self.pending_validations.is_empty()
            && self
                .last_validation_report
                .as_ref()
                .is_some_and(|report| report.decision == ValidationDecision::Accept))
    }

    /// Save a validated chain checkpoint.
    pub fn save_checkpoint(&self, step_index: usize) -> Result<std::path::PathBuf, ForgeError> {
        if !self.is_validated_checkpoint()? {
            return Err(ForgeError::ValidationFailed(
                "Cannot save checkpoint before accepted validation".to_string(),
            ));
        }

        let path = Self::checkpoint_path(step_index);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ForgeError::IoError(format!("Failed to create checkpoint dir: {}", e))
            })?;
        }
        self.save(&path)?;
        Ok(path)
    }

    /// Load a checkpoint only when resume was explicitly approved by the caller.
    pub fn load_checkpoint_for_resume(
        step_index: usize,
        approved: bool,
    ) -> Result<Self, ForgeError> {
        if !approved {
            return Err(ForgeError::InvalidConfiguration(
                "Resume requires explicit approval".to_string(),
            ));
        }

        let state = Self::load(&Self::checkpoint_path(step_index))?;
        if !state.is_validated_checkpoint()? {
            return Err(ForgeError::ValidationFailed(
                "Checkpoint is not a validated resume point".to_string(),
            ));
        }
        Ok(state)
    }

    /// Load default state only when resume was explicitly approved by the caller.
    pub fn load_default_for_resume(approved: bool) -> Result<Self, ForgeError> {
        if !approved {
            return Err(ForgeError::InvalidConfiguration(
                "Resume requires explicit approval".to_string(),
            ));
        }

        let state = Self::load_default()?;
        if !state.is_validated_checkpoint()? {
            return Err(ForgeError::ValidationFailed(
                "Default state is not a validated resume point".to_string(),
            ));
        }
        Ok(state)
    }

    /// Build the next chain-step state while preserving auditable history.
    pub fn continue_chain_step(
        &self,
        task: String,
        max_iterations: u32,
        mode: crate::types::ExecutionMode,
    ) -> Result<Self, ForgeError> {
        if !self.is_validated_checkpoint()? {
            return Err(ForgeError::ValidationFailed(
                "Cannot continue chain from an unvalidated state".to_string(),
            ));
        }

        let mut state = Self {
            session_id: self.session_id.clone(),
            iteration: 0,
            max_iterations,
            task,
            mode,
            status: SessionStatus::Running,
            files_written: self.files_written.clone(),
            files_read: self.files_read.clone(),
            change_history: self.change_history.clone(),
            completion_reason: None,
            last_validation_report: self.last_validation_report.clone(),
            state_hash: String::new(),
            previous_hash: Some(self.state_hash.clone()),
            snapshots: self.snapshots.clone(),
            pending_validations: Vec::new(),
            cardinality_violations: self.cardinality_violations,
            has_hash_mismatch: false,
        };
        state.state_hash = compute_hash(&state);
        Ok(state)
    }
}

fn path_to_string(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string()
}

fn session_status_name(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Initializing => "Initializing",
        SessionStatus::Running => "Running",
        SessionStatus::Complete => "Complete",
        SessionStatus::Error => "Error",
        SessionStatus::Halted => "Halted",
    }
}

fn execution_mode_name(mode: crate::types::ExecutionMode) -> &'static str {
    match mode {
        crate::types::ExecutionMode::Analysis => "analysis",
        crate::types::ExecutionMode::Edit => "edit",
        crate::types::ExecutionMode::Fix => "fix",
        crate::types::ExecutionMode::Batch => "batch",
    }
}

fn parse_session_status(raw: &str) -> Result<SessionStatus, ForgeError> {
    match raw {
        "Initializing" => Ok(SessionStatus::Initializing),
        "Running" => Ok(SessionStatus::Running),
        "Complete" => Ok(SessionStatus::Complete),
        "Error" => Ok(SessionStatus::Error),
        "Halted" => Ok(SessionStatus::Halted),
        _ => Err(ForgeError::InvalidConfiguration(format!(
            "Unknown session status '{}'",
            raw
        ))),
    }
}

fn mutation_type_name(mutation_type: MutationType) -> &'static str {
    match mutation_type {
        MutationType::Write => "Write",
        MutationType::Patch => "Patch",
        MutationType::Delete => "Delete",
        MutationType::Move => "Move",
    }
}

fn parse_mutation_type(raw: &str) -> Result<MutationType, ForgeError> {
    match raw {
        "Write" => Ok(MutationType::Write),
        "Patch" => Ok(MutationType::Patch),
        "Delete" => Ok(MutationType::Delete),
        "Move" => Ok(MutationType::Move),
        _ => Err(ForgeError::InvalidConfiguration(format!(
            "Unknown mutation type '{}'",
            raw
        ))),
    }
}

fn validation_decision_name(decision: ValidationDecision) -> &'static str {
    match decision {
        ValidationDecision::Accept => "Accept",
        ValidationDecision::Reject => "Reject",
        ValidationDecision::Escalate => "Escalate",
    }
}

fn parse_validation_decision(raw: &str) -> Result<ValidationDecision, ForgeError> {
    match raw {
        "Accept" => Ok(ValidationDecision::Accept),
        "Reject" => Ok(ValidationDecision::Reject),
        "Escalate" => Ok(ValidationDecision::Escalate),
        _ => Err(ForgeError::InvalidConfiguration(format!(
            "Unknown validation decision '{}'",
            raw
        ))),
    }
}

fn validation_stage_name(stage: ValidationStage) -> &'static str {
    match stage {
        ValidationStage::Syntax => "Syntax",
        ValidationStage::Format => "Format",
        ValidationStage::Lint => "Lint",
        ValidationStage::Build => "Build",
        ValidationStage::Test => "Test",
    }
}

fn parse_validation_stage(raw: &str) -> Result<ValidationStage, ForgeError> {
    match raw {
        "Syntax" => Ok(ValidationStage::Syntax),
        "Format" => Ok(ValidationStage::Format),
        "Lint" => Ok(ValidationStage::Lint),
        "Build" => Ok(ValidationStage::Build),
        "Test" => Ok(ValidationStage::Test),
        _ => Err(ForgeError::InvalidConfiguration(format!(
            "Unknown validation stage '{}'",
            raw
        ))),
    }
}

fn file_record_to_value(record: &FileRecord) -> Value {
    json!({
        "path": path_to_string(&record.path),
        "content_hash": record.content_hash,
        "size_bytes": record.size_bytes,
        "total_lines": record.total_lines,
        "lines_read": record.lines_read.map(|(offset, limit)| vec![offset, limit]),
        "is_full_read": record.is_full_read,
        "read_at_iteration": record.read_at_iteration,
        "content": record.content,
    })
}

fn parse_file_records(items: &[Value]) -> Result<HashMap<PathBuf, FileRecord>, ForgeError> {
    let mut records = HashMap::new();
    for item in items {
        let object = item.as_object().ok_or_else(|| {
            ForgeError::InvalidConfiguration("files_read entries must be objects".to_string())
        })?;

        let path = PathBuf::from(object.get("path").and_then(Value::as_str).ok_or_else(|| {
            ForgeError::InvalidConfiguration("file record missing path".to_string())
        })?);

        let lines_read = object
            .get("lines_read")
            .and_then(Value::as_array)
            .map(|pair| {
                if pair.len() != 2 {
                    return Err(ForgeError::InvalidConfiguration(
                        "lines_read must contain exactly two integers".to_string(),
                    ));
                }
                let offset = pair[0].as_u64().ok_or_else(|| {
                    ForgeError::InvalidConfiguration(
                        "lines_read offset must be an integer".to_string(),
                    )
                })? as usize;
                let limit = pair[1].as_u64().ok_or_else(|| {
                    ForgeError::InvalidConfiguration(
                        "lines_read limit must be an integer".to_string(),
                    )
                })? as usize;
                Ok((offset, limit))
            })
            .transpose()?;

        let record = FileRecord {
            path: path.clone(),
            content_hash: object
                .get("content_hash")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration("file record missing content_hash".to_string())
                })?
                .to_string(),
            size_bytes: object
                .get("size_bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration("file record missing size_bytes".to_string())
                })?,
            total_lines: object
                .get("total_lines")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration("file record missing total_lines".to_string())
                })? as usize,
            lines_read,
            is_full_read: object
                .get("is_full_read")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration("file record missing is_full_read".to_string())
                })?,
            read_at_iteration: object
                .get("read_at_iteration")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration(
                        "file record missing read_at_iteration".to_string(),
                    )
                })? as u32,
            content: object
                .get("content")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        };

        records.insert(path, record);
    }
    Ok(records)
}

fn validation_stage_result_to_value(result: &ValidationStageResult) -> Value {
    json!({
        "stage": validation_stage_name(result.stage),
        "passed": result.passed,
        "message": result.message,
        "execution_time_ms": result.execution_time_ms,
    })
}

fn parse_validation_stage_results(
    items: &[Value],
) -> Result<Vec<ValidationStageResult>, ForgeError> {
    items
        .iter()
        .map(|item| {
            let object = item.as_object().ok_or_else(|| {
                ForgeError::InvalidConfiguration(
                    "stage_results entries must be objects".to_string(),
                )
            })?;

            Ok(ValidationStageResult {
                stage: parse_validation_stage(
                    object.get("stage").and_then(Value::as_str).ok_or_else(|| {
                        ForgeError::InvalidConfiguration("stage result missing stage".to_string())
                    })?,
                )?,
                passed: object
                    .get("passed")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| {
                        ForgeError::InvalidConfiguration("stage result missing passed".to_string())
                    })?,
                message: object
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ForgeError::InvalidConfiguration("stage result missing message".to_string())
                    })?
                    .to_string(),
                execution_time_ms: object
                    .get("execution_time_ms")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        ForgeError::InvalidConfiguration(
                            "stage result missing execution_time_ms".to_string(),
                        )
                    })?,
            })
        })
        .collect()
}

fn validation_report_to_value(report: &ValidationReport) -> Value {
    json!({
        "decision": validation_decision_name(report.decision),
        "stage_results": report.stage_results.iter().map(validation_stage_result_to_value).collect::<Vec<_>>(),
        "message": report.message,
        "requires_revert": report.requires_revert,
    })
}

fn parse_validation_report(value: &Value) -> Result<ValidationReport, ForgeError> {
    let object = value.as_object().ok_or_else(|| {
        ForgeError::InvalidConfiguration("validation_report must be an object".to_string())
    })?;

    let stage_results = object
        .get("stage_results")
        .and_then(Value::as_array)
        .map(|items| parse_validation_stage_results(items))
        .transpose()?
        .unwrap_or_default();

    Ok(ValidationReport {
        decision: parse_validation_decision(
            object
                .get("decision")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration(
                        "validation_report missing decision".to_string(),
                    )
                })?,
        )?,
        stage_results,
        message: object
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ForgeError::InvalidConfiguration("validation_report missing message".to_string())
            })?
            .to_string(),
        requires_revert: object
            .get("requires_revert")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn change_record_to_value(record: &ChangeRecord) -> Value {
    json!({
        "session_id": record.session_id.to_string(),
        "iteration": record.iteration,
        "timestamp": record.timestamp,
        "mutation": {
            "path": path_to_string(&record.mutation.path),
            "mutation_type": mutation_type_name(record.mutation.mutation_type),
            "content_hash_before": record.mutation.content_hash_before,
            "content_hash_after": record.mutation.content_hash_after,
        },
        "validation_report": validation_report_to_value(&record.validation_report),
    })
}

fn parse_change_record(
    value: &Value,
    fallback_session_id: &str,
) -> Result<ChangeRecord, ForgeError> {
    let object = value.as_object().ok_or_else(|| {
        ForgeError::InvalidConfiguration("change_history entries must be objects".to_string())
    })?;

    let mutation = object
        .get("mutation")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ForgeError::InvalidConfiguration("change record missing mutation".to_string())
        })?;

    Ok(ChangeRecord {
        session_id: SessionId::from_string(
            object
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or(fallback_session_id)
                .to_string(),
        ),
        iteration: object
            .get("iteration")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ForgeError::InvalidConfiguration("change record missing iteration".to_string())
            })? as u32,
        timestamp: object
            .get("timestamp")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ForgeError::InvalidConfiguration("change record missing timestamp".to_string())
            })?,
        mutation: Mutation {
            path: PathBuf::from(
                mutation
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ForgeError::InvalidConfiguration("mutation missing path".to_string())
                    })?,
            ),
            mutation_type: parse_mutation_type(
                mutation
                    .get("mutation_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ForgeError::InvalidConfiguration(
                            "mutation missing mutation_type".to_string(),
                        )
                    })?,
            )?,
            content_hash_before: mutation
                .get("content_hash_before")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            content_hash_after: mutation
                .get("content_hash_after")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        },
        validation_report: parse_validation_report(object.get("validation_report").ok_or_else(
            || {
                ForgeError::InvalidConfiguration(
                    "change record missing validation_report".to_string(),
                )
            },
        )?)?,
    })
}

fn file_snapshot_to_value(snapshot: &FileSnapshot) -> Value {
    json!({
        "path": path_to_string(&snapshot.path),
        "content": snapshot.content,
        "content_hash": snapshot.content_hash,
        "captured_at": snapshot.captured_at,
    })
}

fn parse_snapshots(items: &[Value]) -> Result<HashMap<PathBuf, FileSnapshot>, ForgeError> {
    let mut snapshots = HashMap::new();
    for item in items {
        let object = item.as_object().ok_or_else(|| {
            ForgeError::InvalidConfiguration("snapshot entries must be objects".to_string())
        })?;

        let path = PathBuf::from(object.get("path").and_then(Value::as_str).ok_or_else(|| {
            ForgeError::InvalidConfiguration("snapshot missing path".to_string())
        })?);

        let snapshot = FileSnapshot {
            path: path.clone(),
            content: object
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration("snapshot missing content".to_string())
                })?
                .to_string(),
            content_hash: object
                .get("content_hash")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration("snapshot missing content_hash".to_string())
                })?
                .to_string(),
            captured_at: object
                .get("captured_at")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ForgeError::InvalidConfiguration("snapshot missing captured_at".to_string())
                })?,
        };

        snapshots.insert(path, snapshot);
    }
    Ok(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_validation_report() -> ValidationReport {
        ValidationReport {
            decision: ValidationDecision::Accept,
            stage_results: vec![ValidationStageResult {
                stage: ValidationStage::Syntax,
                passed: true,
                message: "syntax ok".to_string(),
                execution_time_ms: 3,
            }],
            message: "All validation stages passed".to_string(),
            requires_revert: false,
        }
    }

    fn sample_mutation(path: &str, mutation_type: MutationType) -> Mutation {
        Mutation {
            path: PathBuf::from(path),
            mutation_type,
            content_hash_before: Some("sha256:before".to_string()),
            content_hash_after: Some("sha256:after".to_string()),
        }
    }

    #[test]
    fn commit_records_change_history_and_updates_hash() {
        let mut state = AgentState::new(5, "test".to_string(), crate::types::ExecutionMode::Edit);
        state.start().unwrap();
        let previous_hash = state.state_hash.clone();
        let mutation = sample_mutation("src/lib.rs", MutationType::Write);

        state
            .commit(&sample_validation_report(), std::slice::from_ref(&mutation))
            .unwrap();

        assert_eq!(state.change_history.len(), 1);
        assert!(state.files_written.contains(&PathBuf::from("src/lib.rs")));
        assert_eq!(state.change_history[0].mutation.path, mutation.path);
        assert_ne!(state.state_hash, previous_hash);
        state.verify_integrity().unwrap();
    }

    #[test]
    fn save_and_load_preserve_auditable_state() {
        let temp = TempDir::new().unwrap();
        let state_path = temp.path().join("session.json");

        let mut state = AgentState::new(8, "test".to_string(), crate::types::ExecutionMode::Edit);
        state.start().unwrap();
        state.record_file_read(FileRecord::new("src/lib.rs", "fn demo() {}\n", None, 1));
        state.capture_snapshot(&PathBuf::from("src/lib.rs"), "old content");
        state
            .commit(
                &sample_validation_report(),
                &[sample_mutation("src/lib.rs", MutationType::Patch)],
            )
            .unwrap();
        state.next_iteration().unwrap();
        state
            .complete(CompletionReason::new(
                "Updated src/lib.rs line 1 and validated syntax.",
            ))
            .unwrap();

        state.save(&state_path).unwrap();
        let loaded = AgentState::load(&state_path).unwrap();

        assert_eq!(loaded.iteration, state.iteration);
        assert_eq!(loaded.max_iterations, state.max_iterations);
        assert_eq!(loaded.status, SessionStatus::Complete);
        assert_eq!(loaded.files_read.len(), 1);
        assert_eq!(loaded.files_written.len(), 1);
        assert_eq!(loaded.change_history.len(), 1);
        assert_eq!(loaded.snapshots.len(), 1);
        assert_eq!(loaded.state_hash, state.state_hash);
        assert_eq!(
            loaded
                .completion_reason
                .as_ref()
                .map(|reason| reason.as_str()),
            state
                .completion_reason
                .as_ref()
                .map(|reason| reason.as_str())
        );
        loaded.verify_integrity().unwrap();
    }
}
