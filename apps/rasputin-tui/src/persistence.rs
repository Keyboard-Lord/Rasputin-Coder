//! Persistence module - simple JSON state storage
//!
//! Stores: active repo, recent repos, conversations, messages, runtime events, model status

use crate::forge_runtime::GitGrounding;
use crate::state::ObjectiveSatisfaction;
use anyhow::Result;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::PathBuf;
use tracing::{debug, info};

/// Canonical app state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentState {
    pub version: String,
    pub last_updated: DateTime<Local>,
    pub active_repo: Option<String>,
    #[serde(default)]
    pub active_conversation: Option<String>,
    #[serde(default)]
    pub active_chain_id: Option<String>,
    pub recent_repos: Vec<RecentRepo>,
    pub conversations: Vec<PersistentConversation>,
    #[serde(default)]
    pub chains: Vec<PersistentChain>,
    pub last_model_status: Option<ModelStatus>,
    #[serde(default)]
    pub chain_policy: ChainPolicy,
    /// V2.4: First-class projects
    #[serde(default)]
    pub projects: Vec<Project>,
    /// V2.4: Currently active project ID
    #[serde(default)]
    pub active_project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentRepo {
    pub path: String,
    pub name: String,
    pub last_opened: DateTime<Local>,
    pub ollama_model: Option<String>,
}

/// V2.4: First-class Project model - user-facing workspace concept
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique project identifier
    pub id: String,
    /// User-facing project name
    pub name: String,
    /// Associated repo path (technical backing)
    pub repo_path: Option<String>,
    /// When project was created
    pub created_at: DateTime<Local>,
    /// When project was last updated
    pub updated_at: DateTime<Local>,
    /// Whether this is the currently active project
    #[serde(default)]
    pub is_active: bool,
    /// Recent conversations associated with this project
    #[serde(default)]
    pub recent_conversation_ids: Vec<String>,
    /// Project metadata (for future extension)
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

