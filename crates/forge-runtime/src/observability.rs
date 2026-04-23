//! Observability Layer - Execution Transparency and Debugging
//!
//! Provides execution timeline, failure explanations, replay inspection,
//! tool tracing, and state change visibility for developer debugging.
//! Per SPRINT: Runtime Observability + Debugging Layer

use crate::chain_executor::ChainEvent;
use crate::types::{ChainStatus, ValidationDecision};
use serde::{Deserialize, Serialize};

// ============================================================================
// DELIVERABLE 1: Structured Execution Timeline
// ============================================================================

/// Phase of execution for timeline entries
/// Every major runtime event maps to exactly one timeline phase
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelinePhase {
    /// Runtime initialization
    RuntimeInit,
    /// Chain execution starting
    ChainStart,
    /// Individual step starting
    StepStart,
    /// Input sent to planner
    PlannerInput,
    /// Planner output classification
    PlannerDecision,
    /// Tool execution permission check
    ToolGateCheck,
    /// Tool actually executing
    ToolExecution,
    /// Detecting state mutations
    MutationDetection,
    /// Validation starting
    ValidationStart,
    /// Individual validation stage
    ValidationStage,
    /// Validation result recorded
    ValidationResult,
    /// Changes committed
    Commit,
    /// Changes reverted
    Revert,
    /// Checkpoint saved to disk
    CheckpointSaved,
    /// Resume guard check
    ResumeGuard,
    /// Replay verification check
    ReplayCheck,
    /// Planning phase
    Planning,
    /// Step execution
    Execution,
    /// Validation phase (detailed)
    Validation,
    /// Completion phase
    Completion,
    /// Chain lifecycle event
    ChainLifecycle,
    /// Checkpoint event
    Checkpoint,
    /// Runtime completed successfully
    RuntimeComplete,
    /// Runtime failed
    RuntimeFailure,
}

impl std::fmt::Display for TimelinePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RuntimeInit => write!(f, "runtime_init"),
            Self::ChainStart => write!(f, "chain_start"),
            Self::StepStart => write!(f, "step_start"),
            Self::PlannerInput => write!(f, "planner_input"),
            Self::PlannerDecision => write!(f, "planner_decision"),
            Self::ToolGateCheck => write!(f, "tool_gate_check"),
            Self::ToolExecution => write!(f, "tool_execution"),
            Self::MutationDetection => write!(f, "mutation_detection"),
            Self::ValidationStart => write!(f, "validation_start"),
            Self::ValidationStage => write!(f, "validation_stage"),
            Self::ValidationResult => write!(f, "validation_result"),
            Self::Commit => write!(f, "commit"),
            Self::Revert => write!(f, "revert"),
            Self::CheckpointSaved => write!(f, "checkpoint_saved"),
            Self::ResumeGuard => write!(f, "resume_guard"),
            Self::ReplayCheck => write!(f, "replay_check"),
            Self::Planning => write!(f, "planning"),
            Self::Execution => write!(f, "execution"),
            Self::Validation => write!(f, "validation"),
            Self::Completion => write!(f, "completion"),
            Self::ChainLifecycle => write!(f, "chain_lifecycle"),
            Self::Checkpoint => write!(f, "checkpoint"),
            Self::RuntimeComplete => write!(f, "runtime_complete"),
            Self::RuntimeFailure => write!(f, "runtime_failure"),
        }
    }
}

/// Status of a timeline entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineStatus {
    /// Entry is pending (not yet started)
    Pending,
    /// Entry is currently running
    Running,
    /// Entry completed successfully
    Completed,
    /// Entry failed
    Failed,
    /// Entry was skipped
    Skipped,
    /// Entry was blocked (waiting for external action)
    Blocked,
}

impl std::fmt::Display for TimelineStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
            Self::Blocked => write!(f, "blocked"),
        }
    }
}

/// Outcome of a complete timeline run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineOutcome {
    /// Run completed successfully
    Success,
    /// Run failed
    Failure { reason: String },
    /// Run was cancelled
    Cancelled,
    /// Run is still in progress
    InProgress,
}

