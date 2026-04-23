//! Observability View Models - TUI Integration
//!
//! Provides view models for displaying observability data in the TUI.
//! Maps runtime observability structures to UI-friendly formats.

use crate::forge_runtime::observability as runtime_obs;

/// View model for timeline entry
#[derive(Debug, Clone)]
pub struct TimelineEntryView {
    pub index: u32,
    pub phase: String,
    pub status: TimelineEntryStatus,
    pub summary: String,
    pub detail: Option<String>,
    pub step_index: Option<u32>,
    pub tool_name: Option<String>,
    pub timestamp: String,
    pub is_failure_point: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineEntryStatus {
    Success,
    Failure,
    Warning,
    Pending,
    Running,
}

impl TimelineEntryView {
    pub fn from_runtime(entry: &runtime_obs::TimelineEntry) -> Self {
        let status = match entry.status {
            runtime_obs::TimelineStatus::Completed => TimelineEntryStatus::Success,
            runtime_obs::TimelineStatus::Failed => TimelineEntryStatus::Failure,
            runtime_obs::TimelineStatus::Skipped => TimelineEntryStatus::Warning,
            runtime_obs::TimelineStatus::Pending => TimelineEntryStatus::Pending,
            runtime_obs::TimelineStatus::Running => TimelineEntryStatus::Running,
        };

        Self {
            index: entry.index,
            phase: entry.phase.to_string(),
            status,
            summary: entry.summary.clone(),
            detail: entry.detail.clone(),
            step_index: entry.related_step,
            tool_name: entry.related_tool.clone(),
            timestamp: format_timestamp(entry.timestamp),
            is_failure_point: matches!(entry.status, runtime_obs::TimelineStatus::Failed),
        }
    }
}

/// View model for execution timeline
#[derive(Debug, Clone)]
pub struct ExecutionTimelineView {
    pub run_id: String,
    pub task: String,
    pub entries: Vec<TimelineEntryView>,
    pub duration_ms: Option<u64>,
    pub outcome: TimelineOutcome,
    pub failure_entry_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineOutcome {
    Success,
    Failure,
    InProgress,
}

impl ExecutionTimelineView {
    pub fn from_runtime(timeline: &runtime_obs::ExecutionTimeline) -> Self {
        let entries: Vec<_> = timeline
            .entries
            .iter()
            .map(TimelineEntryView::from_runtime)
            .collect();

        let failure_entry_index = entries.iter().position(|e| e.is_failure_point);

        let outcome = match &timeline.outcome {
            runtime_obs::TimelineOutcome::Success => TimelineOutcome::Success,
            runtime_obs::TimelineOutcome::Failure { .. } => TimelineOutcome::Failure,
            runtime_obs::TimelineOutcome::InProgress => TimelineOutcome::InProgress,
        };

        Self {
            run_id: timeline.run_id.clone(),
            task: timeline.task.clone(),
            entries,
            duration_ms: timeline.total_duration_ms(),
            outcome,
            failure_entry_index,
        }
    }

    /// Get the index of the first failure entry
    pub fn first_failure_index(&self) -> Option<usize> {
        self.failure_entry_index
    }

    /// Check if any step failed
    pub fn has_failure(&self) -> bool {
        self.failure_entry_index.is_some()
    }
}

/// View model for failure explanation
#[derive(Debug, Clone)]
pub struct FailureExplanationView {
    pub class: String,
    pub short_message: String,
    pub likely_cause: String,
    pub impact: String,
    pub suggested_next_action: String,
    pub technical_detail: Option<String>,
    pub step_index: Option<u32>,
}

impl FailureExplanationView {
    pub fn from_runtime(exp: &runtime_obs::FailureExplanation, step: Option<u32>) -> Self {
        Self {
            class: exp.category.to_string(),
            short_message: exp.headline.clone(),
            likely_cause: exp.explanation.clone(),
            impact: if exp.context.is_empty() {
                "The task could not complete successfully".to_string()
            } else {
                format!("{}", exp.context)
            },
            suggested_next_action: exp
                .remediation
                .first()
                .map(|r| r.clone())
                .unwrap_or_else(|| "Review the error and try again".to_string()),
            technical_detail: None,
            step_index: step,
        }
    }
}

/// View model for step summary
#[derive(Debug, Clone)]
pub struct StepSummaryView {
    pub step_index: u32,
    pub status: StepStatus,
    pub summary: String,
    pub tool_name: Option<String>,
    pub mutations_committed: usize,
    pub mutations_reverted: usize,
    pub has_checkpoint: bool,
    pub has_replay_divergence: bool,
    pub repair_attempted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Reverted,
    Checkpointed,
}

impl StepSummaryView {
    pub fn new(step_index: u32) -> Self {
        Self {
            step_index,
            status: StepStatus::Pending,
            summary: String::new(),
            tool_name: None,
            mutations_committed: 0,
            mutations_reverted: 0,
            has_checkpoint: false,
            has_replay_divergence: false,
            repair_attempted: false,
        }
    }

