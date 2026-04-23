//! Chain Registry - First-class Chain Lifecycle Management
//!
//! Makes chains durable, navigable, trackable product objects.
//! Sprint: Chain UX + Session Continuity + Repo-Aware Context Assembly

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// ===========================================================================
/// CHAIN LIFECYCLE STATUS
/// ===========================================================================

/// Product-level chain lifecycle states for operator visibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainLifecycleStatus {
    /// Chain drafted but not ready to run
    Draft,
    /// Chain ready to start execution
    Ready,
    /// Chain currently executing
    Running,
    /// Chain paused waiting for operator approval
    WaitingForApproval,
    /// Chain halted (failure, manual stop, etc.)
    Halted,
    /// Chain failed, cannot continue without intervention
    Failed,
    /// Chain completed successfully
    Complete,
    /// Chain archived (no longer active but retained)
    Archived,
}

impl std::fmt::Display for ChainLifecycleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Ready => write!(f, "ready"),
            Self::Running => write!(f, "running"),
            Self::WaitingForApproval => write!(f, "waiting-for-approval"),
            Self::Halted => write!(f, "halted"),
            Self::Failed => write!(f, "failed"),
            Self::Complete => write!(f, "complete"),
            Self::Archived => write!(f, "archived"),
        }
    }
}

impl ChainLifecycleStatus {
    /// Check if chain can be started from this state
    pub fn can_start(&self) -> bool {
        matches!(self, Self::Ready | Self::Draft)
    }

    /// Check if chain can be resumed from this state
    pub fn can_resume(&self) -> bool {
        matches!(self, Self::Halted | Self::WaitingForApproval | Self::Failed)
    }

    /// Check if chain can be archived from this state
    pub fn can_archive(&self) -> bool {
        matches!(self, Self::Complete | Self::Failed | Self::Halted)
    }

    /// Check if chain is active (not terminal)
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Draft | Self::Ready | Self::Running | Self::WaitingForApproval | Self::Halted
        )
    }

    /// Check if chain is in terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Archived)
    }
}

/// ===========================================================================
/// CHAIN RECORD
/// ===========================================================================

/// Durable record of a chain for product-level tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainRecord {
    /// Unique chain identifier
    pub chain_id: String,
    /// Human-readable title
    pub title: String,
    /// Chain objective description
    pub objective: String,
    /// Current lifecycle status
    pub status: ChainLifecycleStatus,
    /// Creation timestamp (unix millis)
    pub created_at: u64,
    /// Last update timestamp (unix millis)
    pub updated_at: u64,
    /// Total number of steps
    pub step_count: u32,
    /// Number of completed steps
    pub completed_steps: u32,
    /// Associated repo path (if any)
    pub repo_path: Option<String>,
    /// Summary of last run (for quick inspection)
    pub last_run_summary: Option<String>,
}

impl ChainRecord {
    /// Create a new chain record
    pub fn new(
        chain_id: impl Into<String>,
        title: impl Into<String>,
        objective: impl Into<String>,
    ) -> Self {
        let now = crate::types::timestamp_now();
        Self {
            chain_id: chain_id.into(),
            title: title.into(),
            objective: objective.into(),
            status: ChainLifecycleStatus::Draft,
            created_at: now,
            updated_at: now,
            step_count: 0,
            completed_steps: 0,
            repo_path: None,
            last_run_summary: None,
        }
    }

    /// Update status and timestamp
    pub fn update_status(&mut self, status: ChainLifecycleStatus) {
        self.status = status;
        self.updated_at = crate::types::timestamp_now();
    }

    /// Set repo association
    pub fn with_repo(mut self, repo_path: impl Into<String>) -> Self {
        self.repo_path = Some(repo_path.into());
        self
    }

    /// Set step counts
    pub fn with_steps(mut self, total: u32, completed: u32) -> Self {
        self.step_count = total;
        self.completed_steps = completed;
        self
    }

    /// Update run summary
    pub fn update_summary(&mut self, summary: impl Into<String>) {
        self.last_run_summary = Some(summary.into());
        self.updated_at = crate::types::timestamp_now();
    }
}

/// ===========================================================================
/// CHAIN REGISTRY
/// ===========================================================================