/// Single entry in the execution timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    /// Sequential index in the timeline
    pub index: u32,
    /// Phase of execution
    pub phase: TimelinePhase,
    /// Status of this entry
    pub status: TimelineStatus,
    /// Brief human-readable summary
    pub summary: String,
    /// Detailed explanation (optional)
    pub detail: Option<String>,
    /// Related chain step index (if applicable)
    pub related_step: Option<u32>,
    /// Related tool name (if applicable)
    pub related_tool: Option<String>,
    /// Related validation stage (if applicable)
    pub related_validation_stage: Option<String>,
    /// Timestamp (Unix millis)
    pub timestamp: u64,
    /// Duration in milliseconds (if completed)
    pub duration_ms: Option<u64>,
}

impl TimelineEntry {
    /// Create a new timeline entry
    pub fn new(
        index: u32,
        phase: TimelinePhase,
        status: TimelineStatus,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            index,
            phase,
            status,
            summary: summary.into(),
            detail: None,
            related_step: None,
            related_tool: None,
            related_validation_stage: None,
            timestamp: crate::types::timestamp_now(),
            duration_ms: None,
        }
    }

    /// Add detail to this entry
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Link to a chain step
    pub fn with_step(mut self, step: u32) -> Self {
        self.related_step = Some(step);
        self
    }

    /// Link to a tool
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.related_tool = Some(tool.into());
        self
    }

    /// Link to a validation stage
    pub fn with_validation_stage(mut self, stage: impl Into<String>) -> Self {
        self.related_validation_stage = Some(stage.into());
        self
    }

    /// Mark entry as completed with duration
    pub fn completed(mut self, duration_ms: u64) -> Self {
        self.status = TimelineStatus::Completed;
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Mark entry as failed
    pub fn failed(mut self, reason: impl Into<String>) -> Self {
        self.status = TimelineStatus::Failed;
        self.detail = Some(reason.into());
        self
    }
}

/// Complete execution timeline for a run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTimeline {
    /// Unique run identifier
    pub run_id: String,
    /// Task description
    pub task: String,
    /// Chronological timeline entries
    pub entries: Vec<TimelineEntry>,
    /// Start timestamp (Unix millis)
    pub started_at: u64,
    /// End timestamp (if finished)
    pub finished_at: Option<u64>,
    /// Final outcome
    pub outcome: TimelineOutcome,
    /// Chain status (if chain execution)
    pub chain_status: Option<ChainStatus>,
}

impl ExecutionTimeline {
    /// Create a new timeline for a task
    pub fn new(run_id: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            task: task.into(),
            entries: Vec::new(),
            started_at: crate::types::timestamp_now(),
            finished_at: None,
            outcome: TimelineOutcome::InProgress,
            chain_status: None,
        }
    }

    /// Add an entry to the timeline
    pub fn add_entry(&mut self, entry: TimelineEntry) {
        self.entries.push(entry);
    }

    /// Mark timeline as completed
    pub fn complete_success(&mut self) {
        self.finished_at = Some(crate::types::timestamp_now());
        self.outcome = TimelineOutcome::Success;
    }

    /// Mark timeline as failed
    pub fn complete_failure(&mut self, reason: impl Into<String>) {
        self.finished_at = Some(crate::types::timestamp_now());
        self.outcome = TimelineOutcome::Failure {
            reason: reason.into(),
        };
    }

    /// Get entries for a specific phase
    pub fn entries_for_phase(&self, phase: TimelinePhase) -> Vec<&TimelineEntry> {
        self.entries.iter().filter(|e| e.phase == phase).collect()
    }

    /// Get entries for a specific step
    pub fn entries_for_step(&self, step: u32) -> Vec<&TimelineEntry> {
        self.entries
            .iter()
            .filter(|e| e.related_step == Some(step))
            .collect()
    }

    /// Get failed entries
    pub fn failed_entries(&self) -> Vec<&TimelineEntry> {
        self.entries
            .iter()
            .filter(|e| e.status == TimelineStatus::Failed)
            .collect()
    }

    /// Get total duration (if finished)
    pub fn total_duration_ms(&self) -> Option<u64> {
        match (self.started_at, self.finished_at) {
            (_, Some(finished)) => Some(finished - self.started_at),
            _ => None,
        }
    }

    /// Export timeline as JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

// ============================================================================
// Timeline Builder - Construct timeline from chain events
// ============================================================================

/// Builder for constructing timelines from chain execution
pub struct TimelineBuilder;