impl Project {
    /// Create a new project with the given name
    pub fn new(name: impl Into<String>) -> Self {
        let now = Local::now();
        Self {
            id: format!("project-{}", uuid::Uuid::new_v4()),
            name: name.into(),
            repo_path: None,
            created_at: now,
            updated_at: now,
            is_active: true,
            recent_conversation_ids: vec![],
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create a project from an existing directory
    pub fn from_directory(path: impl Into<String>, name: impl Into<String>) -> Self {
        let mut project = Self::new(name);
        project.repo_path = Some(path.into());
        project
    }

    /// Update the last accessed time
    pub fn touch(&mut self) {
        self.updated_at = Local::now();
    }

    /// Associate a conversation with this project
    pub fn add_conversation(&mut self, conversation_id: impl Into<String>) {
        let id = conversation_id.into();
        if !self.recent_conversation_ids.contains(&id) {
            self.recent_conversation_ids.push(id);
        }
        // Keep only most recent 10
        if self.recent_conversation_ids.len() > 10 {
            self.recent_conversation_ids.remove(0);
        }
        self.touch();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentConversation {
    pub id: String,
    pub title: String,
    pub repo_path: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default = "default_conversation_mode")]
    pub mode: String,
    #[serde(default)]
    pub execution: PersistentExecutionState,
    #[serde(default)]
    pub inspector: PersistentInspectorState,
    pub messages: Vec<PersistentMessage>,
    pub runtime_events: Vec<PersistentEvent>,
    #[serde(default)]
    pub structured_outputs: Vec<PersistentStructuredOutput>,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub chain_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentStructuredOutput {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub source: String,
    pub content: String,
    pub timestamp: DateTime<Local>,
}

/// Chain step within a persistent chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentChainStep {
    pub id: String,
    pub description: String,
    pub status: ChainStepStatus,
    /// Original failed step for an explicit retry/refinement step.
    #[serde(default)]
    pub retry_of: Option<String>,
    /// 0 for original steps, 1..N for policy-bounded retry steps.
    #[serde(default)]
    pub retry_attempt: u32,
    /// AUTHORITATIVE STEP OUTCOME - execution truth for this step
    #[serde(default)]
    pub execution_outcome: Option<ExecutionOutcome>,
    /// Canonical classification of the latest captured execution result.
    #[serde(default)]
    pub execution_result_class: Option<ExecutionResultClass>,
    /// Append-only execution captures for this step.
    #[serde(default)]
    pub execution_results: Vec<ExecutionResultCapture>,
    /// Structured analysis of the latest failed/partial/blocked result.
    #[serde(default)]
    pub failure_reason: Option<FailureReason>,
    /// Recovery action type when this step was generated by the feedback loop.
    #[serde(default)]
    pub recovery_step_kind: Option<RecoveryStepKind>,
    /// Bounded evidence snapshot used to generate this recovery step.
    #[serde(default)]
    pub evidence_snapshot: Option<String>,
    /// Track if force override was used for this step
    #[serde(default)]
    pub force_override_used: bool,
    pub tool_calls: Vec<String>,
    pub result_summary: Option<String>,
    pub validation_passed: Option<bool>,
    pub started_at: Option<DateTime<Local>>,
    pub completed_at: Option<DateTime<Local>>,
    pub error_message: Option<String>,
    /// Replay/audit record for this step (populated when step completes)
    #[serde(default)]
    pub replay_record: Option<StepReplayRecord>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionResultClass {
    Success,
    Failure,
    Partial,
    Blocked,
}

impl ExecutionResultClass {
    pub fn classify(
        success: bool,
        exit_code: Option<i32>,
        error_message: Option<&str>,
        test_results: Option<&str>,
    ) -> Self {
        let error = error_message.unwrap_or_default().to_lowercase();
        let tests = test_results.unwrap_or_default().to_lowercase();

        if success
            && exit_code.unwrap_or(0) == 0
            && !tests.contains("failed")
            && !tests.contains("error")
        {
            return Self::Success;
        }

        let combined = format!("{} {}", error, tests);
        if combined.contains("blocked")
            || combined.contains("approval")
            || combined.contains("permission")
            || combined.contains("preflight")
            || combined.contains("missing context")
        {
            return Self::Blocked;
        }

        if combined.contains("partial") || combined.contains("warning") {
            return Self::Partial;
        }

        Self::Failure
    }

    pub fn allows_retry(self) -> bool {
        matches!(self, Self::Failure | Self::Partial)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureReasonKind {
    CompileError,
    MissingFile,
    TestFailure,
    PermissionDenied,
    PreflightBlocked,
    PartialProgress,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailureReason {
    pub kind: FailureReasonKind,
    pub summary: String,
    pub evidence: String,
    pub recoverable: bool,
}

impl FailureReason {
    pub fn analyze(
        result_class: ExecutionResultClass,
        stdout: &str,
        stderr: &str,
        exit_code: Option<i32>,
        test_results: Option<&str>,
    ) -> Self {
        let evidence = bounded_evidence_snapshot(stdout, stderr, test_results);
        let combined = format!(
            "{}\n{}\n{}",
            stdout.to_lowercase(),
            stderr.to_lowercase(),
            test_results.unwrap_or_default().to_lowercase()
        );

        let (kind, summary, recoverable) = if matches!(result_class, ExecutionResultClass::Blocked)
        {
            if combined.contains("permission") || combined.contains("approval") {
                (
                    FailureReasonKind::PermissionDenied,
                    "Execution blocked by permission or approval requirement".to_string(),
                    false,
                )
            } else if combined.contains("preflight") || combined.contains("missing context") {
                (
                    FailureReasonKind::PreflightBlocked,
                    "Execution blocked before the step could run".to_string(),
                    false,
                )
            } else {
                (
                    FailureReasonKind::Unknown,
                    "Execution blocked by an unrecoverable condition".to_string(),
                    false,
                )
            }
        } else if matches!(result_class, ExecutionResultClass::Partial) {
            (
                FailureReasonKind::PartialProgress,
                "Step made partial progress and requires a bounded continuation".to_string(),
                true,
            )
        } else if combined.contains("no such file")
            || combined.contains("not found")
            || combined.contains("missing file")
        {
            (
                FailureReasonKind::MissingFile,
                "Required file or path was missing".to_string(),
                true,
            )
        } else if combined.contains("test result") && combined.contains("failed")
            || combined.contains("assertion")
            || combined.contains("panicked")
            || combined.contains("expected")
        {
            (
                FailureReasonKind::TestFailure,
                "Validation or test failure detected".to_string(),
                true,
            )
        } else if combined.contains("compile")
            || combined.contains("compiler")
            || combined.contains("error[")
            || combined.contains("syntax")
        {
            (
                FailureReasonKind::CompileError,
                "Compile or syntax error detected".to_string(),
                true,
            )
        } else if exit_code.unwrap_or(0) != 0 {
            (
                FailureReasonKind::Unknown,
                "Execution failed with non-zero exit status".to_string(),
                true,
            )
        } else {
            (
                FailureReasonKind::Unknown,
                "Execution failed without a recognized deterministic signal".to_string(),
                result_class.allows_retry(),
            )
        };

        Self {
            kind,
            summary,
            evidence,
            recoverable,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStepKind {
    Fix,
    Retry,
    Patch,
    ReRun,
}

impl RecoveryStepKind {
    pub fn from_failure_reason(reason: &FailureReason) -> Self {
        match reason.kind {
            FailureReasonKind::CompileError | FailureReasonKind::MissingFile => Self::Patch,
            FailureReasonKind::TestFailure => Self::Fix,
            FailureReasonKind::PartialProgress => Self::Retry,
            FailureReasonKind::Unknown => Self::Retry,
            FailureReasonKind::PermissionDenied | FailureReasonKind::PreflightBlocked => {
                Self::ReRun
            }
        }
    }
}

fn bounded_evidence_snapshot(stdout: &str, stderr: &str, test_results: Option<&str>) -> String {
    const MAX_CHARS: usize = 2048;
    let mut evidence = String::new();
    if !stdout.is_empty() {
        evidence.push_str("stdout:\n");
        evidence.push_str(stdout);
    }
    if !stderr.is_empty() {
        if !evidence.is_empty() {
            evidence.push('\n');
        }
        evidence.push_str("stderr:\n");
        evidence.push_str(stderr);
    }
    if let Some(tests) = test_results {
        if !tests.is_empty() {
            if !evidence.is_empty() {
                evidence.push('\n');
            }
            evidence.push_str("tests:\n");
            evidence.push_str(tests);
        }
    }

    let char_count = evidence.chars().count();
    if char_count <= MAX_CHARS {
        evidence
    } else {
        evidence
            .chars()
            .skip(char_count.saturating_sub(MAX_CHARS))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResultCapture {
    pub attempt: u32,
    pub result_class: ExecutionResultClass,
    pub stdout: String,
    pub stderr: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub test_results: Option<String>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<FailureReason>,
    pub captured_at: DateTime<Local>,
    #[serde(default)]
    pub generated_retry_step_id: Option<String>,
    #[serde(default)]
    pub affected_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChainStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
    Skipped,
}

/// Context file entry with V3 authority metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextFileEntry {
    pub path: String,
    pub reason: String,
    pub priority: u32,
    pub included: bool,
    pub trimmed_reason: Option<String>,
}

/// Validation status for context assembly
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextValidationStatus {
    #[default]
    Valid,
    Warning,
    Invalid,
}

/// Validation result for context assembly
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextValidationResult {
    pub status: ContextValidationStatus,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub total_files: usize,
    pub estimated_token_usage: usize,
}

/// Budget information for context assembly
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextBudgetInfo {
    pub max_files: usize,
    pub max_tokens: usize,
    pub files_selected: usize,
    pub tokens_used: usize,
    pub trimming_triggered: bool,
}

/// Complete context assembly state for V3 authority layer
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextAssemblyState {
    pub files: Vec<ContextFileEntry>,
    pub validation: ContextValidationResult,
    pub budget: ContextBudgetInfo,
    pub summary: String,
    pub assembled_at: DateTime<Local>,
}

/// Durable chain record - survives restart
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentChain {
    pub id: String,
    pub name: String,
    pub objective: String,
    #[serde(default)]
    pub raw_prompt: String,
    pub status: ChainLifecycleStatus,
    pub steps: Vec<PersistentChainStep>,
    pub active_step: Option<usize>,
    pub repo_path: Option<String>,
    pub conversation_id: Option<String>,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    pub completed_at: Option<DateTime<Local>>,
    pub archived: bool,
    pub total_steps_executed: u32,
    pub total_steps_failed: u32,
    /// AUTHORITATIVE EXECUTION OUTCOME - UI renders from this
    #[serde(default)]
    pub execution_outcome: Option<ExecutionOutcome>,
    /// Track if force override was used (maps to SuccessWithWarnings)
    #[serde(default)]
    pub force_override_used: bool,
    /// Existing completion truth for the objective, including explicit artifact contracts.
    #[serde(default)]
    pub objective_satisfaction: ObjectiveSatisfaction,
    /// Context Assembly V2: files selected for context in this chain
    #[serde(default)]
    pub selected_context_files: Vec<String>,
    /// Context Assembly V3: full authority metadata
    #[serde(default)]
    pub context_state: Option<ContextAssemblyState>,
    /// Current approval checkpoint (if waiting for approval)
    #[serde(default)]
    pub pending_checkpoint: Option<ApprovalCheckpoint>,
    /// Git repository state at chain start
    #[serde(default)]
    pub git_grounding: Option<GitGrounding>,
    /// V1.6 AUDIT: Immutable audit log of all state transitions and events
    #[serde(default)]
    pub audit_log: crate::state::AuditLog,
}

impl PersistentChain {
    pub fn raw_prompt_text(&self) -> &str {
        if self.raw_prompt.trim().is_empty() {
            &self.objective
        } else {
            &self.raw_prompt
        }
    }

    /// V1.6 AUDIT: Append an event to the chain's audit log
    pub fn audit_event(&mut self, event: crate::state::AuditEvent) {
        self.audit_log.append(event);
    }

    /// V1.6 AUDIT: Get the last n audit events
    pub fn get_last_audit_events(&self, n: usize) -> Vec<&crate::state::AuditEvent> {
        self.audit_log.get_last_n(n)
    }

    /// V1.6 AUDIT: Get the full transition history
    pub fn get_transition_history(&self) -> Vec<&crate::state::AuditEvent> {
        self.audit_log.get_transition_history()
    }

    /// V1.6 AUDIT: Get the outcome trace
    pub fn get_outcome_trace(&self) -> Vec<&crate::state::AuditEvent> {
        self.audit_log.get_outcome_trace()
    }
    /// Check if this chain can execute a step right now
    /// Returns ExecutionReadiness with detailed status
    pub fn check_execution_readiness(&self, policy: &ChainPolicy) -> ExecutionReadiness {
        // 1. Check chain is in runnable state
        let can_run = matches!(
            self.status,
            ChainLifecycleStatus::Running
                | ChainLifecycleStatus::Ready
                | ChainLifecycleStatus::Draft
        );

        if !can_run {
            return ExecutionReadiness::blocked(BlockedReason::ChainNotRunnable);
        }

        // 2. Check max steps not exceeded
        if self.total_steps_executed >= policy.max_steps {
            return ExecutionReadiness::blocked(BlockedReason::MaxStepsReached);
        }

        // 3. Check consecutive failures
        if self.total_steps_failed >= policy.max_consecutive_failures {
            return ExecutionReadiness::blocked(BlockedReason::TooManyFailures);
        }

        // 4. Check context availability
        let has_context = self.context_state.is_some() || !self.selected_context_files.is_empty();
        if !has_context {
            return ExecutionReadiness::blocked(BlockedReason::MissingContext);
        }

        // 5. Check context validation (V3 only)
        let context_valid = if let Some(ref context) = self.context_state {
            matches!(context.validation.status, ContextValidationStatus::Valid)
        } else {
            true // V2 context assumed valid
        };

        if !context_valid {
            return ExecutionReadiness::blocked(BlockedReason::InvalidContext);
        }

        // 6. Check for pending approval checkpoint
        if let Some(ref checkpoint) = self.pending_checkpoint {
            if checkpoint.is_pending() {
                return ExecutionReadiness::blocked(BlockedReason::WaitingForApproval);
            }
        }

        // 7. Find next pending step
        let next_step = self
            .steps
            .iter()
            .find(|s| matches!(s.status, ChainStepStatus::Pending))
            .or_else(|| {
                // Fall back to active step if still pending
                self.active_step
                    .and_then(|idx| self.steps.get(idx))
                    .filter(|s| matches!(s.status, ChainStepStatus::Pending))
            });

        match next_step {
            Some(step) => {
                let mut readiness =
                    ExecutionReadiness::ready(step.id.clone(), step.description.clone());
                readiness.context_available = true;
                readiness.context_valid = context_valid;
                readiness.system_ready = true;
                readiness.validation_passing = true;
                readiness
            }
            None => {
                // No pending steps - chain is effectively complete
                ExecutionReadiness::blocked(BlockedReason::ChainNotRunnable)
            }
        }
    }

    /// Quick check if chain can auto-advance to next step
    pub fn can_auto_advance(&self, policy: &ChainPolicy) -> bool {
        policy.auto_advance && self.check_execution_readiness(policy).can_execute
    }

    /// Get next step info without full readiness check
    pub fn next_pending_step(&self) -> Option<&PersistentChainStep> {
        self.steps
            .iter()
            .find(|s| matches!(s.status, ChainStepStatus::Pending))
            .or_else(|| {
                self.active_step
                    .and_then(|idx| self.steps.get(idx))
                    .filter(|s| matches!(s.status, ChainStepStatus::Pending))
            })
    }

    /// Enqueue an explicit retry/refinement step after a failed or partial step.
    /// The original failed step is not rewritten; retry lineage is stored on the new step.
    pub fn enqueue_retry_step(
        &mut self,
        failed_step_id: &str,
        result_class: ExecutionResultClass,
        failure_reason: &FailureReason,
        evidence_snapshot: &str,
        max_retries: u32,
        max_chain_recovery_depth: u32,
    ) -> Option<PersistentChainStep> {
        if !result_class.allows_retry() || !failure_reason.recoverable {
            return None;
        }

        let chain_recovery_depth = self
            .steps
            .iter()
            .filter(|step| step.retry_of.is_some())
            .count();
        if chain_recovery_depth as u32 >= max_chain_recovery_depth {
            return None;
        }

        let failed_index = self.steps.iter().position(|s| s.id == failed_step_id)?;
        let failed_step = self.steps.get(failed_index)?.clone();
        let root_step_id = failed_step
            .retry_of
            .clone()
            .unwrap_or_else(|| failed_step.id.clone());

        let highest_attempt = self
            .steps
            .iter()
            .filter(|step| {
                step.id == root_step_id || step.retry_of.as_deref() == Some(&root_step_id)
            })
            .map(|step| step.retry_attempt)
            .max()
            .unwrap_or(0);
        let next_attempt = highest_attempt.saturating_add(1);

        if next_attempt > max_retries {
            return None;
        }

        let retry_step = PersistentChainStep {
            id: format!("retry-{}-{}", next_attempt, uuid::Uuid::new_v4()),
            description: format!(
                "{:?} recovery for step after {:?}: {}\nFailure reason: {}\nEvidence:\n{}",
                RecoveryStepKind::from_failure_reason(failure_reason),
                result_class,
                failed_step.description,
                failure_reason.summary,
                evidence_snapshot
            ),
            status: ChainStepStatus::Pending,
            retry_of: Some(root_step_id),
            retry_attempt: next_attempt,
            execution_outcome: None,
            execution_result_class: None,
            execution_results: vec![],
            failure_reason: Some(failure_reason.clone()),
            recovery_step_kind: Some(RecoveryStepKind::from_failure_reason(failure_reason)),
            evidence_snapshot: Some(evidence_snapshot.to_string()),
            force_override_used: false,
            tool_calls: vec![],
            result_summary: None,
            validation_passed: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            replay_record: None,
        };

        self.steps.insert(failed_index + 1, retry_step.clone());
        Some(retry_step)
    }

    /// Check if chain is blocked and get the reason
    pub fn blocked_state(&self, policy: &ChainPolicy) -> Option<BlockedReason> {
        self.check_execution_readiness(policy).reason
    }

    /// Create an approval checkpoint for a step
    pub fn create_checkpoint(
        &mut self,
        step_id: String,
        step_description: String,
        risk_eval: &RiskEvaluation,
    ) -> &ApprovalCheckpoint {
        let checkpoint = ApprovalCheckpoint::new(
            risk_eval.checkpoint_type,
            risk_eval.reason.clone(),
            risk_eval.level,
            step_id,
            step_description,
            risk_eval.affected_files.clone(),
        );
        self.pending_checkpoint = Some(checkpoint);
        self.pending_checkpoint.as_ref().unwrap()
    }

    /// Approve the pending checkpoint
    pub fn approve_checkpoint(&mut self) -> bool {
        if let Some(ref mut checkpoint) = self.pending_checkpoint {
            checkpoint.approve();
            true
        } else {
            false
        }
    }

    /// Deny the pending checkpoint
    pub fn deny_checkpoint(&mut self) -> bool {
        if let Some(ref mut checkpoint) = self.pending_checkpoint {
            checkpoint.deny();
            true
        } else {
            false
        }
    }

    /// Clear the pending checkpoint
    pub fn clear_checkpoint(&mut self) {
        self.pending_checkpoint = None;
    }

    /// Check if there's a pending checkpoint
    pub fn has_pending_checkpoint(&self) -> bool {
        self.pending_checkpoint
            .as_ref()
            .map(|c| c.is_pending())
            .unwrap_or(false)
    }

    /// Get the pending checkpoint if any
    pub fn get_pending_checkpoint(&self) -> Option<&ApprovalCheckpoint> {
        self.pending_checkpoint.as_ref()
    }

    /// AUTHORITATIVE OUTCOME COMPUTATION - SINGLE WRITE POINT
    /// Aggregates step outcomes into chain-level outcome using worst-case rules:
    /// - Any Failed step -> chain = Failed
    /// - Any Blocked step (terminal) -> chain = Blocked
    /// - All Success + any SuccessWithWarnings -> chain = SuccessWithWarnings
    /// - All Success -> chain = Success
    pub fn finalize_outcome(&mut self) -> ExecutionOutcome {
        let outcome = self.aggregate_step_outcomes();
        self.execution_outcome = Some(outcome);
        outcome
    }

    /// Get the authoritative outcome if set
    pub fn get_outcome(&self) -> Option<ExecutionOutcome> {
        self.execution_outcome
    }

    /// Mark that force override was used (affects outcome classification)
    pub fn mark_force_override(&mut self) {
        self.force_override_used = true;
    }

    pub fn recorded_affected_paths(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut paths = Vec::new();

        for step in &self.steps {
            for capture in &step.execution_results {
                for path in &capture.affected_paths {
                    if seen.insert(path.clone()) {
                        paths.push(path.clone());
                    }
                }
            }
        }

        paths
    }

    /// AGGREGATE step outcomes into chain-level truth
    /// This ensures multi-step chains cannot hide warning/failure states
    fn aggregate_step_outcomes(&self) -> ExecutionOutcome {
        // Collect outcomes from all executed steps (not pending)
        let executed_steps: Vec<_> = self
            .steps
            .iter()
            .filter(|s| !matches!(s.status, ChainStepStatus::Pending))
            .collect();

        if executed_steps.is_empty() {
            // No steps executed yet - treat as blocked (not started)
            return ExecutionOutcome::Blocked;
        }

        // Check for any failed steps (highest priority)
        let has_failed = executed_steps.iter().any(|s| {
            s.execution_outcome == Some(ExecutionOutcome::Failed)
                || matches!(s.status, ChainStepStatus::Failed)
        });
        // Also check chain-level status for failed
        let chain_failed = matches!(self.status, ChainLifecycleStatus::Failed);
        if has_failed || chain_failed {
            return ExecutionOutcome::Failed;
        }

        // Check for blocked steps that haven't been resolved
        let has_blocked = executed_steps.iter().any(|s| {
            s.execution_outcome == Some(ExecutionOutcome::Blocked)
                || matches!(s.status, ChainStepStatus::Blocked)
        });
        // Only report blocked if chain hasn't completed (not all steps done)
        let all_steps_complete = self.steps.iter().all(|s| {
            matches!(s.status, ChainStepStatus::Completed)
                || s.execution_outcome == Some(ExecutionOutcome::Success)
                || s.execution_outcome == Some(ExecutionOutcome::SuccessWithWarnings)
        });
        // Check chain-level blocked status
        let chain_blocked = matches!(
            self.status,
            ChainLifecycleStatus::Halted | ChainLifecycleStatus::WaitingForApproval
        );
        if (has_blocked || chain_blocked) && !all_steps_complete {
            return ExecutionOutcome::Blocked;
        }

        if self
            .objective_satisfaction
            .artifact_contract
            .as_ref()
            .is_some_and(|contract| contract.has_requirements() && !contract.is_satisfied())
        {
            return ExecutionOutcome::Blocked;
        }

        // Check for warning steps (SuccessWithWarnings) or chain-level force override
        let has_warnings = executed_steps.iter().any(|s| {
            s.execution_outcome == Some(ExecutionOutcome::SuccessWithWarnings)
                || s.force_override_used
        });
        let chain_force_override = self.force_override_used;

        // All executed steps are successful (maybe with warnings)
        if has_warnings || chain_force_override {
            ExecutionOutcome::SuccessWithWarnings
        } else {
            ExecutionOutcome::Success
        }
    }

    /// Record outcome for a specific step when it completes
    pub fn record_step_outcome(
        &mut self,
        step_id: &str,
        outcome: ExecutionOutcome,
        force_used: bool,
    ) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == step_id) {
            step.execution_outcome = Some(outcome);
            step.force_override_used = force_used;
            // Also update step status to match outcome
            step.status = match outcome {
                ExecutionOutcome::Success | ExecutionOutcome::SuccessWithWarnings => {
                    ChainStepStatus::Completed
                }
                ExecutionOutcome::Blocked => ChainStepStatus::Blocked,
                ExecutionOutcome::Failed => ChainStepStatus::Failed,
            };
        }
    }

    /// Check if any executed step has warnings
    pub fn has_warning_steps(&self) -> bool {
        self.steps.iter().any(|s| {
            s.execution_outcome == Some(ExecutionOutcome::SuccessWithWarnings)
                || s.force_override_used
        })
    }

    /// Get count of steps by outcome
    pub fn count_steps_by_outcome(&self) -> (usize, usize, usize, usize) {
        let mut success = 0;
        let mut warnings = 0;
        let mut blocked = 0;
        let mut failed = 0;

        for step in &self.steps {
            match step.execution_outcome {
                Some(ExecutionOutcome::Success) => success += 1,
                Some(ExecutionOutcome::SuccessWithWarnings) => warnings += 1,
                Some(ExecutionOutcome::Blocked) => blocked += 1,
                Some(ExecutionOutcome::Failed) => failed += 1,
                None => {
                    // Fallback to status if outcome not set
                    match step.status {
                        ChainStepStatus::Completed => success += 1,
                        ChainStepStatus::Blocked => blocked += 1,
                        ChainStepStatus::Failed => failed += 1,
                        _ => {}
                    }
                }
            }
        }

        (success, warnings, blocked, failed)
    }
}

/// AUTHORITATIVE EXECUTION OUTCOME - UI Single Source of Truth
/// Maps from runtime ExecutionOutcome. UI renders ONLY from this.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    /// All steps completed successfully, no warnings
    Success,
    /// Completed but with non-fatal warnings (e.g., force override used)
    SuccessWithWarnings,
    /// Could not complete due to blocking condition
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

    /// Compute outcome from chain lifecycle status and force override flag
    pub fn from_chain_status(status: ChainLifecycleStatus, force_override_used: bool) -> Self {
        match status {
            ChainLifecycleStatus::Complete => {
                if force_override_used {
                    ExecutionOutcome::SuccessWithWarnings
                } else {
                    ExecutionOutcome::Success
                }
            }
            ChainLifecycleStatus::Failed => ExecutionOutcome::Failed,
            ChainLifecycleStatus::Halted | ChainLifecycleStatus::WaitingForApproval => {
                ExecutionOutcome::Blocked
            }
            // Transitional states - treat as blocked (not yet complete)
            _ => ExecutionOutcome::Blocked,
        }
    }

    /// Check if outcome represents a failure state
    pub fn is_failure(&self) -> bool {
        matches!(self, ExecutionOutcome::Failed)
    }

    /// Check if outcome represents blocked state
    pub fn is_blocked(&self) -> bool {
        matches!(self, ExecutionOutcome::Blocked)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChainLifecycleStatus {
    Draft,
    Ready,
    Running,
    WaitingForApproval,
    Halted,
    Failed,
    Complete,
    Archived,
}

impl ChainLifecycleStatus {
    /// V1.6 REPLAY: Convert to ExecutionState for replay validation
    pub fn to_execution_state(self) -> crate::state::ExecutionState {
        match self {
            ChainLifecycleStatus::Draft => crate::state::ExecutionState::Idle,
            ChainLifecycleStatus::Ready => crate::state::ExecutionState::Idle,
            ChainLifecycleStatus::Running => crate::state::ExecutionState::Executing,
            ChainLifecycleStatus::WaitingForApproval => {
                crate::state::ExecutionState::WaitingForApproval
            }
            ChainLifecycleStatus::Halted => crate::state::ExecutionState::Blocked,
            ChainLifecycleStatus::Failed => crate::state::ExecutionState::Failed,
            ChainLifecycleStatus::Complete => crate::state::ExecutionState::Done,
            ChainLifecycleStatus::Archived => crate::state::ExecutionState::Done,
        }
    }

    /// V1.6 CHECKPOINT: Check if this is a terminal (non-resumable) status
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ChainLifecycleStatus::Complete
                | ChainLifecycleStatus::Failed
                | ChainLifecycleStatus::Archived
        )
    }
}

/// Why a chain is blocked from execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    /// No context assembled for this chain
    MissingContext,
    /// Context validation failed or has warnings that prevent execution
    InvalidContext,
    /// Required files for the step are not present
    MissingFiles(Vec<String>),
    /// System not in valid state (e.g., no repo, no model)
    SystemNotReady,
    /// Validation gate failed
    ValidationFailed,
    /// Step execution failed and halt_on_failure is true
    StepFailed,
    /// Max steps reached
    MaxStepsReached,
    /// Waiting for operator approval
    WaitingForApproval,
    /// Consecutive failures exceeded threshold
    TooManyFailures,
    /// Chain is not in runnable state
    ChainNotRunnable,
}

impl BlockedReason {
    /// Get human-readable description
    pub fn description(&self) -> String {
        match self {
            BlockedReason::MissingContext => "No context assembled".to_string(),
            BlockedReason::InvalidContext => "Context validation failed".to_string(),
            BlockedReason::MissingFiles(files) => format!("Missing files: {}", files.join(", ")),
            BlockedReason::SystemNotReady => "System not ready".to_string(),
            BlockedReason::ValidationFailed => "Validation failed".to_string(),
            BlockedReason::StepFailed => "Step failed".to_string(),
            BlockedReason::MaxStepsReached => "Max steps reached".to_string(),
            BlockedReason::WaitingForApproval => "Waiting for approval".to_string(),
            BlockedReason::TooManyFailures => "Too many consecutive failures".to_string(),
            BlockedReason::ChainNotRunnable => "Chain not in runnable state".to_string(),
        }
    }
}

/// Risk level for approval checkpoints
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Low risk - auto-execute (e.g., read operations, small changes)
    Low,
    /// Medium risk - configurable approval (e.g., single file write)
    Medium,
    /// High risk - always require approval (e.g., deletes, dependency changes)
    High,
}

impl RiskLevel {
    /// Get display label
    pub fn label(&self) -> &'static str {
        match self {
            RiskLevel::Low => "Low",
            RiskLevel::Medium => "Medium",
            RiskLevel::High => "High",
        }
    }

    /// Get color for UI display
    pub fn color(&self) -> &'static str {
        match self {
            RiskLevel::Low => "green",
            RiskLevel::Medium => "yellow",
            RiskLevel::High => "red",
        }
    }
}

/// Type of operation that triggered checkpoint
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointType {
    /// File write operation
    FileWrite,
    /// File deletion
    FileDelete,
    /// Dependency change (Cargo.toml, package.json, etc.)
    DependencyChange,
    /// Large context shift (many files modified)
    LargeContextShift,
    /// Unknown or unclassified operation
    UnknownOperation,
    /// Multiple file modifications
    MultiFileModification,
    /// Configuration file change
    ConfigChange,
    /// Build script modification
    BuildScriptChange,
}

impl CheckpointType {
    /// Get human-readable description
    pub fn description(&self) -> String {
        match self {
            CheckpointType::FileWrite => "File write".to_string(),
            CheckpointType::FileDelete => "File deletion".to_string(),
            CheckpointType::DependencyChange => "Dependency change".to_string(),
            CheckpointType::LargeContextShift => "Large context shift".to_string(),
            CheckpointType::UnknownOperation => "Unknown operation".to_string(),
            CheckpointType::MultiFileModification => "Multi-file modification".to_string(),
            CheckpointType::ConfigChange => "Configuration change".to_string(),
            CheckpointType::BuildScriptChange => "Build script change".to_string(),
        }
    }
}

/// Approval decision status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    /// Pending operator decision
    Pending,
    /// Operator approved - can execute
    Approved,
    /// Operator denied - skip or halt
    Denied,
}

/// Approval checkpoint for risky operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalCheckpoint {
    pub id: String,
    pub checkpoint_type: CheckpointType,
    pub reason: String,
    pub risk_level: RiskLevel,
    pub status: ApprovalStatus,
    pub step_id: String,
    pub step_description: String,
    pub created_at: DateTime<Local>,
    pub decided_at: Option<DateTime<Local>>,
    pub affected_files: Vec<String>,
}

impl ApprovalCheckpoint {
    /// Create new pending checkpoint
    pub fn new(
        checkpoint_type: CheckpointType,
        reason: String,
        risk_level: RiskLevel,
        step_id: String,
        step_description: String,
        affected_files: Vec<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            checkpoint_type,
            reason,
            risk_level,
            status: ApprovalStatus::Pending,
            step_id,
            step_description,
            created_at: Local::now(),
            decided_at: None,
            affected_files,
        }
    }