/// Registry for tracking all chains in the product
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChainRegistry {
    /// All chains indexed by ID
    pub chains: HashMap<String, ChainRecord>,
    /// Currently active chain ID (if any)
    pub active_chain_id: Option<String>,
    /// Recently accessed chain IDs (for quick switching)
    pub recent_chain_ids: Vec<String>,
}

impl ChainRegistry {
    /// Create empty registry
    pub fn new() -> Self {
        Self {
            chains: HashMap::new(),
            active_chain_id: None,
            recent_chain_ids: Vec::new(),
        }
    }

    /// Register a new chain
    pub fn register(&mut self, record: ChainRecord) -> Result<(), ChainRegistryError> {
        if self.chains.contains_key(&record.chain_id) {
            return Err(ChainRegistryError::DuplicateChainId(record.chain_id));
        }
        let chain_id = record.chain_id.clone();
        self.chains.insert(chain_id.clone(), record);
        self.add_to_recent(chain_id);
        Ok(())
    }

    /// Get a chain record
    pub fn get(&self, chain_id: &str) -> Option<&ChainRecord> {
        self.chains.get(chain_id)
    }

    /// Get mutable chain record
    pub fn get_mut(&mut self, chain_id: &str) -> Option<&mut ChainRecord> {
        self.chains.get_mut(chain_id)
    }

    /// Set active chain
    pub fn set_active(&mut self, chain_id: impl Into<String>) -> Result<(), ChainRegistryError> {
        let chain_id = chain_id.into();
        if !self.chains.contains_key(&chain_id) {
            return Err(ChainRegistryError::ChainNotFound(chain_id));
        }
        // Add current active to recent before switching
        if let Some(current) = &self.active_chain_id {
            self.add_to_recent(current.clone());
        }
        self.active_chain_id = Some(chain_id);
        Ok(())
    }

    /// Clear active chain
    pub fn clear_active(&mut self) {
        if let Some(current) = &self.active_chain_id {
            self.add_to_recent(current.clone());
        }
        self.active_chain_id = None;
    }

    /// Archive a chain
    pub fn archive(&mut self, chain_id: &str) -> Result<(), ChainRegistryError> {
        let record = self
            .chains
            .get_mut(chain_id)
            .ok_or_else(|| ChainRegistryError::ChainNotFound(chain_id.to_string()))?;

        if !record.status.can_archive() {
            return Err(ChainRegistryError::InvalidArchiveState {
                chain_id: chain_id.to_string(),
                status: record.status,
            });
        }

        record.update_status(ChainLifecycleStatus::Archived);
        if self.active_chain_id.as_deref() == Some(chain_id) {
            self.clear_active();
        }
        Ok(())
    }

    /// List all chains
    pub fn list_all(&self) -> Vec<&ChainRecord> {
        let mut chains: Vec<_> = self.chains.values().collect();
        chains.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        chains
    }

    /// List chains by status
    pub fn list_by_status(&self, status: ChainLifecycleStatus) -> Vec<&ChainRecord> {
        let mut chains: Vec<_> = self
            .chains
            .values()
            .filter(|c| c.status == status)
            .collect();
        chains.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        chains
    }

    /// Get active chain (if any)
    pub fn active_chain(&self) -> Option<&ChainRecord> {
        self.active_chain_id
            .as_ref()
            .and_then(|id| self.chains.get(id))
    }

    /// Get recent chains (excluding active)
    pub fn recent_chains(&self, limit: usize) -> Vec<&ChainRecord> {
        self.recent_chain_ids
            .iter()
            .filter(|id| self.active_chain_id.as_deref() != Some(id.as_str()))
            .filter_map(|id| self.chains.get(id))
            .take(limit)
            .collect()
    }

    /// Add to recent list, maintaining uniqueness and recency
    fn add_to_recent(&mut self, chain_id: String) {
        // Remove if exists to move to front
        self.recent_chain_ids.retain(|id| id != &chain_id);
        // Add to front
        self.recent_chain_ids.insert(0, chain_id);
        // Keep only last 10
        if self.recent_chain_ids.len() > 10 {
            self.recent_chain_ids.truncate(10);
        }
    }
}

/// ===========================================================================
/// CHAIN REGISTRY ERRORS
/// ===========================================================================