impl TimelineBuilder {
    /// Build timeline from chain events
    pub fn from_chain_events(run_id: &str, task: &str, events: &[ChainEvent]) -> ExecutionTimeline {
        let mut timeline = ExecutionTimeline::new(run_id, task);
        let mut entry_index = 0u32;

        for event in events {
            let entry = Self::event_to_entry(event, entry_index);
            timeline.add_entry(entry);
            entry_index += 1;
        }

        // Set final outcome based on last events
        if let Some(last) = events.last() {
            match last {
                ChainEvent::ChainCompleted { .. } => timeline.complete_success(),
                ChainEvent::ChainFailed { .. } => {
                    timeline.complete_failure("Chain execution failed")
                }
                _ => {}
            }
        }

        timeline
    }

    /// Convert a chain event to timeline entry
    fn event_to_entry(event: &ChainEvent, index: u32) -> TimelineEntry {
        match event {
            ChainEvent::ChainCreated {
                chain_id,
                objective,
                step_count,
            } => TimelineEntry::new(
                index,
                TimelinePhase::ChainLifecycle,
                TimelineStatus::Completed,
                format!("Chain '{}' created with {} steps", chain_id, step_count),
            )
            .with_detail(format!("Objective: {}", objective)),
            ChainEvent::StepStarted {
                step_index,
                description,
            } => TimelineEntry::new(
                index,
                TimelinePhase::Execution,
                TimelineStatus::Running,
                format!("Step {} started", step_index),
            )
            .with_step(*step_index as u32)
            .with_detail(description.clone()),
            ChainEvent::StepValidated {
                step_index,
                decision,
            } => {
                let (status, summary) = match decision {
                    ValidationDecision::Accept => {
                        (TimelineStatus::Completed, "Validation passed".to_string())
                    }
                    ValidationDecision::Reject => {
                        (TimelineStatus::Failed, "Validation rejected".to_string())
                    }
                    ValidationDecision::Escalate => {
                        (TimelineStatus::Failed, "Validation escalated".to_string())
                    }
                };
                TimelineEntry::new(index, TimelinePhase::Validation, status, summary)
                    .with_step(*step_index as u32)
            }
            ChainEvent::StepCompleted {
                step_index,
                outcome_summary,
            } => TimelineEntry::new(
                index,
                TimelinePhase::Completion,
                TimelineStatus::Completed,
                format!("Step {} completed", step_index),
            )
            .with_step(*step_index as u32)
            .with_detail(outcome_summary.clone()),
            ChainEvent::StepFailed { step_index, reason } => TimelineEntry::new(
                index,
                TimelinePhase::Completion,
                TimelineStatus::Failed,
                format!("Step {} failed", step_index),
            )
            .with_step(*step_index as u32)
            .with_detail(reason.clone()),
            ChainEvent::ChainAdvanced { from_step, to_step } => TimelineEntry::new(
                index,
                TimelinePhase::ChainLifecycle,
                TimelineStatus::Completed,
                format!("Advanced from step {} to {}", from_step, to_step),
            ),
            ChainEvent::CheckpointSaved { step_index, .. } => TimelineEntry::new(
                index,
                TimelinePhase::Checkpoint,
                TimelineStatus::Completed,
                format!("Checkpoint saved at step {}", step_index),
            )
            .with_step(*step_index as u32),
            ChainEvent::ChainCompleted { .. } => TimelineEntry::new(
                index,
                TimelinePhase::ChainLifecycle,
                TimelineStatus::Completed,
                "Chain execution completed",
            ),
            ChainEvent::ChainFailed {
                at_step, reason, ..
            } => TimelineEntry::new(
                index,
                TimelinePhase::ChainLifecycle,
                TimelineStatus::Failed,
                format!("Chain failed at step {}", at_step),
            )
            .with_step(*at_step as u32)
            .with_detail(reason.clone()),
        }
    }
}

// ============================================================================
// DELIVERABLE 2: Failure Explainer - Readable Error Messages
// ============================================================================

/// Classification of failures for user-friendly explanations
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    /// Validation stage failed (syntax, format, lint, build, test)
    ValidationFailed,
    /// Tool execution failed
    ToolExecutionFailed,
    /// Planner produced invalid output
    PlannerError,
    /// Chain execution error
    ChainError,
    /// State integrity issue
    StateError,
    /// Timeout or resource limit
    ResourceLimit,
    /// Unknown/unexpected failure
    Unknown,
}