    /// Mark as approved
    pub fn approve(&mut self) {
        self.status = ApprovalStatus::Approved;
        self.decided_at = Some(Local::now());
    }

    /// Mark as denied
    pub fn deny(&mut self) {
        self.status = ApprovalStatus::Denied;
        self.decided_at = Some(Local::now());
    }

    /// Check if approved
    pub fn is_approved(&self) -> bool {
        matches!(self.status, ApprovalStatus::Approved)
    }

    /// Check if denied
    pub fn is_denied(&self) -> bool {
        matches!(self.status, ApprovalStatus::Denied)
    }

    /// Check if pending
    pub fn is_pending(&self) -> bool {
        matches!(self.status, ApprovalStatus::Pending)
    }
}

/// Risk evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskEvaluation {
    pub level: RiskLevel,
    pub reason: String,
    pub checkpoint_type: CheckpointType,
    pub affected_files: Vec<String>,
    pub requires_approval: bool,
}

impl RiskEvaluation {
    /// Evaluate risk for a step action
    pub fn evaluate(step_action: &crate::state::StepAction, affected_paths: &[String]) -> Self {
        use crate::state::StepAction;

        let mut risk_level = RiskLevel::Low;
        let mut reason = "Standard operation".to_string();
        let mut checkpoint_type = CheckpointType::UnknownOperation;
        let affected_files = affected_paths.to_vec();

        match step_action {
            StepAction::WriteFile { path, .. } => {
                risk_level = RiskLevel::Medium;
                reason = format!("Writing file: {}", path);
                checkpoint_type = CheckpointType::FileWrite;

                // Elevate risk for certain file types
                if is_high_risk_file(path) {
                    risk_level = RiskLevel::High;
                    reason = format!("Writing sensitive file: {}", path);
                }
            }
            StepAction::PatchFile { path, .. } => {
                risk_level = RiskLevel::Medium;
                reason = format!("Modifying file: {}", path);
                checkpoint_type = CheckpointType::FileWrite;

                if is_high_risk_file(path) {
                    risk_level = RiskLevel::High;
                    reason = format!("Modifying sensitive file: {}", path);
                }
            }
            StepAction::ReadFile { .. } => {
                risk_level = RiskLevel::Low;
                reason = "Reading file (safe)".to_string();
                checkpoint_type = CheckpointType::UnknownOperation;
            }
            StepAction::RunCommand { command, .. } => {
                risk_level = RiskLevel::Medium;
                reason = format!("Running command: {}", command);
                checkpoint_type = CheckpointType::UnknownOperation;

                // Elevate for destructive commands
                if is_destructive_command(command) {
                    risk_level = RiskLevel::High;
                    reason = format!("Destructive command: {}", command);
                }
            }
            StepAction::Git { operation, .. } => match operation.as_str() {
                "commit" | "push" | "merge" => {
                    risk_level = RiskLevel::High;
                    reason = format!("Git {} operation", operation);
                    checkpoint_type = CheckpointType::UnknownOperation;
                }
                _ => {
                    risk_level = RiskLevel::Low;
                    reason = format!("Git {} (safe)", operation);
                }
            },
            StepAction::CreateDirectory { .. } => {
                risk_level = RiskLevel::Low;
                reason = "Creating directory (safe)".to_string();
            }
            _ => {
                // Default evaluation based on affected paths count
                if affected_paths.len() > 5 {
                    risk_level = RiskLevel::Medium;
                    reason = format!("Multiple files affected ({})", affected_paths.len());
                    checkpoint_type = CheckpointType::MultiFileModification;
                }
            }
        }

        // Check for large context shifts
        if affected_paths.len() > 10 {
            risk_level = RiskLevel::High;
            reason = format!("Large context shift: {} files", affected_paths.len());
            checkpoint_type = CheckpointType::LargeContextShift;
        }

        let requires_approval = matches!(risk_level, RiskLevel::High);

        Self {
            level: risk_level,
            reason,
            checkpoint_type,
            affected_files,
            requires_approval,
        }
    }
}

// ============================================================================
// V1.6 CHECKPOINT: Validated execution checkpoint snapshots
// Durable, audit-grounded resumable execution state with workspace integrity
// ============================================================================

/// V1.6 CHECKPOINT: Source/reason for checkpoint creation
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointSource {
    /// Manual checkpoint created by operator
    Manual,
    /// Auto-checkpoint after successful validated step completion
    AutoValidatedStep,
    /// Checkpoint created during safe halt
    SafeHalt,
    /// Checkpoint at approval pause boundary
    ApprovalPause,
    /// Checkpoint created for crash recovery
    CrashRecovery,
    /// Checkpoint at explicit save point
    ExplicitSave,
}

/// V1.6 CHECKPOINT: Validation status of a checkpoint
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointValidationStatus {
    /// Checkpoint validated and ready for resume
    Valid,
    /// Checkpoint has warnings but may be resumable
    Warning,
    /// Checkpoint is invalid and cannot be used for resume
    Invalid,
    /// Checkpoint validation not yet performed
    Unchecked,
}

/// V1.6 CHECKPOINT: Result of attempting to resume from a checkpoint
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckpointResumeResult {
    /// Resume successful, chain ready to continue
    Success {
        chain_id: String,
        resumed_step: usize,
        message: String,
    },
    /// Resume blocked due to validation failure
    Blocked {
        reason: String,
        recovery_action: String,
    },
    /// Checkpoint stale - workspace has diverged
    Stale {
        checkpoint_hash: String,
        current_hash: String,
        diverged_files: Vec<String>,
    },
    /// Checkpoint corrupted or unreadable
    Corrupted { path: String, error: String },
    /// Checkpoint/audit divergence detected
    Divergent {
        checkpoint_state: String,
        replayed_state: String,
        audit_event_index: usize,
    },
}

/// Operator-facing checkpoint verdict.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOperatorStatus {
    Valid,
    Stale,
    Corrupted,
    Divergent,
    Missing,
}

impl CheckpointOperatorStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Valid => "Valid",
            Self::Stale => "Stale",
            Self::Corrupted => "Corrupted",
            Self::Divergent => "Divergent",
            Self::Missing => "Missing",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Valid => "✓",
            Self::Stale => "!",
            Self::Corrupted => "x",
            Self::Divergent => "~",
            Self::Missing => "?",
        }
    }
}

/// Operator-facing result for one validation dimension.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointCheckStatus {
    Passed,
    Failed,
    NotChecked,
}

impl CheckpointCheckStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotChecked => "not checked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointCheckReport {
    pub status: CheckpointCheckStatus,
    pub detail: String,
}

impl CheckpointCheckReport {
    pub fn passed(detail: impl Into<String>) -> Self {
        Self {
            status: CheckpointCheckStatus::Passed,
            detail: detail.into(),
        }
    }

    pub fn failed(detail: impl Into<String>) -> Self {
        Self {
            status: CheckpointCheckStatus::Failed,
            detail: detail.into(),
        }
    }