/// Errors that can occur in chain registry operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainRegistryError {
    DuplicateChainId(String),
    ChainNotFound(String),
    InvalidArchiveState {
        chain_id: String,
        status: ChainLifecycleStatus,
    },
}

impl std::fmt::Display for ChainRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateChainId(id) => write!(f, "Chain '{}' already exists", id),
            Self::ChainNotFound(id) => write!(f, "Chain '{}' not found", id),
            Self::InvalidArchiveState { chain_id, status } => {
                write!(
                    f,
                    "Cannot archive chain '{}' in state '{}'",
                    chain_id, status
                )
            }
        }
    }
}

impl std::error::Error for ChainRegistryError {}

/// ===========================================================================
/// SESSION CONTINUITY STATE
/// ===========================================================================

/// Persisted operator context for session continuity across restarts
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionContinuityState {
    /// Active repo path at shutdown
    pub active_repo: Option<String>,
    /// Active chain ID at shutdown
    pub active_chain_id: Option<String>,
    /// Last selected inspector tab (for UI restore)
    pub last_selected_inspector_tab: Option<String>,
    /// Last selected step in active chain
    pub last_selected_step: Option<u32>,
    /// Last replay focus (if applicable)
    pub last_replay_focus: Option<String>,
    /// Recently accessed chain IDs
    pub recent_chain_ids: Vec<String>,
    /// Chain ID with pending checkpoint at shutdown
    pub pending_checkpoint_chain_id: Option<String>,
}

impl SessionContinuityState {
    /// Check if there's meaningful state to restore
    pub fn has_restorable_state(&self) -> bool {
        self.active_repo.is_some()
            || self.active_chain_id.is_some()
            || self.pending_checkpoint_chain_id.is_some()
    }

    /// Get restore summary for operator display
    pub fn restore_summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(repo) = &self.active_repo {
            parts.push(format!("repo: {}", repo));
        }

        if let Some(chain) = &self.active_chain_id {
            parts.push(format!("chain: {}", chain));
        }

        if let Some(step) = self.last_selected_step {
            parts.push(format!("step: {}", step));
        }

        if self.pending_checkpoint_chain_id.is_some() {
            parts.push("pending approval".to_string());
        }