impl std::fmt::Display for FailureCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ValidationFailed => write!(f, "Validation Failed"),
            Self::ToolExecutionFailed => write!(f, "Tool Execution Failed"),
            Self::PlannerError => write!(f, "Planner Error"),
            Self::ChainError => write!(f, "Chain Execution Error"),
            Self::StateError => write!(f, "State Integrity Error"),
            Self::ResourceLimit => write!(f, "Resource Limit Reached"),
            Self::Unknown => write!(f, "Unknown Error"),
        }
    }
}

/// Explanation of a failure with actionable remediation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureExplanation {
    /// Category of the failure
    pub category: FailureCategory,
    /// Human-readable headline
    pub headline: String,
    /// Detailed explanation of what happened
    pub explanation: String,
    /// What was the user trying to do
    pub context: String,
    /// Specific remediation steps
    pub remediation: Vec<String>,
    /// Related files or tools
    pub related_items: Vec<String>,
}

impl FailureExplanation {
    /// Create a new explanation
    pub fn new(
        category: FailureCategory,
        headline: impl Into<String>,
        explanation: impl Into<String>,
    ) -> Self {
        Self {
            category,
            headline: headline.into(),
            explanation: explanation.into(),
            context: String::new(),
            remediation: Vec::new(),
            related_items: Vec::new(),
        }
    }

    /// Add context about what the user was doing
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }

    /// Add a remediation step
    pub fn add_remediation(mut self, step: impl Into<String>) -> Self {
        self.remediation.push(step.into());
        self
    }

    /// Add related item (file, tool, etc.)
    pub fn add_related(mut self, item: impl Into<String>) -> Self {
        self.related_items.push(item.into());
        self
    }

    /// Format as readable text for CLI output
    pub fn format_cli(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("\n❌ {}\n", self.headline));
        output.push_str(&format!("   Category: {}\n", self.category));
        output.push('\n');

        if !self.context.is_empty() {
            output.push_str(&format!("Context: {}\n", self.context));
            output.push('\n');
        }

        output.push_str(&format!("What happened: {}\n", self.explanation));
        output.push('\n');

        if !self.remediation.is_empty() {
            output.push_str("How to fix:\n");
            for (i, step) in self.remediation.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", i + 1, step));
            }
            output.push('\n');
        }

        if !self.related_items.is_empty() {
            output.push_str("Related: ");
            output.push_str(&self.related_items.join(", "));
            output.push('\n');
        }

        output
    }
}

/// Explains failures in user-friendly terms
pub struct FailureExplainer;

impl FailureExplainer {
    /// Explain a validation failure
    pub fn explain_validation_failure(
        stage: &str,
        _reason: &str,
        file: Option<&str>,
    ) -> FailureExplanation {
        let category = FailureCategory::ValidationFailed;

        let headline = format!("{} validation failed", stage);

        let explanation = format!(
            "The {} stage detected an issue that prevents the code from being accepted. \
             This is a safety feature to ensure only working code is committed.",
            stage
        );

        let mut explanation = FailureExplanation::new(category, headline, explanation);

        if let Some(f) = file {
            explanation = explanation
                .with_context(format!("Validating changes to {}", f))
                .add_related(f);
        }

        explanation = explanation
            .add_remediation(format!("Review the {} error message above", stage))
            .add_remediation("Fix the reported issue in your code")
            .add_remediation("The task will retry automatically");

        explanation
    }

    /// Explain a tool execution failure
    pub fn explain_tool_failure(tool_name: &str, _error: &str) -> FailureExplanation {
        let category = FailureCategory::ToolExecutionFailed;
        let headline = format!("Tool '{}' failed", tool_name);

        let explanation = format!(
            "The {} tool encountered an error during execution. \
             This could be due to invalid arguments, missing files, or system issues.",
            tool_name
        );

        FailureExplanation::new(category, headline, explanation)
            .with_context(format!("Executing tool: {}", tool_name))
            .add_remediation("Check that all file paths are correct")
            .add_remediation("Verify the tool arguments are valid")
            .add_remediation("Review the error details above")
            .add_related(tool_name)
    }