    pub fn from_mutation_summary(
        step_index: u32,
        summary: &runtime_obs::StepMutationSummary,
    ) -> Self {
        Self {
            step_index,
            status: StepStatus::Passed,
            summary: String::new(),
            tool_name: None,
            mutations_committed: summary.committed_mutations.len(),
            mutations_reverted: summary.reverted_mutations.len(),
            has_checkpoint: false,
            has_replay_divergence: false,
            repair_attempted: false,
        }
    }

    /// Get badge text for this step
    pub fn badge_text(&self) -> &'static str {
        match self.status {
            StepStatus::Pending => "●",
            StepStatus::Running => "▶",
            StepStatus::Passed => "✓",
            StepStatus::Failed => "✗",
            StepStatus::Reverted => "↩",
            StepStatus::Checkpointed => "⚡",
        }
    }
}

/// View model for planner trace
#[derive(Debug, Clone)]
pub struct PlannerTraceView {
    pub step_index: u32,
    pub raw_excerpt: String,
    pub classification: String,
    pub accepted_tool: Option<String>,
    pub rejected_reason: Option<String>,
    pub repair_attempted: bool,
    pub repair_outcome: Option<String>,
}

impl PlannerTraceView {
    pub fn from_runtime(trace: &runtime_obs::PlannerTrace) -> Self {
        Self {
            step_index: trace.step_index,
            raw_excerpt: trace.raw_output_excerpt.clone(),
            classification: trace.classification.clone(),
            accepted_tool: trace.accepted_tool.clone(),
            rejected_reason: trace.rejected_reason.clone(),
            repair_attempted: trace.repair_attempted,
            repair_outcome: trace.repair_outcome.clone(),
        }
    }

    /// Check if this trace represents a successful tool call
    pub fn is_accepted(&self) -> bool {
        self.accepted_tool.is_some() && self.rejected_reason.is_none()
    }

    /// Check if this trace represents a rejection
    pub fn is_rejected(&self) -> bool {
        self.rejected_reason.is_some()
    }
}

/// View model for replay comparison section
#[derive(Debug, Clone)]
pub struct ReplayComparisonSectionView {
    pub section: String,
    pub matched: bool,
    pub expected: String,
    pub actual: String,
    pub explanation: Option<String>,
}

/// View model for replay comparison
#[derive(Debug, Clone)]
pub struct ReplayComparisonView {
    pub original_run_id: String,
    pub replay_run_id: String,
    pub matched: bool,
    pub sections: Vec<ReplayComparisonSectionView>,
    pub first_mismatch: Option<ReplayComparisonSectionView>,
}

impl ReplayComparisonView {
    pub fn from_runtime(comparison: &runtime_obs::ReplayComparison) -> Self {
        let sections: Vec<_> = comparison
            .compared_sections
            .iter()
            .map(|s| ReplayComparisonSectionView {
                section: s.section.clone(),
                matched: s.matched,
                expected: s.expected.clone(),
                actual: s.actual.clone(),
                explanation: s.explanation.clone(),
            })
            .collect();

        let first_mismatch = sections.iter().find(|s| !s.matched).cloned();

        Self {
            original_run_id: comparison.original_run_id.clone(),
            replay_run_id: comparison.replay_run_id.clone(),
            matched: comparison.matched,
            sections,
            first_mismatch,
        }
    }
}

/// View model for debug bundle
#[derive(Debug, Clone)]
pub struct DebugBundleView {
    pub run_id: String,
    pub task: String,
    pub export_path: String,
    pub files: Vec<String>,
    pub has_failure_explanation: bool,
    pub has_replay_comparison: bool,
    pub export_success: bool,
}

impl DebugBundleView {
    pub fn from_runtime(bundle: &runtime_obs::DebugBundle, path: &str) -> Self {
        let files = vec![
            "timeline.json".to_string(),
            "planner_traces.json".to_string(),
            "mutations.json".to_string(),
            "state_summaries.json".to_string(),
            "debug_bundle.json".to_string(),
        ];

        Self {
            run_id: bundle.run_id.clone(),
            task: bundle.task.clone(),
            export_path: path.to_string(),
            files,
            has_failure_explanation: bundle.failure_explanation.is_some(),
            has_replay_comparison: bundle.replay_comparison.is_some(),
            export_success: true,
        }
    }
}

/// End of run summary for display
#[derive(Debug, Clone)]
pub struct RunSummaryView {
    pub outcome: String,
    pub step_count: usize,
    pub committed_files: usize,
    pub reverted_files: usize,
    pub validation_passed: bool,
    pub replay_status: String,
    pub bundle_exported: bool,
    pub bundle_path: Option<String>,
}

impl RunSummaryView {
    pub fn success(step_count: usize, committed: usize) -> Self {
        Self {
            outcome: "Success".to_string(),
            step_count,
            committed_files: committed,
            reverted_files: 0,
            validation_passed: true,
            replay_status: "Not checked".to_string(),
            bundle_exported: false,
            bundle_path: None,
        }
    }