        if parts.is_empty() {
            "no previous state".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// ===========================================================================
/// CONTEXT ASSEMBLY V2 TYPES
/// ===========================================================================

/// Result of repo-aware context assembly
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextAssemblyResult {
    /// Selected files with reasons
    pub selected_files: Vec<ContextFileSelection>,
    /// Excluded files with reasons
    pub excluded_files: Vec<ContextExclusion>,
    /// Human-readable summary
    pub summary: String,
    /// Token budget used
    pub token_budget_used: usize,
    /// Total token budget available
    pub token_budget_total: usize,
}

impl ContextAssemblyResult {
    /// Create empty result
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a selected file
    pub fn add_selected(
        &mut self,
        path: impl Into<String>,
        reason: impl Into<String>,
        priority: u32,
    ) {
        self.selected_files.push(ContextFileSelection {
            path: path.into(),
            reason: reason.into(),
            priority,
        });
        // Sort by priority (higher first)
        self.selected_files
            .sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Add an excluded file
    pub fn add_excluded(&mut self, path: impl Into<String>, reason: impl Into<String>) {
        self.excluded_files.push(ContextExclusion {
            path: path.into(),
            reason: reason.into(),
        });
    }

    /// Get file paths sorted by priority
    pub fn paths_by_priority(&self) -> Vec<&str> {
        self.selected_files
            .iter()
            .map(|s| s.path.as_str())
            .collect()
    }
}

/// A file selected for context assembly
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFileSelection {
    /// File path
    pub path: String,
    /// Why this file was selected
    pub reason: String,
    /// Priority (higher = more important)
    pub priority: u32,
}

/// A file excluded from context assembly
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextExclusion {
    /// File path
    pub path: String,
    /// Why this file was excluded
    pub reason: String,
}

/// ===========================================================================
/// CONTEXT AUTHORITY LAYER V3 TYPES
/// ===========================================================================

/// Validation status for context assembly
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextValidationStatus {
    Valid,
    Warning,
    Invalid,
}

impl Default for ContextValidationStatus {
    fn default() -> Self {
        ContextValidationStatus::Valid
    }
}

/// Structured validation result for assembled context
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextValidationResult {
    /// Overall validation status
    pub status: ContextValidationStatus,
    /// Warning messages
    pub warnings: Vec<String>,
    /// Error messages
    pub errors: Vec<String>,
    /// Total files considered
    pub total_files: usize,
    /// Estimated token usage
    pub estimated_token_usage: usize,
    /// Number of duplicate selections detected
    pub duplicate_count: usize,
    /// Number of files trimmed due to budget
    pub trimmed_count: usize,
    /// Expected anchor files that were missing
    pub missing_expected_files: Vec<String>,
}

impl ContextValidationResult {
    /// Create a valid result with no issues
    pub fn valid() -> Self {
        Self {
            status: ContextValidationStatus::Valid,
            ..Default::default()
        }
    }

    /// Add a warning
    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
        if self.status == ContextValidationStatus::Valid {
            self.status = ContextValidationStatus::Warning;
        }
    }

    /// Add an error
    pub fn error(&mut self, message: impl Into<String>) {
        self.errors.push(message.into());
        self.status = ContextValidationStatus::Invalid;
    }

    /// Check if valid
    pub fn is_valid(&self) -> bool {
        self.status != ContextValidationStatus::Invalid
    }
}

/// Budget information for context assembly
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextBudgetInfo {
    /// Maximum files allowed
    pub max_files: usize,
    /// Maximum tokens allowed
    pub max_tokens: usize,
    /// Number of files selected after trimming
    pub files_selected: usize,
    /// Tokens used after trimming
    pub tokens_used: usize,
    /// Whether trimming was triggered
    pub trimming_triggered: bool,
    /// Number of files trimmed
    pub files_trimmed: usize,
}

/// Enhanced file selection with inclusion status and trimming info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFileSelectionV3 {
    /// File path
    pub path: String,
    /// Why this file was selected
    pub reason: String,
    /// Priority (higher = more important)
    pub priority: u32,
    /// Whether this file was included in final context
    pub included: bool,
    /// Reason for trimming (if excluded after selection)
    pub trimmed_reason: Option<String>,
    /// Estimated token count for this file
    pub estimated_tokens: Option<usize>,
}

impl ContextFileSelectionV3 {
    /// Create a new included file selection
    pub fn included(path: impl Into<String>, reason: impl Into<String>, priority: u32) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
            priority,
            included: true,
            trimmed_reason: None,
            estimated_tokens: None,
        }
    }

    /// Create a trimmed (excluded) file selection
    pub fn trimmed(
        path: impl Into<String>,
        reason: impl Into<String>,
        priority: u32,
        trim_reason: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
            priority,
            included: false,
            trimmed_reason: Some(trim_reason.into()),
            estimated_tokens: None,
        }
    }
}

/// Diff between two context assembly results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextDiff {
    /// Files added since previous assembly
    pub added_files: Vec<String>,
    /// Files removed since previous assembly
    pub removed_files: Vec<String>,
    /// Files unchanged between assemblies
    pub unchanged_files: Vec<String>,
    /// Files with priority changes
    pub priority_changed: Vec<(String, u32, u32)>, // (path, old_priority, new_priority)
    /// Files with reason changes
    pub reason_changed: Vec<(String, String, String)>, // (path, old_reason, new_reason)
}

impl ContextDiff {
    /// Compute diff between two context assembly results
    pub fn compute(previous: &ContextAssemblyResultV3, current: &ContextAssemblyResultV3) -> Self {
        let prev_paths: std::collections::HashSet<_> =
            previous.files.iter().map(|f| &f.path).collect();
        let curr_paths: std::collections::HashSet<_> =
            current.files.iter().map(|f| &f.path).collect();

        let added_files: Vec<String> = curr_paths
            .difference(&prev_paths)
            .map(|&s| s.clone())
            .collect();
        let removed_files: Vec<String> = prev_paths
            .difference(&curr_paths)
            .map(|&s| s.clone())
            .collect();
        let unchanged_files: Vec<String> = curr_paths
            .intersection(&prev_paths)
            .map(|&s| s.clone())
            .collect();

        // Detect priority and reason changes for unchanged files
        let mut priority_changed = vec![];
        let mut reason_changed = vec![];

        for path in &unchanged_files {
            if let (Some(prev), Some(curr)) = (
                previous.files.iter().find(|f| &f.path == path),
                current.files.iter().find(|f| &f.path == path),
            ) {
                if prev.priority != curr.priority {
                    priority_changed.push((path.clone().into(), prev.priority, curr.priority));
                }
                if prev.reason != curr.reason {
                    reason_changed.push((
                        path.clone().into(),
                        prev.reason.clone(),
                        curr.reason.clone(),
                    ));
                }
            }
        }

        Self {
            added_files,
            removed_files,
            unchanged_files,
            priority_changed,
            reason_changed,
        }
    }

    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        !self.added_files.is_empty()
            || !self.removed_files.is_empty()
            || !self.priority_changed.is_empty()
            || !self.reason_changed.is_empty()
    }
}