    /// Explain a planner error
    pub fn explain_planner_error(error_type: &str, _details: &str) -> FailureExplanation {
        let category = FailureCategory::PlannerError;
        let headline = format!("Planner {} error", error_type);

        let explanation = format!(
            "The AI planner produced {} output that couldn't be processed. \
             This is usually a temporary issue with the model response format.",
            error_type
        );

        FailureExplanation::new(category, headline, explanation)
            .with_context("The AI was generating a response to your task")
            .add_remediation("The system will automatically retry with corrections")
            .add_remediation("If this persists, try rephrasing your task")
    }

    /// Explain a chain execution failure
    pub fn explain_chain_failure(step: usize, _reason: &str) -> FailureExplanation {
        let category = FailureCategory::ChainError;
        let headline = format!("Task chain failed at step {}", step + 1);

        let explanation = format!(
            "The multi-step task could not complete because step {} encountered an error. \
             All changes from this step have been reverted to maintain consistency.",
            step + 1
        );

        FailureExplanation::new(category, headline, explanation)
            .with_context(format!("Executing step {} of the task chain", step + 1))
            .add_remediation("Review the error from the failed step above")
            .add_remediation("You can resume from the last successful checkpoint")
            .add_remediation("Consider breaking the task into smaller steps")
    }

    /// Explain a timeout
    pub fn explain_timeout(operation: &str, limit_ms: u64) -> FailureExplanation {
        let category = FailureCategory::ResourceLimit;
        let headline = format!("{} timed out", operation);

        let seconds = limit_ms / 1000;
        let explanation = format!(
            "The {} operation exceeded the {} second time limit. \
             This prevents runaway processes from consuming resources indefinitely.",
            operation, seconds
        );

        FailureExplanation::new(category, headline, explanation)
            .add_remediation("The operation may be too complex - try a simpler approach")
            .add_remediation("Break the task into smaller steps")
            .add_remediation("Check for infinite loops or excessive file operations")
    }

    /// Explain unknown/unexpected failure
    pub fn explain_unknown(error: &str) -> FailureExplanation {
        let category = FailureCategory::Unknown;
        let headline = "Unexpected error occurred";

        let explanation = format!(
            "An unexpected error occurred: {}. \
             This may be a bug or an edge case we haven't handled yet.",
            error
        );

        FailureExplanation::new(category, headline, explanation)
            .add_remediation("Try the operation again")
            .add_remediation("If this persists, report the issue with the error details")
    }
}

// ============================================================================
// DELIVERABLE 3: Step Diff + State Change Inspection
// ============================================================================

/// Status of a mutation in a step
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationStatus {
    /// Mutation was attempted but not yet committed
    Attempted,
    /// Mutation was committed successfully
    Committed,
    /// Mutation was reverted
    Reverted,
    /// Mutation was blocked
    Blocked,
}

/// Summary of a single mutation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationSummary {
    /// Path that was mutated
    pub path: String,
    /// Type of mutation (write, patch, delete)
    pub mutation_type: String,
    /// Fingerprint before mutation
    pub before_fingerprint: Option<String>,
    /// Fingerprint after mutation
    pub after_fingerprint: Option<String>,
    /// Current status
    pub status: MutationStatus,
}

/// Summary of all mutations in a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepMutationSummary {
    /// Step index
    pub step_index: u32,
    /// All mutations that were attempted
    pub attempted_mutations: Vec<MutationSummary>,
    /// Mutations that were committed
    pub committed_mutations: Vec<MutationSummary>,
    /// Mutations that were reverted
    pub reverted_mutations: Vec<MutationSummary>,
}

impl StepMutationSummary {
    /// Create a new summary for a step
    pub fn new(step_index: u32) -> Self {
        Self {
            step_index,
            attempted_mutations: Vec::new(),
            committed_mutations: Vec::new(),
            reverted_mutations: Vec::new(),
        }
    }

    /// Add an attempted mutation
    pub fn add_attempted(&mut self, mutation: MutationSummary) {
        self.attempted_mutations.push(mutation);
    }

    /// Mark a mutation as committed
    pub fn mark_committed(&mut self, path: &str) {
        if let Some(m) = self.attempted_mutations.iter_mut().find(|m| m.path == path) {
            m.status = MutationStatus::Committed;
            self.committed_mutations.push(m.clone());
        }
    }