    pub fn not_checked(detail: impl Into<String>) -> Self {
        Self {
            status: CheckpointCheckStatus::NotChecked,
            detail: detail.into(),
        }
    }
}

/// First-class checkpoint validation report for operator surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointOperatorReport {
    pub chain_id: String,
    pub checkpoint_id: Option<String>,
    pub checkpoint_timestamp: Option<DateTime<Local>>,
    pub active_step: Option<usize>,
    pub step_description: Option<String>,
    pub audit_cursor: Option<usize>,
    pub audit_log_len: usize,
    pub workspace_hash: Option<String>,
    pub workspace_result: CheckpointCheckReport,
    pub replay_result: CheckpointCheckReport,
    pub final_status: CheckpointOperatorStatus,
    pub resume_allowed: bool,
    pub smallest_safe_next_action: String,
}

impl CheckpointOperatorReport {
    pub fn missing(chain_id: impl Into<String>, audit_log_len: usize) -> Self {
        Self {
            chain_id: chain_id.into(),
            checkpoint_id: None,
            checkpoint_timestamp: None,
            active_step: None,
            step_description: None,
            audit_cursor: None,
            audit_log_len,
            workspace_hash: None,
            workspace_result: CheckpointCheckReport::not_checked("no checkpoint selected"),
            replay_result: CheckpointCheckReport::not_checked("no audit cursor available"),
            final_status: CheckpointOperatorStatus::Missing,
            resume_allowed: false,
            smallest_safe_next_action:
                "/checkpoint list, then restart from a validated checkpoint boundary".to_string(),
        }
    }

    pub fn corrupted(
        chain_id: impl Into<String>,
        checkpoint_id: impl Into<String>,
        error: impl Into<String>,
        audit_log_len: usize,
    ) -> Self {
        let error = error.into();
        Self {
            chain_id: chain_id.into(),
            checkpoint_id: Some(checkpoint_id.into()),
            checkpoint_timestamp: None,
            active_step: None,
            step_description: None,
            audit_cursor: None,
            audit_log_len,
            workspace_hash: None,
            workspace_result: CheckpointCheckReport::failed(format!(
                "checkpoint cannot be decoded: {}",
                error
            )),
            replay_result: CheckpointCheckReport::not_checked(
                "replay blocked because checkpoint is unreadable",
            ),
            final_status: CheckpointOperatorStatus::Corrupted,
            resume_allowed: false,
            smallest_safe_next_action: "Delete corrupted checkpoint and use a valid checkpoint"
                .to_string(),
        }
    }
}

/// V1.6 CHECKPOINT: Validated execution checkpoint snapshot
/// Durable, audit-grounded checkpoint with workspace integrity verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionCheckpoint {
    /// Unique checkpoint identifier
    pub checkpoint_id: String,
    /// Chain this checkpoint belongs to
    pub chain_id: String,
    /// Active step index at checkpoint (next step to execute)
    pub active_step: Option<usize>,
    /// Chain lifecycle status at checkpoint
    pub lifecycle_status: ChainLifecycleStatus,
    /// Aggregated execution outcome so far
    pub aggregated_outcome: Option<ExecutionOutcome>,
    /// Current execution state at checkpoint boundary
    pub execution_state: crate::state::ExecutionState,
    /// Audit cursor - last processed audit event index
    pub audit_cursor: usize,
    /// Workspace hash summary at checkpoint time
    pub workspace_hash: String,
    /// Files tracked in workspace hash (for partial verification)
    pub tracked_files: Vec<String>,
    /// Individual file hashes captured at checkpoint time
    #[serde(default)]
    pub tracked_file_hashes: Vec<FileHash>,
    /// Validation status proving safe checkpoint boundary
    pub validation_status: CheckpointValidationStatus,
    /// Checkpoint creation timestamp
    pub created_at: DateTime<Local>,
    /// Source/reason for checkpoint creation
    pub source: CheckpointSource,
    /// Optional message or context
    pub message: Option<String>,
    /// Schema version for backward compatibility
    pub schema_version: u32,
}

impl ExecutionCheckpoint {
    /// Current schema version
    pub const CURRENT_SCHEMA: u32 = 1;

    /// Create a new validated checkpoint
    pub fn new(
        chain_id: String,
        active_step: Option<usize>,
        lifecycle_status: ChainLifecycleStatus,
        aggregated_outcome: Option<ExecutionOutcome>,
        execution_state: crate::state::ExecutionState,
        audit_cursor: usize,
        workspace_hash: String,
        tracked_files: Vec<String>,
        source: CheckpointSource,
        message: Option<String>,
    ) -> Self {
        Self {
            checkpoint_id: format!(
                "chk-{}-{}",
                chain_id,
                uuid::Uuid::new_v4().to_string()[..8].to_string()
            ),
            chain_id,
            active_step,
            lifecycle_status,
            aggregated_outcome,
            execution_state,
            audit_cursor,
            workspace_hash,
            tracked_files,
            tracked_file_hashes: Vec::new(),
            validation_status: CheckpointValidationStatus::Unchecked,
            created_at: Local::now(),
            source,
            message,
            schema_version: Self::CURRENT_SCHEMA,
        }
    }

    /// Attach the captured per-file hash evidence for workspace divergence reporting.
    pub fn with_tracked_file_hashes(mut self, hashes: Vec<FileHash>) -> Self {
        self.tracked_file_hashes = hashes;
        self
    }

    /// Mark checkpoint as validated
    pub fn mark_valid(&mut self) {
        self.validation_status = CheckpointValidationStatus::Valid;
    }

    /// Mark checkpoint as having warnings
    pub fn mark_warning(&mut self, _reason: &str) {
        self.validation_status = CheckpointValidationStatus::Warning;
    }

    /// Mark checkpoint as invalid
    pub fn mark_invalid(&mut self) {
        self.validation_status = CheckpointValidationStatus::Invalid;
    }

    /// Check if checkpoint can be used for resume
    pub fn is_resumable(&self) -> bool {
        matches!(
            self.validation_status,
            CheckpointValidationStatus::Valid | CheckpointValidationStatus::Warning
        )
    }

    /// Get checkpoint filename
    pub fn filename(&self) -> String {
        format!("{}.json", self.checkpoint_id)
    }
}

/// V1.6 CHECKPOINT: Workspace hash summary for integrity verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHash {
    /// Overall workspace hash (combined hash of all tracked files)
    pub overall_hash: String,
    /// Individual file hashes
    pub file_hashes: Vec<FileHash>,
    /// Hash calculation timestamp
    pub calculated_at: DateTime<Local>,
    /// Base directory for the workspace
    pub base_path: String,
}

/// V1.6 CHECKPOINT: Individual file hash entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHash {
    /// Relative file path
    pub path: String,
    /// File content hash (blake3)
    pub hash: String,
    /// File size in bytes
    pub size: u64,
    /// Last modified timestamp
    pub modified: DateTime<Local>,
}

/// Check if a file is high-risk
fn is_high_risk_file(path: &str) -> bool {
    let high_risk_patterns = [
        "Cargo.toml",
        "package.json",
        "requirements.txt",
        "Makefile",
        "Dockerfile",
        ".github/workflows",
        ".env",
        "config",
        "settings",
    ];
    high_risk_patterns.iter().any(|p| path.contains(p))
}

/// Check if a command is destructive
fn is_destructive_command(command: &str) -> bool {
    let destructive_patterns = [
        "rm -rf", "rm -r /", "dd if=", "mkfs", "> /dev", "shutdown", "reboot",
    ];
    destructive_patterns.iter().any(|p| command.contains(p))
}

impl BlockedReason {
    /// Get suggested action to resolve
    pub fn suggested_action(&self) -> String {
        match self {
            BlockedReason::MissingContext => "/plan context".to_string(),
            BlockedReason::InvalidContext => "/plan context".to_string(),
            BlockedReason::MissingFiles(_) => "/read <file>".to_string(),
            BlockedReason::SystemNotReady => "Attach project and configure model".to_string(),
            BlockedReason::ValidationFailed => "/validate".to_string(),
            BlockedReason::StepFailed => "/chain resume".to_string(),
            BlockedReason::MaxStepsReached => "/chains to select different chain".to_string(),
            BlockedReason::WaitingForApproval => "/approve or /deny".to_string(),
            BlockedReason::TooManyFailures => "/chains".to_string(),
            BlockedReason::ChainNotRunnable => "/chain status".to_string(),
        }
    }
}

/// Execution readiness check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReadiness {
    pub can_execute: bool,
    pub reason: Option<BlockedReason>,
    pub context_available: bool,
    pub context_valid: bool,
    pub system_ready: bool,
    pub validation_passing: bool,
    pub next_step_id: Option<String>,
    pub next_step_description: Option<String>,
}

impl ExecutionReadiness {
    /// Create ready state
    pub fn ready(next_step_id: String, description: String) -> Self {
        Self {
            can_execute: true,
            reason: None,
            context_available: true,
            context_valid: true,
            system_ready: true,
            validation_passing: true,
            next_step_id: Some(next_step_id),
            next_step_description: Some(description),
        }
    }

    /// Create blocked state
    pub fn blocked(reason: BlockedReason) -> Self {
        Self {
            can_execute: false,
            reason: Some(reason),
            context_available: false,
            context_valid: false,
            system_ready: false,
            validation_passing: false,
            next_step_id: None,
            next_step_description: None,
        }
    }
}

/// Chain execution policy - bounded execution constraints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainPolicy {
    pub max_steps: u32,
    pub require_validation_each_step: bool,
    pub halt_on_failure: bool,
    pub max_consecutive_failures: u32,
    pub auto_retry_on_validation_failure: bool,
    #[serde(default = "default_max_auto_retries_per_step")]
    pub max_auto_retries_per_step: u32,
    #[serde(default = "default_max_chain_recovery_depth")]
    pub max_chain_recovery_depth: u32,
    pub require_approval_after_step_count: Option<u32>,
    #[serde(default)]
    pub auto_resume: bool, // Phase C: Auto-trigger /chain resume on step completion
    #[serde(default)]
    pub auto_advance: bool, // Auto-progress to next step when current completes
    // Approval checkpoint policy settings
    #[serde(default)]
    pub require_approval_for_medium: bool, // Require approval for medium risk (default: false)
    #[serde(default = "default_approval_high")]
    pub require_approval_for_high: bool, // Require approval for high risk (default: true)
    #[serde(default)]
    pub allow_auto_low_risk: bool, // Auto-execute low risk (default: false)
}

fn default_approval_high() -> bool {
    true // High risk always requires approval by default
}

fn default_max_auto_retries_per_step() -> u32 {
    1
}

fn default_max_chain_recovery_depth() -> u32 {
    3
}

impl Default for ChainPolicy {
    fn default() -> Self {
        Self {
            max_steps: 100,
            require_validation_each_step: true,
            halt_on_failure: true,
            max_consecutive_failures: 3,
            auto_retry_on_validation_failure: false,
            max_auto_retries_per_step: default_max_auto_retries_per_step(),
            max_chain_recovery_depth: default_max_chain_recovery_depth(),
            require_approval_after_step_count: None,
            auto_resume: false,  // Default: manual resume (opt-in for auto)
            auto_advance: false, // Default: manual step progression
            require_approval_for_medium: false, // Medium risk auto-executes by default
            require_approval_for_high: true, // High risk always requires approval
            allow_auto_low_risk: true, // Low risk auto-executes
        }
    }
}

impl ChainPolicy {
    /// Check if approval is required for a given risk level
    pub fn requires_approval_for(&self, risk_level: RiskLevel) -> bool {
        match risk_level {
            RiskLevel::Low => !self.allow_auto_low_risk,
            RiskLevel::Medium => self.require_approval_for_medium,
            RiskLevel::High => self.require_approval_for_high,
        }
    }

    /// Check if a step can auto-execute based on risk
    pub fn can_auto_execute(&self, risk_level: RiskLevel) -> bool {
        !self.requires_approval_for(risk_level)
    }
}

/// Replay record for a completed step - enables reconstruction, review, and comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReplayRecord {
    /// Unique replay record ID
    pub replay_id: String,
    /// Associated chain ID
    pub chain_id: String,
    /// Associated step ID
    pub step_id: String,
    /// Task/objective summary
    pub task_summary: String,
    /// Step description
    pub step_description: String,
    /// Execution start time
    pub execution_start: DateTime<Local>,
    /// Execution end time
    pub execution_end: DateTime<Local>,
    /// Planner model used
    pub planner_model: Option<String>,
    /// Context fingerprint (hash of selected files)
    pub context_fingerprint: String,
    /// Number of context files
    pub context_file_count: usize,
    /// State fingerprint before execution
    pub state_before: String,
    /// State fingerprint after execution
    pub state_after: String,
    /// Validation result summary
    pub validation_summary: Option<String>,
    /// Approval checkpoint data if approval was required
    pub approval_checkpoint: Option<ApprovalCheckpoint>,
    /// Tool calls executed during this step
    pub tool_calls: Vec<ToolCallRecord>,
    /// Mutations attempted
    pub mutations_attempted: Vec<String>,
    /// Mutations committed (files written)
    pub mutations_committed: Vec<String>,
    /// Mutations reverted (if any)
    pub mutations_reverted: Vec<String>,
    /// Final outcome
    pub outcome: ReplayOutcome,
    /// Execution fingerprint (deterministic seal)
    pub execution_fingerprint: String,
    /// Replay comparison result (if replayed)
    pub replay_comparison: Option<ReplayComparisonResult>,
    /// Git repository state at step execution time
    pub git_grounding: Option<GitGrounding>,
}