/// Enhanced Context Assembly Result V3 with full authority metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextAssemblyResultV3 {
    /// Files with full selection metadata
    pub files: Vec<ContextFileSelectionV3>,
    /// Files explicitly excluded
    pub excluded: Vec<ContextExclusion>,
    /// Validation result
    pub validation: ContextValidationResult,
    /// Budget information
    pub budget: ContextBudgetInfo,
    /// Human-readable summary
    pub summary: String,
}

impl ContextAssemblyResultV3 {
    /// Get paths of included files only
    pub fn included_paths(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|f| f.included)
            .map(|f| f.path.as_str())
            .collect()
    }

    /// Get files that were trimmed (excluded due to budget)
    pub fn trimmed_files(&self) -> Vec<&ContextFileSelectionV3> {
        self.files
            .iter()
            .filter(|f| !f.included && f.trimmed_reason.is_some())
            .collect()
    }

    /// Check if trimming occurred
    pub fn was_trimmed(&self) -> bool {
        self.budget.trimming_triggered || self.files.iter().any(|f| !f.included)
    }
}

/// ===========================================================================
/// CHAIN PLAN SUMMARY
/// ===========================================================================

/// Pre-run summary for operator review
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChainPlanSummary {
    /// Chain ID
    pub chain_id: String,
    /// Interpreted objective from task intake
    pub interpreted_objective: String,
    /// Task classification
    pub task_class: String,
    /// Risk assessment
    pub risk_level: String,
    /// Likely target files
    pub likely_targets: Vec<String>,
    /// Selected context files
    pub selected_context_files: Vec<String>,
    /// Expected checkpoints
    pub checkpoint_plan: Vec<String>,
    /// Git grounding summary
    pub git_grounding_summary: String,
}

impl ChainPlanSummary {
    /// Create new plan summary
    pub fn new(chain_id: impl Into<String>, objective: impl Into<String>) -> Self {
        Self {
            chain_id: chain_id.into(),
            interpreted_objective: objective.into(),
            ..Default::default()
        }
    }

    /// Format as human-readable text
    pub fn format_display(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!("Chain: {}", self.chain_id));
        lines.push(format!("Objective: {}", self.interpreted_objective));
        lines.push(format!(
            "Class: {} | Risk: {}",
            self.task_class, self.risk_level
        ));

        if !self.likely_targets.is_empty() {
            lines.push(format!("Targets: {}", self.likely_targets.join(", ")));
        }

        if !self.selected_context_files.is_empty() {
            lines.push(format!(
                "Context: {} files",
                self.selected_context_files.len()
            ));
        }

        if !self.checkpoint_plan.is_empty() {
            lines.push("Checkpoints:".to_string());
            for checkpoint in &self.checkpoint_plan {
                lines.push(format!("  - {}", checkpoint));
            }
        }

        if !self.git_grounding_summary.is_empty() {
            lines.push(format!("Git: {}", self.git_grounding_summary));
        }

        lines.join("\n")
    }
}

/// ===========================================================================
/// CHAIN CONTINUATION SUMMARY
/// ===========================================================================

/// Summary for chain resume/continuation decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainContinuationSummary {
    /// Chain ID
    pub chain_id: String,
    /// Whether chain can be resumed
    pub resumable: bool,
    /// Current chain status
    pub current_status: String,
    /// Reason if blocked
    pub blocked_reason: Option<String>,
    /// Next required operator action
    pub next_required_action: String,
    /// Step that would resume (if applicable)
    pub resume_target_step: Option<u32>,
}