    /// Mark a mutation as reverted
    pub fn mark_reverted(&mut self, path: &str) {
        if let Some(m) = self.attempted_mutations.iter_mut().find(|m| m.path == path) {
            m.status = MutationStatus::Reverted;
            self.reverted_mutations.push(m.clone());
        }
    }
}

/// State fingerprint summary at a step boundary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStateSummary {
    /// Step index
    pub step_index: u32,
    /// Fingerprint before this step
    pub previous_state_fingerprint: Option<String>,
    /// Fingerprint after this step
    pub current_state_fingerprint: String,
    /// Names of fields that changed
    pub changed_fields: Vec<String>,
}

// ============================================================================
// DELIVERABLE 4: Planner → Runtime Trace View
// ============================================================================

/// Trace of planner output processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerTrace {
    /// Step index
    pub step_index: u32,
    /// Bounded excerpt of raw planner output
    pub raw_output_excerpt: String,
    /// Classification result
    pub classification: String,
    /// Tool that was accepted (if any)
    pub accepted_tool: Option<String>,
    /// Rejection reason (if blocked)
    pub rejected_reason: Option<String>,
    /// Whether repair was attempted
    pub repair_attempted: bool,
    /// Outcome of repair (if attempted)
    pub repair_outcome: Option<String>,
}

impl PlannerTrace {
    /// Create a new planner trace
    pub fn new(step_index: u32, raw_output: &str) -> Self {
        // Bound the raw output excerpt to avoid huge traces
        let excerpt = if raw_output.len() > 500 {
            format!("{}... (truncated)", &raw_output[..500])
        } else {
            raw_output.to_string()
        };

        Self {
            step_index,
            raw_output_excerpt: excerpt,
            classification: String::new(),
            accepted_tool: None,
            rejected_reason: None,
            repair_attempted: false,
            repair_outcome: None,
        }
    }

    /// Mark as accepted tool call
    pub fn accepted(mut self, tool: impl Into<String>) -> Self {
        self.classification = "accepted_tool_call".to_string();
        self.accepted_tool = Some(tool.into());
        self
    }

    /// Mark as rejected
    pub fn rejected(mut self, reason: impl Into<String>) -> Self {
        self.classification = "rejected".to_string();
        self.rejected_reason = Some(reason.into());
        self
    }

    /// Mark as repair attempted
    pub fn with_repair(mut self, outcome: impl Into<String>) -> Self {
        self.repair_attempted = true;
        self.repair_outcome = Some(outcome.into());
        self
    }
}

// ============================================================================
// DELIVERABLE 5: Replay Inspector + Run Comparison
// ============================================================================

/// Comparison between original and replay runs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayComparison {
    /// Original run ID
    pub original_run_id: String,
    /// Replay run ID
    pub replay_run_id: String,
    /// Whether runs matched
    pub matched: bool,
    /// Individual section comparisons
    pub compared_sections: Vec<ReplayComparisonSection>,
}

/// Single section comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayComparisonSection {
    /// Name of section
    pub section: String,
    /// Whether this section matched
    pub matched: bool,
    /// Expected value
    pub expected: String,
    /// Actual value
    pub actual: String,
    /// Explanation of mismatch (if any)
    pub explanation: Option<String>,
}

impl ReplayComparison {
    /// Create a new comparison
    pub fn new(original_run_id: impl Into<String>, replay_run_id: impl Into<String>) -> Self {
        Self {
            original_run_id: original_run_id.into(),
            replay_run_id: replay_run_id.into(),
            matched: true,
            compared_sections: Vec::new(),
        }
    }

    /// Add a section comparison
    pub fn add_section(
        &mut self,
        section: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) {
        let section = section.into();
        let expected = expected.into();
        let actual = actual.into();
        let matched = expected == actual;

        if !matched {
            self.matched = false;
        }

        self.compared_sections.push(ReplayComparisonSection {
            section,
            matched,
            expected,
            actual,
            explanation: None,
        });
    }

    /// Get first mismatch (if any)
    pub fn first_mismatch(&self) -> Option<&ReplayComparisonSection> {
        self.compared_sections.iter().find(|s| !s.matched)
    }
}

// ============================================================================
// DELIVERABLE 6: Debug Bundle Export
// ============================================================================