    pub fn failure(step_index: usize, _reason: &str) -> Self {
        Self {
            outcome: format!("Failed at step {}", step_index + 1),
            step_count: step_index + 1,
            committed_files: 0,
            reverted_files: 0,
            validation_passed: false,
            replay_status: "Not checked".to_string(),
            bundle_exported: false,
            bundle_path: None,
        }
    }

    /// Format as compact CLI output
    pub fn format_compact(&self) -> String {
        let mut lines = vec![
            format!("Run: {}", self.outcome),
            format!(
                "  Steps: {} | Files committed: {}",
                self.step_count, self.committed_files
            ),
        ];

        if self.reverted_files > 0 {
            lines.push(format!("  Files reverted: {}", self.reverted_files));
        }

        if self.replay_status != "Not checked" {
            lines.push(format!("  Replay: {}", self.replay_status));
        }

        if self.bundle_exported {
            if let Some(path) = &self.bundle_path {
                lines.push(format!("  Debug bundle: {}", path));
            }
        }

        lines.join("\n")
    }
}

/// Helper function to format timestamp
fn format_timestamp(timestamp: u64) -> String {
    use chrono::{Local, TimeZone};

    if let Some(dt) = Local.timestamp_millis_opt(timestamp as i64).single() {
        dt.format("%H:%M:%S").to_string()
    } else {
        format!("{}", timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_entry_view_from_runtime() {
        let entry = runtime_obs::TimelineEntry::new(
            0,
            "tool_execution",
            runtime_obs::TimelineStatus::Completed,
            "Test step",
        );

        let view = TimelineEntryView::from_runtime(&entry);
        assert_eq!(view.index, 0);
        assert_eq!(view.status, TimelineEntryStatus::Success);
        assert!(!view.is_failure_point);
    }

    #[test]
    fn step_summary_badge_texts() {
        assert_eq!(
            StepSummaryView::new(0)
                .with_status(StepStatus::Passed)
                .badge_text(),
            "✓"
        );
        assert_eq!(
            StepSummaryView::new(0)
                .with_status(StepStatus::Failed)
                .badge_text(),
            "✗"
        );
        assert_eq!(
            StepSummaryView::new(0)
                .with_status(StepStatus::Reverted)
                .badge_text(),
            "↩"
        );
    }

    #[test]
    fn run_summary_format_compact() {
        let summary = RunSummaryView::success(5, 3);
        let formatted = summary.format_compact();
        assert!(formatted.contains("Success"));
        assert!(formatted.contains("5"));
        assert!(formatted.contains("3"));
    }
}

// Helper trait for test status setting
trait WithStatus {
    fn with_status(self, status: StepStatus) -> Self;
}

impl WithStatus for StepSummaryView {
    fn with_status(mut self, status: StepStatus) -> Self {
        self.status = status;
        self
    }
}