impl ChainContinuationSummary {
    /// Create resumable summary
    pub fn resumable(
        chain_id: impl Into<String>,
        status: impl Into<String>,
        target_step: u32,
    ) -> Self {
        Self {
            chain_id: chain_id.into(),
            resumable: true,
            current_status: status.into(),
            blocked_reason: None,
            next_required_action: format!("Resume from step {}", target_step),
            resume_target_step: Some(target_step),
        }
    }

    /// Create blocked summary
    pub fn blocked(
        chain_id: impl Into<String>,
        status: impl Into<String>,
        reason: impl Into<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            chain_id: chain_id.into(),
            resumable: false,
            current_status: status.into(),
            blocked_reason: Some(reason.into()),
            next_required_action: next_action.into(),
            resume_target_step: None,
        }
    }

    /// Format as human-readable text
    pub fn format_display(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!("Chain: {}", self.chain_id));
        lines.push(format!("Status: {}", self.current_status));

        if self.resumable {
            if let Some(step) = self.resume_target_step {
                lines.push(format!("Resumable: yes (step {})", step));
            } else {
                lines.push("Resumable: yes".to_string());
            }
        } else {
            lines.push("Resumable: no".to_string());
            if let Some(reason) = &self.blocked_reason {
                lines.push(format!("Blocked: {}", reason));
            }
        }

        lines.push(format!("Next action: {}", self.next_required_action));

        lines.join("\n")
    }
}

/// ===========================================================================
/// TESTS
/// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_lifecycle_status_transitions() {
        assert!(ChainLifecycleStatus::Ready.can_start());
        assert!(ChainLifecycleStatus::Draft.can_start());
        assert!(!ChainLifecycleStatus::Running.can_start());

        assert!(ChainLifecycleStatus::Halted.can_resume());
        assert!(ChainLifecycleStatus::Failed.can_resume());
        assert!(!ChainLifecycleStatus::Running.can_resume());

        assert!(ChainLifecycleStatus::Complete.can_archive());
        assert!(!ChainLifecycleStatus::Running.can_archive());
    }

    #[test]
    fn test_chain_record_creation() {
        let record = ChainRecord::new("chain-1", "Test Chain", "Test objective");
        assert_eq!(record.chain_id, "chain-1");
        assert_eq!(record.status, ChainLifecycleStatus::Draft);
        assert_eq!(record.step_count, 0);
    }

    #[test]
    fn test_chain_registry_basic_operations() {
        let mut registry = ChainRegistry::new();

        let record = ChainRecord::new("chain-1", "Test", "Test objective");
        registry.register(record).unwrap();

        assert!(registry.get("chain-1").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_chain_registry_duplicate_detection() {
        let mut registry = ChainRegistry::new();

        let record = ChainRecord::new("chain-1", "Test", "Test objective");
        registry.register(record.clone()).unwrap();

        let result = registry.register(record);
        assert!(matches!(
            result,
            Err(ChainRegistryError::DuplicateChainId(_))
        ));
    }

    #[test]
    fn test_context_assembly_result() {
        let mut result = ContextAssemblyResult::new();
        result.add_selected("src/main.rs", "entry point", 100);
        result.add_selected("src/lib.rs", "library root", 90);
        result.add_excluded("target/", "build artifacts");

        assert_eq!(result.selected_files.len(), 2);
        assert_eq!(result.excluded_files.len(), 1);

        let paths = result.paths_by_priority();
        assert_eq!(paths[0], "src/main.rs");
        assert_eq!(paths[1], "src/lib.rs");
    }

    #[test]
    fn test_session_continuity_summary() {
        let state = SessionContinuityState {
            active_repo: Some("/path/to/repo".to_string()),
            active_chain_id: Some("chain-123".to_string()),
            last_selected_step: Some(3),
            pending_checkpoint_chain_id: Some("chain-123".to_string()),
            ..Default::default()
        };

        let summary = state.restore_summary();
        assert!(summary.contains("repo:"));
        assert!(summary.contains("chain:"));
        assert!(summary.contains("step:"));
        assert!(summary.contains("pending approval"));
    }
}