impl StepReplayRecord {
    /// Create a new replay record for a step
    pub fn new(
        chain_id: String,
        step_id: String,
        task_summary: String,
        step_description: String,
        planner_model: Option<String>,
        context_fingerprint: String,
        context_file_count: usize,
    ) -> Self {
        Self {
            replay_id: format!(
                "replay-{}-{}-{}",
                crate::text::take_chars(&chain_id, 8),
                step_id,
                chrono::Local::now().timestamp()
            ),
            chain_id,
            step_id,
            task_summary,
            step_description,
            execution_start: Local::now(),
            execution_end: Local::now(),
            planner_model,
            context_fingerprint,
            context_file_count,
            state_before: String::new(),
            state_after: String::new(),
            validation_summary: None,
            approval_checkpoint: None,
            tool_calls: Vec::new(),
            mutations_attempted: Vec::new(),
            mutations_committed: Vec::new(),
            mutations_reverted: Vec::new(),
            outcome: ReplayOutcome::Unknown,
            execution_fingerprint: String::new(),
            replay_comparison: None,
            git_grounding: None,
        }
    }

    /// Mark execution as complete and generate fingerprint
    pub fn finalize(&mut self, success: bool) {
        self.execution_end = Local::now();
        self.outcome = if success {
            ReplayOutcome::Success
        } else {
            ReplayOutcome::Failure
        };
        self.execution_fingerprint = self.generate_fingerprint();
    }

    /// Generate deterministic execution fingerprint
    fn generate_fingerprint(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.task_summary.hash(&mut hasher);
        self.step_description.hash(&mut hasher);
        self.context_fingerprint.hash(&mut hasher);
        self.planner_model.hash(&mut hasher);
        self.tool_calls.len().hash(&mut hasher);
        self.mutations_committed.len().hash(&mut hasher);
        self.outcome.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Get execution duration
    pub fn duration(&self) -> chrono::Duration {
        self.execution_end
            .signed_duration_since(self.execution_start)
    }

    /// Check if this step had approval checkpoint
    pub fn had_approval(&self) -> bool {
        self.approval_checkpoint.is_some()
    }

    /// Check if approval was granted
    pub fn approval_granted(&self) -> bool {
        self.approval_checkpoint
            .as_ref()
            .map(|a| a.is_approved())
            .unwrap_or(false)
    }
}

/// Outcome of a replay execution
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReplayOutcome {
    /// Step completed successfully
    Success,
    /// Step failed
    Failure,
    /// Step was skipped
    Skipped,
    /// Outcome unknown or not recorded
    Unknown,
}

/// Record of a tool call during execution
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub timestamp: DateTime<Local>,
    pub success: bool,
    pub affected_files: Vec<String>,
    pub error_message: Option<String>,
}

/// Replay comparison result - outcome of comparing original vs replay
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayComparisonResult {
    /// Whether original and replay matched
    pub matched: bool,
    /// First mismatch found (if any)
    pub first_mismatch: Option<MismatchDetail>,
    /// List of all mismatches
    pub mismatches: Vec<MismatchDetail>,
    /// Human-readable summary
    pub summary: String,
    /// Replay timestamp
    pub replayed_at: DateTime<Local>,
}

impl ReplayComparisonResult {
    /// Create a successful match result
    pub fn matched() -> Self {
        Self {
            matched: true,
            first_mismatch: None,
            mismatches: Vec::new(),
            summary: "Replay matched original execution".to_string(),
            replayed_at: Local::now(),
        }
    }

    /// Create a mismatch result
    pub fn mismatch(first: MismatchDetail, all: Vec<MismatchDetail>) -> Self {
        let summary = format!(
            "Replay diverged: {} mismatches. First: {} - {}",
            all.len(),
            first.category,
            first.description
        );
        Self {
            matched: false,
            first_mismatch: Some(first),
            mismatches: all,
            summary,
            replayed_at: Local::now(),
        }
    }
}

/// Detail of a single mismatch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MismatchDetail {
    /// Category of mismatch
    pub category: String,
    /// Human-readable description
    pub description: String,
    /// Original value
    pub original: String,
    /// Replay value
    pub replay: String,
    /// Severity
    pub severity: MismatchSeverity,
}

/// Severity of a mismatch
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MismatchSeverity {
    /// Minor difference (e.g., timestamps)
    Trivial,
    /// Notable but not execution-breaking
    Minor,
    /// Significant difference in behavior
    Major,
    /// Critical divergence
    Critical,
}

/// Replay engine for comparing executions
pub struct ReplayEngine;

impl ReplayEngine {
    /// Compare an original replay record with a replay execution
    pub fn compare_records(
        original: &StepReplayRecord,
        replay: &StepReplayRecord,
    ) -> ReplayComparisonResult {
        let mut mismatches = Vec::new();

        // Compare step count (simplified - in reality we'd compare full chain)
        // For step-level comparison, we assume same step

        // Compare tool-call sequence
        if original.tool_calls.len() != replay.tool_calls.len() {
            mismatches.push(MismatchDetail {
                category: "tool_calls".to_string(),
                description: format!(
                    "Tool call count mismatch: {} vs {}",
                    original.tool_calls.len(),
                    replay.tool_calls.len()
                ),
                original: format!("{} calls", original.tool_calls.len()),
                replay: format!("{} calls", replay.tool_calls.len()),
                severity: MismatchSeverity::Major,
            });
        } else {
            for (i, (orig, rep)) in original
                .tool_calls
                .iter()
                .zip(replay.tool_calls.iter())
                .enumerate()
            {
                if orig.tool_name != rep.tool_name {
                    mismatches.push(MismatchDetail {
                        category: "tool_sequence".to_string(),
                        description: format!(
                            "Tool {} differs: {} vs {}",
                            i, orig.tool_name, rep.tool_name
                        ),
                        original: orig.tool_name.clone(),
                        replay: rep.tool_name.clone(),
                        severity: MismatchSeverity::Major,
                    });
                }
                if orig.success != rep.success {
                    mismatches.push(MismatchDetail {
                        category: "tool_result".to_string(),
                        description: format!("Tool {} success differs", orig.tool_name),
                        original: if orig.success {
                            "success".to_string()
                        } else {
                            "failed".to_string()
                        },
                        replay: if rep.success {
                            "success".to_string()
                        } else {
                            "failed".to_string()
                        },
                        severity: MismatchSeverity::Critical,
                    });
                }
            }
        }

        // Compare committed file set
        let orig_files: std::collections::HashSet<_> =
            original.mutations_committed.iter().collect();
        let rep_files: std::collections::HashSet<_> = replay.mutations_committed.iter().collect();

        if orig_files != rep_files {
            mismatches.push(MismatchDetail {
                category: "committed_files".to_string(),
                description: "Committed file set differs".to_string(),
                original: format!("{} files", orig_files.len()),
                replay: format!("{} files", rep_files.len()),
                severity: MismatchSeverity::Critical,
            });
        }

        // Compare outcome
        if original.outcome != replay.outcome {
            mismatches.push(MismatchDetail {
                category: "outcome".to_string(),
                description: format!(
                    "Outcome differs: {:?} vs {:?}",
                    original.outcome, replay.outcome
                ),
                original: format!("{:?}", original.outcome),
                replay: format!("{:?}", replay.outcome),
                severity: MismatchSeverity::Critical,
            });
        }

        // Compare execution fingerprints (deterministic comparison)
        if original.execution_fingerprint != replay.execution_fingerprint {
            mismatches.push(MismatchDetail {
                category: "fingerprint".to_string(),
                description: "Execution fingerprint differs (deterministic mismatch)".to_string(),
                original: original.execution_fingerprint.clone(),
                replay: replay.execution_fingerprint.clone(),
                severity: MismatchSeverity::Critical,
            });
        }

        // Compare approval decisions
        match (&original.approval_checkpoint, &replay.approval_checkpoint) {
            (Some(orig), Some(rep)) => {
                if orig.status != rep.status {
                    mismatches.push(MismatchDetail {
                        category: "approval".to_string(),
                        description: "Approval decision differs".to_string(),
                        original: format!("{:?}", orig.status),
                        replay: format!("{:?}", rep.status),
                        severity: MismatchSeverity::Major,
                    });
                }
            }
            (Some(_), None) | (None, Some(_)) => {
                mismatches.push(MismatchDetail {
                    category: "approval".to_string(),
                    description: "Approval checkpoint presence differs".to_string(),
                    original: if original.approval_checkpoint.is_some() {
                        "present".to_string()
                    } else {
                        "absent".to_string()
                    },
                    replay: if replay.approval_checkpoint.is_some() {
                        "present".to_string()
                    } else {
                        "absent".to_string()
                    },
                    severity: MismatchSeverity::Major,
                });
            }
            _ => {}
        }

        if mismatches.is_empty() {
            ReplayComparisonResult::matched()
        } else {
            let first = mismatches[0].clone();
            ReplayComparisonResult::mismatch(first, mismatches)
        }
    }