/// Complete debug artifact bundle for a run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugBundle {
    /// Task description
    pub task: String,
    /// Run ID
    pub run_id: String,
    /// Execution timeline
    pub timeline: ExecutionTimeline,
    /// Planner traces
    pub planner_traces: Vec<PlannerTrace>,
    /// Step mutation summaries
    pub step_mutations: Vec<StepMutationSummary>,
    /// Step state summaries
    pub step_states: Vec<StepStateSummary>,
    /// Failure explanation (if failed)
    pub failure_explanation: Option<FailureExplanation>,
    /// Replay comparison (if replayed)
    pub replay_comparison: Option<ReplayComparison>,
}

impl DebugBundle {
    /// Create a new debug bundle
    pub fn new(
        run_id: impl Into<String>,
        task: impl Into<String>,
        timeline: ExecutionTimeline,
    ) -> Self {
        Self {
            task: task.into(),
            run_id: run_id.into(),
            timeline,
            planner_traces: Vec::new(),
            step_mutations: Vec::new(),
            step_states: Vec::new(),
            failure_explanation: None,
            replay_comparison: None,
        }
    }

    /// Export bundle as JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Export bundle to a directory structure
    pub fn export_to_directory(&self, base_path: &std::path::Path) -> Result<(), std::io::Error> {
        use std::fs;

        // Create run directory
        let run_dir = base_path.join(format!("run_{}", self.run_id));
        fs::create_dir_all(&run_dir)?;

        // Export timeline
        let timeline_json = self
            .timeline
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        fs::write(run_dir.join("timeline.json"), timeline_json)?;

        // Export planner traces
        let traces_json = serde_json::to_string_pretty(&self.planner_traces)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        fs::write(run_dir.join("planner_traces.json"), traces_json)?;

        // Export mutations
        let mutations_json = serde_json::to_string_pretty(&self.step_mutations)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        fs::write(run_dir.join("mutations.json"), mutations_json)?;

        // Export state summaries
        let states_json = serde_json::to_string_pretty(&self.step_states)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        fs::write(run_dir.join("state_summaries.json"), states_json)?;

        // Export full bundle
        let bundle_json = self
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        fs::write(run_dir.join("debug_bundle.json"), bundle_json)?;

        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_entry_creation() {
        let entry = TimelineEntry::new(
            0,
            TimelinePhase::Execution,
            TimelineStatus::Running,
            "Test step",
        );
        assert_eq!(entry.index, 0);
        assert_eq!(entry.phase, TimelinePhase::Execution);
        assert_eq!(entry.status, TimelineStatus::Running);
        assert_eq!(entry.summary, "Test step");
    }

    #[test]
    fn timeline_entry_with_details() {
        let entry = TimelineEntry::new(
            0,
            TimelinePhase::Validation,
            TimelineStatus::Completed,
            "Validation passed",
        )
        .with_detail("All checks passed")
        .with_step(5)
        .with_tool("read_file")
        .with_validation_stage("build");

        assert_eq!(entry.detail, Some("All checks passed".to_string()));
        assert_eq!(entry.related_step, Some(5));
        assert_eq!(entry.related_tool, Some("read_file".to_string()));
        assert_eq!(entry.related_validation_stage, Some("build".to_string()));
    }

    #[test]
    fn execution_timeline_basic() {
        let mut timeline = ExecutionTimeline::new("run-001", "Test task");
        assert_eq!(timeline.run_id, "run-001");
        assert_eq!(timeline.task, "Test task");
        assert!(matches!(timeline.outcome, TimelineOutcome::InProgress));

        timeline.add_entry(TimelineEntry::new(
            0,
            TimelinePhase::Planning,
            TimelineStatus::Completed,
            "Planning done",
        ));
        timeline.complete_success();

        assert!(matches!(timeline.outcome, TimelineOutcome::Success));
        assert!(timeline.finished_at.is_some());
    }

    #[test]
    fn timeline_filtering_by_phase() {
        let mut timeline = ExecutionTimeline::new("run-001", "Test");
        timeline.add_entry(TimelineEntry::new(
            0,
            TimelinePhase::Planning,
            TimelineStatus::Completed,
            "Plan",
        ));
        timeline.add_entry(TimelineEntry::new(
            1,
            TimelinePhase::Execution,
            TimelineStatus::Running,
            "Exec",
        ));
        timeline.add_entry(TimelineEntry::new(
            2,
            TimelinePhase::Validation,
            TimelineStatus::Completed,
            "Valid",
        ));
        timeline.add_entry(TimelineEntry::new(
            3,
            TimelinePhase::Execution,
            TimelineStatus::Completed,
            "Exec 2",
        ));

        let exec_entries = timeline.entries_for_phase(TimelinePhase::Execution);
        assert_eq!(exec_entries.len(), 2);
    }

    #[test]
    fn timeline_failed_entries() {
        let mut timeline = ExecutionTimeline::new("run-001", "Test");
        timeline.add_entry(TimelineEntry::new(
            0,
            TimelinePhase::Execution,
            TimelineStatus::Completed,
            "Success",
        ));
        timeline.add_entry(TimelineEntry::new(
            1,
            TimelinePhase::Validation,
            TimelineStatus::Failed,
            "Build failed",
        ));
        timeline.add_entry(TimelineEntry::new(
            2,
            TimelinePhase::Execution,
            TimelineStatus::Completed,
            "Another success",
        ));

        let failed = timeline.failed_entries();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].summary, "Build failed");
    }

    #[test]
    fn timeline_display_formats() {
        assert_eq!(
            format!("{}", TimelinePhase::ToolExecution),
            "tool_execution"
        );
        assert_eq!(format!("{}", TimelineStatus::Failed), "failed");
    }

    // FailureExplainer tests
    #[test]
    fn failure_explanation_creation() {
        let explanation = FailureExplanation::new(
            FailureCategory::ValidationFailed,
            "Build failed",
            "The build stage detected compilation errors",
        );
        assert_eq!(explanation.category, FailureCategory::ValidationFailed);
        assert_eq!(explanation.headline, "Build failed");
    }

    #[test]
    fn failure_explanation_with_details() {
        let explanation = FailureExplanation::new(
            FailureCategory::ToolExecutionFailed,
            "Tool failed",
            "Execution error",
        )
        .with_context("Reading file: test.rs")
        .add_remediation("Check the file path")
        .add_related("test.rs");

        assert_eq!(explanation.context, "Reading file: test.rs");
        assert_eq!(explanation.remediation.len(), 1);
        assert_eq!(explanation.related_items.len(), 1);
    }

    #[test]
    fn failure_explainer_validation() {
        let explanation =
            FailureExplainer::explain_validation_failure("build", "syntax error", Some("main.rs"));
        assert_eq!(explanation.category, FailureCategory::ValidationFailed);
        assert!(explanation.headline.contains("build"));
        assert!(explanation.context.contains("main.rs"));
    }

    #[test]
    fn failure_explainer_tool_failure() {
        let explanation = FailureExplainer::explain_tool_failure("read_file", "file not found");
        assert_eq!(explanation.category, FailureCategory::ToolExecutionFailed);
        assert!(explanation.headline.contains("read_file"));
    }

    #[test]
    fn failure_explainer_chain_failure() {
        let explanation = FailureExplainer::explain_chain_failure(2, "validation failed");
        assert_eq!(explanation.category, FailureCategory::ChainError);
        assert!(explanation.headline.contains("step 3"));
    }

    #[test]
    fn failure_explainer_planner_error() {
        let explanation = FailureExplainer::explain_planner_error("schema", "invalid JSON");
        assert_eq!(explanation.category, FailureCategory::PlannerError);
        assert!(explanation.headline.contains("schema"));
    }

    #[test]
    fn failure_explainer_timeout() {
        let explanation = FailureExplainer::explain_timeout("tool execution", 30000);
        assert_eq!(explanation.category, FailureCategory::ResourceLimit);
        assert!(explanation.headline.contains("timed out"));
        assert!(explanation.explanation.contains("30"));
    }

    #[test]
    fn failure_explanation_format_cli() {
        let explanation = FailureExplanation::new(
            FailureCategory::ValidationFailed,
            "Test failed",
            "Assertion failed",
        )
        .with_context("Running tests")
        .add_remediation("Check test output")
        .add_remediation("Fix failing test");

        let formatted = explanation.format_cli();
        assert!(formatted.contains("Test failed"));
        assert!(formatted.contains("Validation Failed"));
        assert!(formatted.contains("Running tests"));
        assert!(formatted.contains("How to fix"));
        assert!(formatted.contains("1. Check test output"));
        assert!(formatted.contains("2. Fix failing test"));
    }
}