    /// Generate context fingerprint from selected files
    pub fn generate_context_fingerprint(files: &[String], content: &[(String, String)]) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        files.len().hash(&mut hasher);
        for (path, hash) in content {
            path.hash(&mut hasher);
            hash.hash(&mut hasher);
        }
        format!("ctx-{:016x}", hasher.finish())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentEvent {
    pub timestamp: DateTime<Local>,
    pub source: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentExecutionState {
    #[serde(default = "default_conversation_mode")]
    pub mode: String,
    #[serde(default = "default_execution_state")]
    pub state: String,
    #[serde(default)]
    pub active_objective: Option<String>,
    #[serde(default = "default_last_action")]
    pub last_action: String,
    #[serde(default)]
    pub current_step: Option<String>,
    #[serde(default)]
    pub step_index: Option<u32>,
    #[serde(default)]
    pub step_total: Option<u32>,
    #[serde(default)]
    pub active_tool: Option<String>,
    #[serde(default)]
    pub planner_output: Vec<String>,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    #[serde(default)]
    pub file_writes: Vec<String>,
    #[serde(default)]
    pub validation_summary: Option<String>,
    #[serde(default)]
    pub block_reason: Option<String>,
    #[serde(default)]
    pub block_fix: Option<String>,
    #[serde(default)]
    pub block_command: Option<String>,
}

impl Default for PersistentExecutionState {
    fn default() -> Self {
        Self {
            mode: default_conversation_mode(),
            state: default_execution_state(),
            active_objective: None,
            last_action: default_last_action(),
            current_step: None,
            step_index: None,
            step_total: None,
            active_tool: None,
            planner_output: vec![],
            tool_calls: vec![],
            file_writes: vec![],
            validation_summary: None,
            block_reason: None,
            block_fix: None,
            block_command: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentInspectorState {
    #[serde(default)]
    pub show_inspector: bool,
    #[serde(default = "default_inspector_tab")]
    pub active_tab: String,
    #[serde(default)]
    pub runtime_scroll: usize,
    #[serde(default)]
    pub validation_scroll: usize,
    #[serde(default)]
    pub logs_scroll: usize,
    #[serde(default)]
    pub preview_scroll: usize,
    #[serde(default)]
    pub diff_scroll: usize,
}

impl Default for PersistentInspectorState {
    fn default() -> Self {
        Self {
            show_inspector: false,
            active_tab: default_inspector_tab(),
            runtime_scroll: 0,
            validation_scroll: 0,
            logs_scroll: 0,
            preview_scroll: 0,
            diff_scroll: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatus {
    pub configured: Option<String>,
    pub active: Option<String>,
    pub ollama_connected: bool,
    pub verified_at: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectsStore {
    version: String,
    last_updated: DateTime<Local>,
    active_repo: Option<String>,
    recent_repos: Vec<RecentRepo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatsStore {
    version: String,
    last_updated: DateTime<Local>,
    active_conversation: Option<String>,
    conversations: Vec<PersistentConversation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExecutionStateStore {
    version: String,
    last_updated: DateTime<Local>,
    last_model_status: Option<ModelStatus>,
}

impl PersistentState {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            last_updated: Local::now(),
            active_repo: None,
            active_conversation: None,
            active_chain_id: None,
            recent_repos: vec![],
            conversations: vec![],
            chains: vec![],
            last_model_status: None,
            chain_policy: ChainPolicy::default(),
            projects: vec![],
            active_project_id: None,
        }
    }

    /// V2.4: Get active project
    pub fn get_active_project(&self) -> Option<&Project> {
        self.active_project_id
            .as_ref()
            .and_then(|id| self.projects.iter().find(|p| p.id == *id))
    }

    /// V2.4: Get mutable active project
    pub fn get_active_project_mut(&mut self) -> Option<&mut Project> {
        let id = self.active_project_id.clone()?;
        self.projects.iter_mut().find(|p| p.id == id)
    }

    /// V2.4: Set active project by ID
    pub fn set_active_project(&mut self, project_id: Option<String>) {
        // Deactivate current project
        if let Some(current_id) = &self.active_project_id {
            if let Some(project) = self.projects.iter_mut().find(|p| p.id == *current_id) {
                project.is_active = false;
            }
        }

        // Activate new project
        self.active_project_id = project_id.clone();
        if let Some(id) = project_id {
            if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                project.is_active = true;
                project.touch();
            }
        }

        self.last_updated = Local::now();
    }

    /// V2.4: Create a new project and set it as active
    pub fn create_project(&mut self, name: impl Into<String>) -> &Project {
        let project = Project::new(name);
        let id = project.id.clone();
        self.projects.push(project);
        self.set_active_project(Some(id));
        self.projects.last().unwrap()
    }

    /// V2.4: Create a project from an existing directory
    pub fn create_project_from_directory(
        &mut self,
        path: impl Into<String>,
        name: impl Into<String>,
    ) -> &Project {
        let project = Project::from_directory(path, name);
        let id = project.id.clone();
        self.projects.push(project);
        self.set_active_project(Some(id));
        self.projects.last().unwrap()
    }

    /// V2.4: Get project by ID
    pub fn get_project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    /// V2.4: Get mutable project by ID
    pub fn get_project_mut(&mut self, id: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    /// V2.4: Get or create active project
    /// Returns the active project, or creates a default one if none exists
    pub fn get_or_create_active_project(&mut self) -> &Project {
        if self.active_project_id.is_none() || self.get_active_project().is_none() {
            // Create a default project
            let project = Project::new("Default Project");
            let id = project.id.clone();
            self.projects.push(project);
            self.active_project_id = Some(id);
        }
        self.get_active_project().unwrap()
    }

    /// Get chain by ID
    pub fn get_chain(&self, id: &str) -> Option<&PersistentChain> {
        self.chains.iter().find(|c| c.id == id)
    }

    /// Get mutable chain by ID
    pub fn get_chain_mut(&mut self, id: &str) -> Option<&mut PersistentChain> {
        self.chains.iter_mut().find(|c| c.id == id)
    }

    /// Get active chain
    pub fn get_active_chain(&self) -> Option<&PersistentChain> {
        self.active_chain_id
            .as_ref()
            .and_then(|id| self.get_chain(id))
    }

    /// Get mutable active chain
    pub fn get_active_chain_mut(&mut self) -> Option<&mut PersistentChain> {
        let id = self.active_chain_id.clone()?;
        self.get_chain_mut(&id)
    }

    /// Set active chain
    pub fn set_active_chain(&mut self, chain_id: Option<String>) {
        self.active_chain_id = chain_id;
        self.last_updated = Local::now();
    }

    /// Create a new chain and set it as active
    pub fn create_chain(
        &mut self,
        name: impl Into<String>,
        objective: impl Into<String>,
    ) -> &PersistentChain {
        let objective = objective.into();
        let id = format!(
            "chain-{}-{:08x}",
            Local::now().timestamp(),
            rand::random::<u32>()
        );
        let chain = PersistentChain {
            id: id.clone(),
            name: name.into(),
            objective: objective.clone(),
            raw_prompt: objective,
            status: ChainLifecycleStatus::Draft,
            steps: vec![],
            active_step: None,
            repo_path: self.active_repo.clone(),
            conversation_id: self.active_conversation.clone(),
            created_at: Local::now(),
            updated_at: Local::now(),
            completed_at: None,
            archived: false,
            total_steps_executed: 0,
            total_steps_failed: 0,
            execution_outcome: None,
            force_override_used: false,
            objective_satisfaction: ObjectiveSatisfaction::default(),
            selected_context_files: vec![],
            context_state: None,
            git_grounding: None,
            pending_checkpoint: None,
            audit_log: crate::state::AuditLog::new(),
        };
        self.chains.push(chain);
        self.set_active_chain(Some(id));
        self.get_chain(&self.active_chain_id.as_ref().unwrap())
            .unwrap()
    }

    /// Archive a chain
    pub fn archive_chain(&mut self, chain_id: &str) -> Result<(), String> {
        if let Some(chain) = self.get_chain_mut(chain_id) {
            if chain.status == ChainLifecycleStatus::Running {
                return Err("Cannot archive a running chain".to_string());
            }
            chain.archived = true;
            chain.status = ChainLifecycleStatus::Archived;
            chain.updated_at = Local::now();

            // If this was the active chain, clear it
            if self.active_chain_id.as_deref() == Some(chain_id) {
                self.active_chain_id = None;
            }
            Ok(())
        } else {
            Err(format!("Chain '{}' not found", chain_id))
        }
    }

    /// Get non-archived chains sorted by most recently updated
    pub fn get_active_chains(&self) -> Vec<&PersistentChain> {
        let mut chains: Vec<_> = self.chains.iter().filter(|c| !c.archived).collect();
        chains.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        chains
    }

    /// Get pending checkpoint for active chain
    pub fn get_active_checkpoint(&self) -> Option<&ApprovalCheckpoint> {
        self.get_active_chain()
            .and_then(|c| c.get_pending_checkpoint())
    }

    /// Check if active chain has pending checkpoint
    pub fn has_active_checkpoint(&self) -> bool {
        self.get_active_chain()
            .map(|c| c.has_pending_checkpoint())
            .unwrap_or(false)
    }

    /// Approve checkpoint for active chain
    pub fn approve_active_checkpoint(&mut self) -> bool {
        if let Some(chain_id) = self.active_chain_id.clone() {
            if let Some(chain) = self.get_chain_mut(&chain_id) {
                return chain.approve_checkpoint();
            }
        }
        false
    }

    /// Deny checkpoint for active chain
    pub fn deny_active_checkpoint(&mut self) -> bool {
        if let Some(chain_id) = self.active_chain_id.clone() {
            if let Some(chain) = self.get_chain_mut(&chain_id) {
                return chain.deny_checkpoint();
            }
        }
        false
    }

    /// Clear checkpoint for active chain
    pub fn clear_active_checkpoint(&mut self) {
        if let Some(chain_id) = self.active_chain_id.clone() {
            if let Some(chain) = self.get_chain_mut(&chain_id) {
                chain.clear_checkpoint();
            }
        }
    }

    pub fn data_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("rasputin")
    }

    pub fn state_path() -> PathBuf {
        Self::data_dir().join("state.json")
    }

    pub fn projects_path() -> PathBuf {
        Self::data_dir().join("projects.json")
    }

    pub fn chats_path() -> PathBuf {
        Self::data_dir().join("chats.json")
    }

    pub fn execution_state_path() -> PathBuf {
        Self::data_dir().join("execution_state.json")
    }

    pub async fn load() -> Result<Self> {
        if Self::split_paths_exist() {
            let state = Self::load_split().await?;
            info!(
                "Loaded persisted state from split files: {} conversations, {} recent repos",
                state.conversations.len(),
                state.recent_repos.len()
            );
            return Ok(state);
        }

        let path = Self::state_path();
        if !path.exists() {
            info!("No persisted state found, creating new");
            return Ok(Self::new());
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let state: Self = serde_json::from_str(&content)?;

        info!(
            "Loaded persisted legacy state: {} conversations, {} recent repos",
            state.conversations.len(),
            state.recent_repos.len()
        );

        Ok(state)
    }

    pub async fn save(&self) -> Result<()> {
        let mut state = self.clone();
        state.last_updated = Local::now();
        Self::save_split_async(&state).await?;
        debug!(
            "Persisted split state to {:?}, {:?}, {:?}",
            Self::projects_path(),
            Self::chats_path(),
            Self::execution_state_path()
        );
        Ok(())
    }

    pub fn save_sync(&self) -> Result<()> {
        let mut state = self.clone();
        state.last_updated = Local::now();
        Self::save_split_sync(&state)?;
        debug!(
            "Persisted split state synchronously to {:?}, {:?}, {:?}",
            Self::projects_path(),
            Self::chats_path(),
            Self::execution_state_path()
        );
        Ok(())
    }

    fn split_paths_exist() -> bool {
        Self::projects_path().exists()
            || Self::chats_path().exists()
            || Self::execution_state_path().exists()
    }

    async fn load_split() -> Result<Self> {
        let projects: ProjectsStore = Self::read_json_or_default_async(
            &Self::projects_path(),
            ProjectsStore {
                version: "1.0".to_string(),
                last_updated: Local::now(),
                active_repo: None,
                recent_repos: vec![],
            },
        )
        .await?;
        let chats: ChatsStore = Self::read_json_or_default_async(
            &Self::chats_path(),
            ChatsStore {
                version: "1.0".to_string(),
                last_updated: Local::now(),
                active_conversation: None,
                conversations: vec![],
            },
        )
        .await?;
        let execution: ExecutionStateStore = Self::read_json_or_default_async(
            &Self::execution_state_path(),
            ExecutionStateStore {
                version: "1.0".to_string(),
                last_updated: Local::now(),
                last_model_status: None,
            },
        )
        .await?;

        Ok(Self {
            version: "1.0".to_string(),
            last_updated: projects
                .last_updated
                .max(chats.last_updated)
                .max(execution.last_updated),
            active_repo: projects.active_repo,
            active_conversation: chats.active_conversation,
            active_chain_id: None,
            recent_repos: projects.recent_repos,
            conversations: chats.conversations,
            chains: vec![],
            last_model_status: execution.last_model_status,
            chain_policy: ChainPolicy::default(),
            // V2.4: Initialize projects from legacy data
            projects: vec![],
            active_project_id: None,
        })
    }

    async fn read_json_or_default_async<T>(path: &PathBuf, default: T) -> Result<T>
    where
        T: DeserializeOwned,
    {
        if !path.exists() {
            return Ok(default);
        }

        let content = tokio::fs::read_to_string(path).await?;
        Ok(serde_json::from_str(&content)?)
    }

    fn save_split_payloads(state: &Self) -> (ProjectsStore, ChatsStore, ExecutionStateStore) {
        let projects = ProjectsStore {
            version: state.version.clone(),
            last_updated: state.last_updated,
            active_repo: state.active_repo.clone(),
            recent_repos: state.recent_repos.clone(),
        };
        let chats = ChatsStore {
            version: state.version.clone(),
            last_updated: state.last_updated,
            active_conversation: state.active_conversation.clone(),
            conversations: state.conversations.clone(),
        };
        let execution = ExecutionStateStore {
            version: state.version.clone(),
            last_updated: state.last_updated,
            last_model_status: state.last_model_status.clone(),
        };
        (projects, chats, execution)
    }

    async fn save_split_async(state: &Self) -> Result<()> {
        let data_dir = Self::data_dir();
        tokio::fs::create_dir_all(&data_dir).await?;

        let (projects, chats, execution) = Self::save_split_payloads(state);
        Self::write_json_atomic_async(&Self::projects_path(), &projects).await?;
        Self::write_json_atomic_async(&Self::chats_path(), &chats).await?;
        Self::write_json_atomic_async(&Self::execution_state_path(), &execution).await?;
        Ok(())
    }

    fn save_split_sync(state: &Self) -> Result<()> {
        let data_dir = Self::data_dir();
        std::fs::create_dir_all(&data_dir)?;

        let (projects, chats, execution) = Self::save_split_payloads(state);
        Self::write_json_atomic_sync(&Self::projects_path(), &projects)?;
        Self::write_json_atomic_sync(&Self::chats_path(), &chats)?;
        Self::write_json_atomic_sync(&Self::execution_state_path(), &execution)?;
        Ok(())
    }

    async fn write_json_atomic_async<T>(path: &PathBuf, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let content = serde_json::to_string_pretty(value)?;
        let temp_path = path.with_extension("tmp");
        tokio::fs::write(&temp_path, content).await?;
        tokio::fs::rename(&temp_path, path).await?;
        Ok(())
    }

    fn write_json_atomic_sync<T>(path: &PathBuf, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let content = serde_json::to_string_pretty(value)?;
        let temp_path = path.with_extension("tmp");
        std::fs::write(&temp_path, content)?;
        std::fs::rename(&temp_path, path)?;
        Ok(())
    }

    /// Add or update a repo in recent repos list
    pub fn touch_repo(&mut self, path: &str, name: &str, ollama_model: Option<&str>) {
        // Remove if exists
        self.recent_repos.retain(|r| r.path != path);

        // Add to front
        self.recent_repos.insert(
            0,
            RecentRepo {
                path: path.to_string(),
                name: name.to_string(),
                last_opened: Local::now(),
                ollama_model: ollama_model.map(|s| s.to_string()),
            },
        );

        // Keep only last 10
        self.recent_repos.truncate(10);

        self.active_repo = Some(path.to_string());
        self.active_conversation = self
            .conversations
            .iter()
            .filter(|conversation| conversation.repo_path.as_deref() == Some(path))
            .max_by_key(|conversation| conversation.updated_at)
            .map(|conversation| conversation.id.clone());
    }

    /// Find conversation by ID or create new
    pub fn get_or_create_conversation(&mut self, id: &str) -> &mut PersistentConversation {
        self.active_conversation = Some(id.to_string());
        if let Some(idx) = self.conversations.iter().position(|c| c.id == id) {
            &mut self.conversations[idx]
        } else {
            let new_conv = PersistentConversation {
                id: id.to_string(),
                title: "New Conversation".to_string(),
                repo_path: self.active_repo.clone(),
                project_id: self.active_repo.clone(),
                mode: default_conversation_mode(),
                execution: PersistentExecutionState::default(),
                inspector: PersistentInspectorState::default(),
                messages: vec![],
                runtime_events: vec![],
                structured_outputs: vec![],
                created_at: Local::now(),
                updated_at: Local::now(),
                archived: false,
                chain_id: self.active_chain_id.clone(),
            };
            self.conversations.push(new_conv);
            self.conversations.last_mut().unwrap()
        }
    }

    /// Add runtime event
    pub fn add_event(&mut self, conv_id: &str, source: &str, level: &str, message: &str) {
        let conv = self.get_or_create_conversation(conv_id);
        conv.runtime_events.push(PersistentEvent {
            timestamp: Local::now(),
            source: source.to_string(),
            level: level.to_string(),
            message: message.to_string(),
        });
        conv.updated_at = Local::now();
    }

    /// Update model status
    pub fn update_model_status(
        &mut self,
        configured: Option<&str>,
        active: Option<&str>,
        connected: bool,
    ) {
        self.last_model_status = Some(ModelStatus {
            configured: configured.map(|s| s.to_string()),
            active: active.map(|s| s.to_string()),
            ollama_connected: connected,
            verified_at: Local::now(),
        });
    }

    /// Archive a conversation by ID
    pub fn archive_conversation(&mut self, conv_id: &str) -> Result<()> {
        if let Some(idx) = self.conversations.iter().position(|c| c.id == conv_id) {
            self.conversations[idx].archived = true;
            self.conversations[idx].updated_at = Local::now();
            info!("Archived conversation: {}", conv_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Conversation not found: {}", conv_id))
        }
    }

    /// Unarchive a conversation by ID
    pub fn unarchive_conversation(&mut self, conv_id: &str) -> Result<()> {
        if let Some(idx) = self.conversations.iter().position(|c| c.id == conv_id) {
            self.conversations[idx].archived = false;
            self.conversations[idx].updated_at = Local::now();
            info!("Unarchived conversation: {}", conv_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Conversation not found: {}", conv_id))
        }
    }

    /// Get active (non-archived) conversations sorted by updated_at desc
    pub fn active_conversations(&self) -> Vec<&PersistentConversation> {
        let mut convs: Vec<&PersistentConversation> =
            self.conversations.iter().filter(|c| !c.archived).collect();
        convs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        convs
    }

    /// Get archived conversations sorted by updated_at desc
    pub fn archived_conversations(&self) -> Vec<&PersistentConversation> {
        let mut convs: Vec<&PersistentConversation> =
            self.conversations.iter().filter(|c| c.archived).collect();
        convs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        convs
    }
}

impl Default for PersistentState {
    fn default() -> Self {
        Self::new()
    }
}

fn default_conversation_mode() -> String {
    "CHAT".to_string()
}

fn default_execution_state() -> String {
    "IDLE".to_string()
}

fn default_last_action() -> String {
    "none".to_string()
}

fn default_inspector_tab() -> String {
    "Runtime".to_string()
}

// ============================================================================
// V1.6 CHECKPOINT MANAGER: Durable checkpoint storage and validation
// ============================================================================

use std::collections::HashMap;
use std::path::Path;

/// V1.6 CHECKPOINT MANAGER: Manages durable checkpoint storage and validation
pub struct CheckpointManager;

impl CheckpointManager {
    /// Get the chains data directory
    pub fn chains_dir() -> PathBuf {
        PersistentState::data_dir().join("chains")
    }

    /// Get the directory for a specific chain
    pub fn chain_dir(chain_id: &str) -> PathBuf {
        Self::chains_dir().join(chain_id)
    }

    /// Get the checkpoints subdirectory for a chain
    pub fn checkpoints_dir(chain_id: &str) -> PathBuf {
        Self::chain_dir(chain_id).join("checkpoints")
    }

    /// Get the workspace hashes file path for a chain
    pub fn workspace_hashes_path(chain_id: &str) -> PathBuf {
        Self::chain_dir(chain_id).join("workspace_hashes.json")
    }

    /// Get the chain state file path
    pub fn chain_state_path(chain_id: &str) -> PathBuf {
        Self::chain_dir(chain_id).join("chain_state.json")
    }

    /// Ensure chain directory structure exists
    pub async fn ensure_chain_dirs(chain_id: &str) -> Result<()> {
        let chain_dir = Self::chain_dir(chain_id);
        let checkpoints_dir = Self::checkpoints_dir(chain_id);

        tokio::fs::create_dir_all(&chain_dir).await?;
        tokio::fs::create_dir_all(&checkpoints_dir).await?;

        Ok(())
    }

    /// Calculate workspace hash for a set of files
    pub async fn calculate_workspace_hash(
        base_path: &Path,
        files: &[String],
    ) -> Result<WorkspaceHash> {
        let mut file_hashes = Vec::new();
        let mut hasher = blake3::Hasher::new();

        for file_path in files {
            let full_path = base_path.join(file_path);

            let content = tokio::fs::read(&full_path).await?;
            let hash = blake3::hash(&content).to_hex().to_string();
            let size = content.len() as u64;
            let modified: DateTime<Local> =
                tokio::fs::metadata(&full_path).await?.modified()?.into();

            hasher.update(file_path.as_bytes());
            hasher.update(hash.as_bytes());

            file_hashes.push(FileHash {
                path: file_path.clone(),
                hash,
                size,
                modified,
            });
        }

        let overall_hash = hasher.finalize().to_hex().to_string();

        Ok(WorkspaceHash {
            overall_hash,
            file_hashes,
            calculated_at: Local::now(),
            base_path: base_path.to_string_lossy().to_string(),
        })
    }

    /// Save a checkpoint to disk
    pub async fn save_checkpoint(checkpoint: &ExecutionCheckpoint) -> Result<PathBuf> {
        Self::ensure_chain_dirs(&checkpoint.chain_id).await?;

        let checkpoints_dir = Self::checkpoints_dir(&checkpoint.chain_id);
        let path = checkpoints_dir.join(checkpoint.filename());

        let content = serde_json::to_string_pretty(checkpoint)?;
        let temp_path = path.with_extension("tmp");

        tokio::fs::write(&temp_path, content).await?;
        tokio::fs::rename(&temp_path, &path).await?;

        info!("Saved checkpoint: {:?}", path);
        Ok(path)
    }

    /// Load a checkpoint from disk
    pub async fn load_checkpoint(
        chain_id: &str,
        checkpoint_id: &str,
    ) -> Result<ExecutionCheckpoint> {
        let checkpoints_dir = Self::checkpoints_dir(chain_id);
        let path = checkpoints_dir.join(format!("{}.json", checkpoint_id));

        let content = tokio::fs::read_to_string(&path).await?;
        let checkpoint: ExecutionCheckpoint = serde_json::from_str(&content)?;

        Ok(checkpoint)
    }

    /// List all checkpoints for a chain
    pub async fn list_checkpoints(chain_id: &str) -> Vec<ExecutionCheckpoint> {
        let checkpoints_dir = Self::checkpoints_dir(chain_id);
        let mut checkpoints = Vec::new();

        if let Ok(mut entries) = tokio::fs::read_dir(&checkpoints_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "json") {
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        if let Ok(checkpoint) =
                            serde_json::from_str::<ExecutionCheckpoint>(&content)
                        {
                            checkpoints.push(checkpoint);
                        }
                    }
                }
            }
        }

        // Sort by creation time, newest first
        checkpoints.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        checkpoints
    }

    /// Get the latest checkpoint for a chain
    pub async fn get_latest_checkpoint(chain_id: &str) -> Option<ExecutionCheckpoint> {
        let checkpoints = Self::list_checkpoints(chain_id).await;
        checkpoints.into_iter().next()
    }

    /// Build an operator-facing validation report for the latest checkpoint.
    pub async fn inspect_latest_checkpoint(
        chain: &PersistentChain,
        base_path: &Path,
    ) -> CheckpointOperatorReport {
        match Self::get_latest_checkpoint(&chain.id).await {
            Some(checkpoint) => Self::inspect_checkpoint(&checkpoint, chain, base_path).await,
            None => CheckpointOperatorReport::missing(chain.id.clone(), chain.audit_log.len()),
        }
    }

    /// Delete a checkpoint
    pub async fn delete_checkpoint(chain_id: &str, checkpoint_id: &str) -> Result<()> {
        let checkpoints_dir = Self::checkpoints_dir(chain_id);
        let path = checkpoints_dir.join(format!("{}.json", checkpoint_id));

        tokio::fs::remove_file(&path).await?;
        info!("Deleted checkpoint: {:?}", path);

        Ok(())
    }

    /// V1.6 CHECKPOINT: Validate checkpoint against current workspace
    pub async fn validate_checkpoint(
        checkpoint: &ExecutionCheckpoint,
        base_path: &Path,
    ) -> CheckpointValidationResult {
        // Check schema version compatibility
        if checkpoint.schema_version != ExecutionCheckpoint::CURRENT_SCHEMA {
            return CheckpointValidationResult::IncompatibleSchema {
                expected: ExecutionCheckpoint::CURRENT_SCHEMA,
                found: checkpoint.schema_version,
            };
        }

        // Verify workspace hash if files are tracked
        if !checkpoint.tracked_files.is_empty() {
            match Self::calculate_workspace_hash(base_path, &checkpoint.tracked_files).await {
                Ok(current_hash) => {
                    if current_hash.overall_hash != checkpoint.workspace_hash {
                        // Calculate diverged files
                        let mut diverged_files = Vec::new();

                        if checkpoint.tracked_file_hashes.is_empty() {
                            diverged_files.extend(checkpoint.tracked_files.iter().cloned());
                        } else {
                            let checkpoint_files: HashMap<String, String> = checkpoint
                                .tracked_file_hashes
                                .iter()
                                .map(|f| (f.path.clone(), f.hash.clone()))
                                .collect();

                            for file_hash in &current_hash.file_hashes {
                                match checkpoint_files.get(&file_hash.path) {
                                    Some(checkpoint_file) if checkpoint_file != &file_hash.hash => {
                                        diverged_files.push(file_hash.path.clone());
                                    }
                                    Some(_) => {}
                                    None => diverged_files.push(file_hash.path.clone()),
                                }
                            }

                            for path in checkpoint_files.keys() {
                                if !current_hash.file_hashes.iter().any(|f| &f.path == path) {
                                    diverged_files.push(path.clone());
                                }
                            }
                        }

                        return CheckpointValidationResult::StaleWorkspace {
                            checkpoint_hash: checkpoint.workspace_hash.clone(),
                            current_hash: current_hash.overall_hash,
                            diverged_files,
                        };
                    }
                }
                Err(e) => {
                    return CheckpointValidationResult::HashCalculationFailed {
                        error: e.to_string(),
                    };
                }
            }
        }

        // Validate audit cursor consistency (replay to cursor and compare)
        // This is done by the caller who has access to the audit log

        CheckpointValidationResult::Valid
    }

    /// V1.6 CHECKPOINT: Produce canonical operator-facing checkpoint validation truth.
    pub async fn inspect_checkpoint(
        checkpoint: &ExecutionCheckpoint,
        chain: &PersistentChain,
        base_path: &Path,
    ) -> CheckpointOperatorReport {
        let step_description = checkpoint
            .active_step
            .and_then(|idx| chain.steps.get(idx))
            .map(|step| step.description.clone());
        let mut report = CheckpointOperatorReport {
            chain_id: checkpoint.chain_id.clone(),
            checkpoint_id: Some(checkpoint.checkpoint_id.clone()),
            checkpoint_timestamp: Some(checkpoint.created_at),
            active_step: checkpoint.active_step,
            step_description,
            audit_cursor: Some(checkpoint.audit_cursor),
            audit_log_len: chain.audit_log.len(),
            workspace_hash: Some(checkpoint.workspace_hash.clone()),
            workspace_result: CheckpointCheckReport::not_checked("workspace hash not checked"),
            replay_result: CheckpointCheckReport::not_checked("audit cursor not replayed"),
            final_status: CheckpointOperatorStatus::Valid,
            resume_allowed: false,
            smallest_safe_next_action: "/chain resume".to_string(),
        };

        match Self::validate_checkpoint(checkpoint, base_path).await {
            CheckpointValidationResult::Valid => {
                report.workspace_result = CheckpointCheckReport::passed(format!(
                    "{} tracked files match checkpoint hash",
                    checkpoint.tracked_files.len()
                ));
            }
            CheckpointValidationResult::StaleWorkspace {
                checkpoint_hash,
                current_hash,
                diverged_files,
            } => {
                report.workspace_result = CheckpointCheckReport::failed(format!(
                    "workspace hash changed: checkpoint {} current {}; files: {}",
                    short_hash(&checkpoint_hash),
                    short_hash(&current_hash),
                    format_file_list(&diverged_files)
                ));
                report.final_status = CheckpointOperatorStatus::Stale;
                report.smallest_safe_next_action =
                    "Resolve workspace divergence, then create a fresh checkpoint".to_string();
                return report;
            }
            CheckpointValidationResult::Corrupted { error } => {
                report.workspace_result =
                    CheckpointCheckReport::failed(format!("checkpoint corrupted: {}", error));
                report.final_status = CheckpointOperatorStatus::Corrupted;
                report.smallest_safe_next_action =
                    "Delete corrupted checkpoint and use a valid checkpoint".to_string();
                return report;
            }
            CheckpointValidationResult::IncompatibleSchema { expected, found } => {
                report.workspace_result = CheckpointCheckReport::failed(format!(
                    "checkpoint schema mismatch: expected {}, found {}",
                    expected, found
                ));
                report.final_status = CheckpointOperatorStatus::Corrupted;
                report.smallest_safe_next_action =
                    "Restart from a current-schema checkpoint".to_string();
                return report;
            }
            CheckpointValidationResult::HashCalculationFailed { error } => {
                report.workspace_result =
                    CheckpointCheckReport::failed(format!("workspace hash failed: {}", error));
                report.final_status = if looks_like_missing_file(&error) {
                    CheckpointOperatorStatus::Missing
                } else {
                    CheckpointOperatorStatus::Corrupted
                };
                report.smallest_safe_next_action =
                    "Restore missing tracked files or create a fresh checkpoint".to_string();
                return report;
            }
        }

        if !checkpoint.is_resumable() {
            report.replay_result =
                CheckpointCheckReport::not_checked("checkpoint is not marked resumable");
            report.final_status = CheckpointOperatorStatus::Corrupted;
            report.smallest_safe_next_action =
                "Create a validated checkpoint before resuming".to_string();
            return report;
        }

        let audit_events = chain.audit_log.get_all_events();
        if checkpoint.audit_cursor > audit_events.len() {
            report.replay_result = CheckpointCheckReport::failed(format!(
                "audit cursor {} exceeds audit log length {}",
                checkpoint.audit_cursor,
                audit_events.len()
            ));
            report.final_status = CheckpointOperatorStatus::Divergent;
            report.smallest_safe_next_action =
                "/audit replay, then restart from a valid checkpoint".to_string();
            return report;
        }

        let replay = chain
            .audit_log
            .replay_to_cursor(crate::state::ExecutionState::Idle, checkpoint.audit_cursor);
        if replay.final_state != checkpoint.execution_state {
            report.replay_result = CheckpointCheckReport::failed(format!(
                "cursor replay state {:?} does not match checkpoint state {:?}",
                replay.final_state, checkpoint.execution_state
            ));
            report.final_status = CheckpointOperatorStatus::Divergent;
            report.smallest_safe_next_action =
                "/audit replay, then restart from a valid checkpoint".to_string();
            return report;
        }

        if !replay.replay_is_deterministic() {
            report.replay_result = CheckpointCheckReport::failed(format!(
                "cursor replay emitted {} warning(s)",
                replay.warnings.len()
            ));
            report.final_status = CheckpointOperatorStatus::Divergent;
            report.smallest_safe_next_action =
                "/audit replay, then create a fresh checkpoint".to_string();
            return report;
        }

        report.replay_result = CheckpointCheckReport::passed(format!(
            "audit events 0..{} replay to {:?}",
            checkpoint.audit_cursor, checkpoint.execution_state
        ));

        if checkpoint.lifecycle_status.is_terminal() && checkpoint.active_step.is_none() {
            report.resume_allowed = false;
            report.smallest_safe_next_action = "Chain is terminal; start a new chain".to_string();
            return report;
        }

        report.resume_allowed = true;
        report.smallest_safe_next_action = "/chain resume".to_string();
        report
    }

    /// V1.6 CHECKPOINT: Resume from a validated checkpoint
    pub async fn resume_from_checkpoint(
        checkpoint: &ExecutionCheckpoint,
        chain: &PersistentChain,
        base_path: &Path,
    ) -> CheckpointResumeResult {
        let report = Self::inspect_checkpoint(checkpoint, chain, base_path).await;
        match report.final_status {
            CheckpointOperatorStatus::Valid if report.resume_allowed => {
                CheckpointResumeResult::Success {
                    chain_id: checkpoint.chain_id.clone(),
                    resumed_step: checkpoint.active_step.unwrap_or(0),
                    message: format!(
                        "Resumed from checkpoint {} at step {:?}",
                        checkpoint.checkpoint_id, checkpoint.active_step
                    ),
                }
            }
            CheckpointOperatorStatus::Stale => CheckpointResumeResult::Stale {
                checkpoint_hash: checkpoint.workspace_hash.clone(),
                current_hash: match Self::validate_checkpoint(checkpoint, base_path).await {
                    CheckpointValidationResult::StaleWorkspace { current_hash, .. } => current_hash,
                    _ => report.workspace_result.detail.clone(),
                },
                diverged_files: match Self::validate_checkpoint(checkpoint, base_path).await {
                    CheckpointValidationResult::StaleWorkspace { diverged_files, .. } => {
                        diverged_files
                    }
                    _ => vec![report.workspace_result.detail.clone()],
                },
            },
            CheckpointOperatorStatus::Corrupted => CheckpointResumeResult::Corrupted {
                path: Self::checkpoints_dir(&checkpoint.chain_id)
                    .join(checkpoint.filename())
                    .to_string_lossy()
                    .to_string(),
                error: report.workspace_result.detail,
            },
            CheckpointOperatorStatus::Divergent => CheckpointResumeResult::Divergent {
                checkpoint_state: format!("{:?}", checkpoint.execution_state),
                replayed_state: format!(
                    "{:?}",
                    chain
                        .audit_log
                        .replay_to_cursor(
                            crate::state::ExecutionState::Idle,
                            checkpoint.audit_cursor
                        )
                        .final_state
                ),
                audit_event_index: checkpoint.audit_cursor,
            },
            CheckpointOperatorStatus::Missing | CheckpointOperatorStatus::Valid => {
                CheckpointResumeResult::Blocked {
                    reason: report.replay_result.detail,
                    recovery_action: report.smallest_safe_next_action,
                }
            }
        }
    }

    /// V1.6 CHECKPOINT: Create checkpoint at validated boundary
    pub async fn create_validated_checkpoint(
        chain: &PersistentChain,
        base_path: &Path,
        source: CheckpointSource,
        message: Option<String>,
    ) -> Result<ExecutionCheckpoint> {
        // Use sync version to avoid recursive async issues
        let tracked_files =
            tokio::task::block_in_place(|| Self::discover_trackable_files_sync(base_path))
                .map_err(|e| anyhow::anyhow!("Failed to discover trackable files: {}", e))?;
        let workspace_hash = Self::calculate_workspace_hash(base_path, &tracked_files).await?;

        let audit_cursor = chain.audit_log.len();
        let execution_state = chain.status.to_execution_state();

        let mut checkpoint = ExecutionCheckpoint::new(
            chain.id.clone(),
            chain
                .steps
                .iter()
                .position(|step| matches!(step.status, ChainStepStatus::Pending)),
            chain.status,
            chain.get_outcome(),
            execution_state,
            audit_cursor,
            workspace_hash.overall_hash.clone(),
            tracked_files,
            source,
            message,
        )
        .with_tracked_file_hashes(workspace_hash.file_hashes);
        checkpoint.mark_valid();

        Self::save_checkpoint(&checkpoint).await?;

        info!(
            "Created validated checkpoint {} for chain {} at audit cursor {}",
            checkpoint.checkpoint_id, chain.id, audit_cursor
        );

        Ok(checkpoint)
    }

    /// Discover files that should be tracked for workspace integrity
    fn discover_trackable_files_sync(base_path: &Path) -> Result<Vec<String>> {
        let mut files = Vec::new();

        // Use std::fs for synchronous directory walking (avoids recursive async)
        let entries = std::fs::read_dir(base_path)?;

        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = entry.metadata().ok();

            if let Some(metadata) = metadata {
                if metadata.is_file() {
                    let relative = path.strip_prefix(base_path).unwrap_or(&path);
                    let relative_str = relative.to_string_lossy().to_string();

                    // Skip certain file types
                    if Self::should_track_file(&relative_str) {
                        files.push(relative_str);
                    }
                } else if metadata.is_dir() {
                    // Recursively scan directories, but skip hidden and target
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                    if !dir_name.starts_with('.')
                        && dir_name != "target"
                        && dir_name != "node_modules"
                    {
                        let sub_files = Self::discover_trackable_files_sync(&path)?;
                        for f in sub_files {
                            files.push(format!("{}/{}", dir_name, f));
                        }
                    }
                }
            }
        }

        Ok(files)
    }

    /// Check if a file should be tracked for integrity
    fn should_track_file(path: &str) -> bool {
        let track_extensions = [
            "rs",
            "py",
            "js",
            "ts",
            "json",
            "toml",
            "yaml",
            "yml",
            "md",
            "txt",
            "sh",
            "Dockerfile",
            "Makefile",
        ];

        let path_lower = path.to_lowercase();

        // Skip certain patterns
        if path_lower.starts_with("target/")
            || path_lower.starts_with("node_modules/")
            || path_lower.starts_with(".git/")
            || path_lower.starts_with(".cache/")
            || path_lower.ends_with(".lock")
            || path_lower.ends_with(".tmp")
        {
            return false;
        }

        // Check if it has a trackable extension
        track_extensions.iter().any(|ext| {
            path_lower.ends_with(&format!(".{}", ext)) || path_lower.contains(&format!(".{}.", ext))
        })
    }
}

/// V1.6 CHECKPOINT: Result of checkpoint validation
#[derive(Debug, Clone)]
pub enum CheckpointValidationResult {
    /// Checkpoint is valid and ready for resume
    Valid,
    /// Checkpoint has stale workspace (files changed)
    StaleWorkspace {
        checkpoint_hash: String,
        current_hash: String,
        diverged_files: Vec<String>,
    },
    /// Checkpoint file is corrupted
    Corrupted { error: String },
    /// Schema version incompatible
    IncompatibleSchema { expected: u32, found: u32 },
    /// Failed to calculate workspace hash
    HashCalculationFailed { error: String },
}

fn short_hash(value: &str) -> String {
    value.chars().take(8).collect()
}

fn format_file_list(files: &[String]) -> String {
    if files.is_empty() {
        "none reported".to_string()
    } else if files.len() <= 5 {
        files.join(", ")
    } else {
        format!(
            "{}, ... and {} more",
            files[..5].join(", "),
            files.len() - 5
        )
    }
}

fn looks_like_missing_file(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("no such file")
        || lower.contains("not found")
        || lower.contains("cannot find")
        || lower.contains("missing")
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    use crate::state::{
        AuditEvent, AuditEventType, ExecutionState, ProgressTransitionEvent,
        reduce_execution_state_with_audit,
    };
    use std::fs;

    fn test_chain(audit_log: crate::state::AuditLog) -> PersistentChain {
        PersistentChain {
            id: "chain-test".to_string(),
            name: "test".to_string(),
            objective: "objective".to_string(),
            raw_prompt: "objective".to_string(),
            status: ChainLifecycleStatus::Running,
            steps: vec![
                PersistentChainStep {
                    id: "step-1".to_string(),
                    description: "done".to_string(),
                    status: ChainStepStatus::Completed,
                    retry_of: None,
                    retry_attempt: 0,
                    execution_outcome: Some(ExecutionOutcome::Success),
                    execution_result_class: None,
                    execution_results: vec![],
                    failure_reason: None,
                    recovery_step_kind: None,
                    evidence_snapshot: None,
                    force_override_used: false,
                    tool_calls: vec![],
                    result_summary: None,
                    validation_passed: Some(true),
                    started_at: None,
                    completed_at: None,
                    error_message: None,
                    replay_record: None,
                },
                PersistentChainStep {
                    id: "step-2".to_string(),
                    description: "next".to_string(),
                    status: ChainStepStatus::Pending,
                    retry_of: None,
                    retry_attempt: 0,
                    execution_outcome: None,
                    execution_result_class: None,
                    execution_results: vec![],
                    failure_reason: None,
                    recovery_step_kind: None,
                    evidence_snapshot: None,
                    force_override_used: false,
                    tool_calls: vec![],
                    result_summary: None,
                    validation_passed: None,
                    started_at: None,
                    completed_at: None,
                    error_message: None,
                    replay_record: None,
                },
            ],
            active_step: Some(1),
            repo_path: None,
            conversation_id: None,
            created_at: Local::now(),
            updated_at: Local::now(),
            completed_at: None,
            archived: false,
            total_steps_executed: 1,
            total_steps_failed: 0,
            execution_outcome: None,
            force_override_used: false,
            objective_satisfaction: ObjectiveSatisfaction::default(),
            selected_context_files: vec!["tracked.txt".to_string()],
            context_state: None,
            pending_checkpoint: None,
            git_grounding: None,
            audit_log,
        }
    }

    fn append_transition(
        audit_log: &mut crate::state::AuditLog,
        current: ExecutionState,
        event: ProgressTransitionEvent,
    ) -> ExecutionState {
        let (result, audit_event) = reduce_execution_state_with_audit(current, event, false);
        audit_log.append(audit_event.expect("transition audit event"));
        match result {
            crate::state::TransitionResult::Applied(next)
            | crate::state::TransitionResult::Normalized { to: next, .. } => next,
            crate::state::TransitionResult::Rejected { current, .. } => current,
        }
    }

    #[tokio::test]
    async fn checkpoint_workspace_hash_detects_changed_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file_path = dir.path().join("tracked.txt");
        fs::write(&file_path, "before").expect("write fixture");

        let tracked = vec!["tracked.txt".to_string()];
        let initial = CheckpointManager::calculate_workspace_hash(dir.path(), &tracked)
            .await
            .expect("initial hash");

        let mut checkpoint = ExecutionCheckpoint::new(
            "chain-test".to_string(),
            Some(0),
            ChainLifecycleStatus::Running,
            None,
            ExecutionState::Executing,
            0,
            initial.overall_hash,
            tracked,
            CheckpointSource::AutoValidatedStep,
            None,
        )
        .with_tracked_file_hashes(initial.file_hashes);
        checkpoint.mark_valid();

        fs::write(&file_path, "after").expect("modify fixture");

        let validation = CheckpointManager::validate_checkpoint(&checkpoint, dir.path()).await;
        match validation {
            CheckpointValidationResult::StaleWorkspace { diverged_files, .. } => {
                assert_eq!(diverged_files, vec!["tracked.txt".to_string()]);
            }
            other => panic!("expected stale workspace, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn resume_replays_only_to_checkpoint_cursor() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("tracked.txt"), "stable").expect("write fixture");
        let tracked = vec!["tracked.txt".to_string()];
        let workspace = CheckpointManager::calculate_workspace_hash(dir.path(), &tracked)
            .await
            .expect("workspace hash");

        let mut audit_log = crate::state::AuditLog::new();
        let mut state = ExecutionState::Idle;
        state = append_transition(
            &mut audit_log,
            state,
            ProgressTransitionEvent::NewRun {
                task: "task".to_string(),
            },
        );
        state = append_transition(
            &mut audit_log,
            state,
            ProgressTransitionEvent::ToolExecuting {
                name: "tool".to_string(),
            },
        );
        assert_eq!(state, ExecutionState::Executing);
        let cursor = audit_log.len();
        let _done = append_transition(
            &mut audit_log,
            state,
            ProgressTransitionEvent::RuntimeFinished { success: true },
        );
        audit_log.append(AuditEvent::state_transition(
            AuditEventType::StateTransitionRejected,
            ExecutionState::Done,
            ExecutionState::Done,
            &ProgressTransitionEvent::ToolExecuting {
                name: "late".to_string(),
            },
            Some("post checkpoint noise"),
        ));

        let chain = test_chain(audit_log);
        let mut checkpoint = ExecutionCheckpoint::new(
            "chain-test".to_string(),
            Some(1),
            ChainLifecycleStatus::Running,
            None,
            ExecutionState::Executing,
            cursor,
            workspace.overall_hash,
            tracked,
            CheckpointSource::AutoValidatedStep,
            None,
        )
        .with_tracked_file_hashes(workspace.file_hashes);
        checkpoint.mark_valid();

        let result =
            CheckpointManager::resume_from_checkpoint(&checkpoint, &chain, dir.path()).await;
        assert!(matches!(
            result,
            CheckpointResumeResult::Success {
                resumed_step: 1,
                ..
            }
        ));
    }
}
