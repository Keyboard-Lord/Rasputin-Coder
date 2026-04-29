use crate::app::App;
use crate::guidance::Severity;
use crate::persistence::{
    ChainLifecycleStatus, ChainStepStatus, ExecutionOutcome, PersistentChain,
};
use crate::repo::capture_git_grounding;
use crate::state::{ExecutionMode, ExecutionState};
use crate::ui::colors;
use crate::ui::widgets::input_bar::truncate_label;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

/// Draw the global status bar at the bottom of the screen
/// Normal mode: Shows simplified human-readable progress and status
/// Operator mode: Shows technical details [Chain] [Step] [State] [Git] [Context] [Mode]
/// Reactive: Shows severity indicators and state explanations
pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width < 40 {
        return;
    }

    // Split area into main bar and explanation line (if needed)
    let explanation = get_state_explanation(app);
    let has_explanation = explanation.is_some() && area.height > 1;

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),                                      // Main status bar
            Constraint::Length(if has_explanation { 1 } else { 0 }), // Explanation line
        ])
        .split(area);

    let main_area = main_layout[0];

    // Background block
    let block = Block::default().style(Style::default().bg(colors::BG_RAISED));
    f.render_widget(block, area);

    // Choose layout based on experience mode
    if app.is_operator_mode() {
        draw_operator_status_bar(f, app, main_area);
    } else {
        draw_normal_status_bar(f, app, main_area);
    }

    // State explanation line (if there's room and reason) - shown in both modes
    if has_explanation && let Some(reason) = explanation {
        let explanation_text = Line::from(vec![Span::styled(
            format!("  {} {}", Severity::Warning.indicator(), reason),
            Style::default()
                .fg(colors::WARNING)
                .add_modifier(Modifier::ITALIC),
        )]);
        f.render_widget(
            Paragraph::new(explanation_text).alignment(Alignment::Left),
            main_layout[1],
        );
    }
}

/// Draw technical status bar for Operator mode
/// Shows: [Chain] [Step] [State] [Git] [Context] [Mode]
fn draw_operator_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let segments = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20), // Chain ID
            Constraint::Length(12), // Step
            Constraint::Length(16), // State (with severity)
            Constraint::Min(20),    // Git
            Constraint::Length(14), // Context
            Constraint::Length(12), // Mode
        ])
        .split(area);

    // 1. Chain segment
    let chain_text = get_chain_segment(app);
    f.render_widget(
        Paragraph::new(chain_text).alignment(Alignment::Center),
        segments[0],
    );

    // 2. Step segment
    let step_text = get_step_segment(app);
    f.render_widget(
        Paragraph::new(step_text).alignment(Alignment::Center),
        segments[1],
    );

    // 3. State segment (with severity and color)
    let (state_text, state_color) = get_state_segment(app);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            state_text,
            Style::default()
                .fg(state_color)
                .add_modifier(Modifier::BOLD),
        )]))
        .alignment(Alignment::Center),
        segments[2],
    );

    // 4. Git segment
    let git_text = get_git_segment(app);
    f.render_widget(
        Paragraph::new(git_text).alignment(Alignment::Center),
        segments[3],
    );

    // 5. Context segment
    let context_text = get_context_segment(app);
    f.render_widget(
        Paragraph::new(context_text).alignment(Alignment::Center),
        segments[4],
    );

    // 6. Mode segment
    let (mode_text, mode_color) = get_mode_segment(app);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            mode_text,
            Style::default().fg(mode_color),
        )]))
        .alignment(Alignment::Center),
        segments[5],
    );
}

/// Draw simplified status bar for Normal mode
/// Shows: [Project] [Progress] [Status] [Model]
fn draw_normal_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let segments = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),    // Project name
            Constraint::Length(16), // Progress
            Constraint::Length(14), // Status
            Constraint::Length(18), // Model
        ])
        .split(area);

    // 1. Project name (cleaner than Chain ID)
    let project_text = get_project_segment(app);
    f.render_widget(
        Paragraph::new(project_text).alignment(Alignment::Left),
        segments[0],
    );

    // 2. Simple progress (not step X/Y technical format)
    let progress_text = get_simple_progress_segment(app);
    f.render_widget(
        Paragraph::new(progress_text).alignment(Alignment::Center),
        segments[1],
    );

    // 3. Human-readable status
    let (status_text, status_color) = get_simple_status_segment(app);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            status_text,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        )]))
        .alignment(Alignment::Center),
        segments[2],
    );

    // 4. Simple model indicator (no DISCONNECTED alarm)
    let model_text = get_simple_model_segment(app);
    f.render_widget(
        Paragraph::new(model_text).alignment(Alignment::Right),
        segments[3],
    );
}

fn get_chain_segment(app: &App) -> Line<'static> {
    let label = Span::styled("Chain: ", Style::default().fg(colors::TEXT_DIM));

    let value = if let Some(ref chain_id) = app.persistence.active_chain_id {
        let short_id = if chain_id.chars().count() > 12 {
            crate::text::truncate_chars(chain_id, 15)
        } else {
            chain_id.clone()
        };
        Span::styled(short_id, Style::default().fg(colors::TEXT_SOFT))
    } else {
        Span::styled("—", Style::default().fg(colors::TEXT_SUBTLE))
    };

    Line::from(vec![label, value])
}

fn get_step_segment(app: &App) -> Line<'static> {
    let label = Span::styled("Step: ", Style::default().fg(colors::TEXT_DIM));

    let value = if let Some(ref chain_id) = app.persistence.active_chain_id {
        if let Some(chain) = app.persistence.get_chain(chain_id) {
            Span::styled(
                step_progress_text(chain),
                Style::default().fg(colors::TEXT_SOFT),
            )
        } else {
            Span::styled("—", Style::default().fg(colors::TEXT_SUBTLE))
        }
    } else {
        Span::styled("—", Style::default().fg(colors::TEXT_SUBTLE))
    };

    Line::from(vec![label, value])
}

fn step_progress_text(chain: &PersistentChain) -> String {
    let total = chain.steps.len();
    if total == 0 {
        return "0/0".to_string();
    }

    let current = if matches!(chain.status, ChainLifecycleStatus::Complete) {
        total
    } else {
        chain
            .active_step
            .and_then(|idx| (idx < total).then_some(idx + 1))
            .or_else(|| {
                chain
                    .steps
                    .iter()
                    .position(|step| matches!(step.status, ChainStepStatus::Running))
                    .map(|idx| idx + 1)
            })
            .or_else(|| {
                chain
                    .steps
                    .iter()
                    .position(|step| {
                        matches!(
                            step.status,
                            ChainStepStatus::Pending
                                | ChainStepStatus::Blocked
                                | ChainStepStatus::Failed
                        )
                    })
                    .map(|idx| idx + 1)
            })
            .unwrap_or_else(|| (chain.total_steps_executed as usize).clamp(1, total))
    };

    format!("{}/{}", current.min(total), total)
}

fn get_state_segment(app: &App) -> (String, Color) {
    // V1.5 UNIFICATION: Use ExecutionOutcome when available for terminal truth
    let chain_outcome = if let Some(ref chain_id) = app.persistence.active_chain_id {
        app.persistence
            .get_chain(chain_id)
            .and_then(|c| c.get_outcome())
    } else {
        None
    };

    let chain_status = if let Some(ref chain_id) = app.persistence.active_chain_id {
        app.persistence.get_chain(chain_id).map(|c| c.status)
    } else {
        None
    };

    state_segment_text(
        app.state.execution.state,
        chain_status,
        chain_outcome,
        app.chat_blocked,
        app.persistence.has_active_checkpoint(),
    )
}

fn state_segment_text(
    execution_state: ExecutionState,
    chain_status: Option<ChainLifecycleStatus>,
    chain_outcome: Option<ExecutionOutcome>,
    chat_blocked: bool,
    has_active_checkpoint: bool,
) -> (String, Color) {
    // V1.5 UNIFICATION: Prioritize outcome over lifecycle for terminal truth
    if let Some(outcome) = chain_outcome {
        return outcome_segment_text(outcome);
    }

    let is_precondition_failed = matches!(execution_state, ExecutionState::PreconditionFailed);

    let is_blocked = chat_blocked
        || matches!(execution_state, ExecutionState::Blocked)
        || matches!(
            chain_status,
            Some(ChainLifecycleStatus::Halted | ChainLifecycleStatus::WaitingForApproval)
        )
        || has_active_checkpoint;

    // Check for failed state
    let is_failed = matches!(execution_state, ExecutionState::Failed)
        || matches!(chain_status, Some(ChainLifecycleStatus::Failed));

    let (text, color, _severity) = if is_precondition_failed {
        let severity = Severity::Warning;
        (
            format!("{} Preflight", severity.indicator()),
            severity.color(),
            severity,
        )
    } else if is_blocked {
        let severity = Severity::Warning;
        (
            format!("{} Blocked", severity.indicator()),
            severity.color(),
            severity,
        )
    } else if is_failed {
        let severity = Severity::Critical;
        (
            format!("{} Failed", severity.indicator()),
            severity.color(),
            severity,
        )
    } else {
        match execution_state {
            ExecutionState::Idle => match chain_status {
                Some(ChainLifecycleStatus::Draft | ChainLifecycleStatus::Ready) => {
                    ("Ready".to_string(), colors::SUCCESS, Severity::Info)
                }
                Some(ChainLifecycleStatus::Running) => {
                    ("Running".to_string(), colors::ACCENT, Severity::Info)
                }
                Some(ChainLifecycleStatus::Complete) => {
                    ("Done".to_string(), colors::SUCCESS, Severity::Info)
                }
                Some(ChainLifecycleStatus::Archived) => {
                    ("Archived".to_string(), colors::TEXT_MUTED, Severity::Info)
                }
                Some(ChainLifecycleStatus::Halted | ChainLifecycleStatus::WaitingForApproval) => {
                    let severity = Severity::Warning;
                    (
                        format!("{} Blocked", severity.indicator()),
                        severity.color(),
                        severity,
                    )
                }
                Some(ChainLifecycleStatus::Failed) => {
                    let severity = Severity::Critical;
                    (
                        format!("{} Failed", severity.indicator()),
                        severity.color(),
                        severity,
                    )
                }
                None => ("Idle".to_string(), colors::TEXT_MUTED, Severity::Info),
            },
            ExecutionState::Planning => ("Planning".to_string(), colors::WARNING, Severity::Info),
            ExecutionState::Executing => ("Running".to_string(), colors::ACCENT, Severity::Info),
            ExecutionState::Responding => ("Running".to_string(), colors::ACCENT, Severity::Info),
            ExecutionState::Validating => {
                ("Validating".to_string(), colors::WARNING, Severity::Info)
            }
            ExecutionState::Repairing => ("Repairing".to_string(), colors::WARNING, Severity::Info),
            ExecutionState::WaitingForApproval => {
                let severity = Severity::Warning;
                (
                    format!("{} Approval", severity.indicator()),
                    severity.color(),
                    severity,
                )
            }
            ExecutionState::Done => ("Done".to_string(), colors::SUCCESS, Severity::Info),
            ExecutionState::Failed => {
                let severity = Severity::Critical;
                (
                    format!("{} Failed", severity.indicator()),
                    severity.color(),
                    severity,
                )
            }
            ExecutionState::Blocked => {
                let severity = Severity::Warning;
                (
                    format!("{} Blocked", severity.indicator()),
                    severity.color(),
                    severity,
                )
            }
            ExecutionState::PreconditionFailed => {
                let severity = Severity::Warning;
                (
                    format!("{} Preflight", severity.indicator()),
                    severity.color(),
                    severity,
                )
            }
        }
    };

    (text, color)
}

/// V1.5 UNIFICATION: Canonical mapping from ExecutionOutcome to status bar display
/// This ensures all UI surfaces render consistently from authoritative outcome
fn outcome_segment_text(outcome: ExecutionOutcome) -> (String, Color) {
    match outcome {
        ExecutionOutcome::Success => {
            let severity = Severity::Info;
            (format!("{} Done", severity.indicator()), colors::SUCCESS)
        }
        ExecutionOutcome::SuccessWithWarnings => {
            let severity = Severity::Warning;
            (
                format!("{} Done (warn)", severity.indicator()),
                colors::WARNING,
            )
        }
        ExecutionOutcome::Blocked => {
            let severity = Severity::Warning;
            (
                format!("{} Blocked", severity.indicator()),
                severity.color(),
            )
        }
        ExecutionOutcome::Failed => {
            let severity = Severity::Critical;
            (format!("{} Failed", severity.indicator()), severity.color())
        }
    }
}

/// Get state explanation for the status bar
/// V1.5 CLEANUP: Prioritizes ExecutionOutcome over legacy block metadata
fn get_state_explanation(app: &App) -> Option<String> {
    // First check for active checkpoint (this is live state, not legacy)
    if app.persistence.has_active_checkpoint()
        && let Some(checkpoint) = app.persistence.get_active_checkpoint()
    {
        return Some(format!(
            "Awaiting approval ({})",
            checkpoint.risk_level.label()
        ));
    }

    // Check for chain-level outcome first (authoritative truth)
    if let Some(ref chain_id) = app.persistence.active_chain_id
        && let Some(chain) = app.persistence.get_chain(chain_id)
    {
        if let Some(outcome) = chain.get_outcome() {
            match outcome {
                ExecutionOutcome::Success => return None, // No explanation needed for clean success
                ExecutionOutcome::SuccessWithWarnings => {
                    return Some("Completed with warnings".to_string());
                }
                ExecutionOutcome::Blocked => {
                    // Only show block metadata when outcome is Blocked
                    return app
                        .state
                        .execution
                        .block_reason
                        .clone()
                        .or_else(|| Some("Execution blocked".to_string()));
                }
                ExecutionOutcome::Failed => {
                    // Only show block metadata when outcome is Failed
                    return app
                        .state
                        .execution
                        .block_reason
                        .clone()
                        .or_else(|| Some("Execution failed".to_string()));
                }
            }
        }

        // Fallback to lifecycle status if outcome not yet computed
        match chain.status {
            ChainLifecycleStatus::Halted => return Some("Execution halted".to_string()),
            ChainLifecycleStatus::Failed => return Some("Execution failed".to_string()),
            _ => {}
        }
    }

    // Legacy fallback for non-chain execution
    if app.chat_blocked {
        return Some("Approval required".to_string());
    }

    if app.state.execution.state == ExecutionState::PreconditionFailed {
        return app
            .state
            .execution
            .block_reason
            .clone()
            .or_else(|| Some("Autonomy preflight failed".to_string()));
    }

    None
}

fn get_git_segment(app: &App) -> Line<'static> {
    let grounding = capture_git_grounding(&app.state.repo.path);

    if !grounding.repo_detected {
        return Line::from(vec![
            Span::styled("Git: ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled("no repo", Style::default().fg(colors::TEXT_SUBTLE)),
        ]);
    }

    let commit = grounding
        .head_commit
        .as_ref()
        .map(|c| crate::text::take_chars(c, 7))
        .unwrap_or_else(|| "???????".to_string());

    let branch = grounding.branch_name.as_deref().unwrap_or("(detached)");

    let dirty_marker = if grounding.is_dirty { "*" } else { "" };

    Line::from(vec![
        Span::styled("Git: ", Style::default().fg(colors::TEXT_DIM)),
        Span::styled(
            format!("{}{} {}{}", commit, dirty_marker, branch, dirty_marker),
            Style::default().fg(colors::TEXT_SOFT),
        ),
    ])
}

fn get_context_segment(app: &App) -> Line<'static> {
    // CRITICAL FIX: Show Ollama connection status clearly
    let (status_text, status_color) = if app.state.ollama_connected {
        ("● CONNECTED", colors::SUCCESS)
    } else {
        ("● DISCONNECTED", colors::ERROR)
    };

    // Show model name if available, truncated
    let model_info = app
        .state
        .model
        .active
        .as_ref()
        .map(|m| {
            let short = if m.chars().count() > 10 {
                format!("{}..", crate::text::take_chars(m, 8))
            } else {
                m.clone()
            };
            format!(" | {}", short)
        })
        .unwrap_or_default();

    Line::from(vec![
        Span::styled(
            status_text,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(model_info, Style::default().fg(colors::TEXT_SOFT)),
    ])
}

fn get_mode_segment(app: &App) -> (String, Color) {
    match app.state.execution.mode {
        ExecutionMode::Chat => ("Chat".to_string(), colors::SUCCESS),
        ExecutionMode::Edit => ("Edit".to_string(), colors::WARNING),
        ExecutionMode::Task => ("Task".to_string(), colors::ACCENT),
    }
}

// ============================================================================
// NORMAL MODE STATUS BAR HELPERS (Human-readable, simplified)
// ============================================================================

/// Simple project name display (replaces Chain ID hash)
fn get_project_segment(app: &App) -> Line<'static> {
    let name = if app.state.repo.name.is_empty() || app.state.repo.name == "No repo" {
        Span::styled("No project", Style::default().fg(colors::TEXT_SUBTLE))
    } else {
        Span::styled(
            truncate_label(&app.state.repo.name, 18),
            Style::default().fg(colors::TEXT_SOFT),
        )
    };

    Line::from(vec![
        Span::styled("Project: ", Style::default().fg(colors::TEXT_DIM)),
        name,
    ])
}

/// Simple progress indicator (replaces technical Step X/Y)
fn get_simple_progress_segment(app: &App) -> Line<'static> {
    let progress = if let Some(ref chain_id) = app.persistence.active_chain_id {
        if let Some(chain) = app.persistence.get_chain(chain_id) {
            let total = chain.steps.len();
            if total == 0 {
                Span::styled("—".to_string(), Style::default().fg(colors::TEXT_SUBTLE))
            } else if matches!(chain.status, ChainLifecycleStatus::Complete) {
                Span::styled("Done".to_string(), Style::default().fg(colors::SUCCESS))
            } else {
                let current = chain
                    .active_step
                    .map(|idx| (idx + 1).min(total))
                    .unwrap_or_else(|| {
                        chain
                            .steps
                            .iter()
                            .position(|s| {
                                matches!(
                                    s.status,
                                    ChainStepStatus::Pending | ChainStepStatus::Running
                                )
                            })
                            .map(|idx| idx + 1)
                            .unwrap_or(1)
                    });
                Span::styled(
                    format!("{} of {}", current, total),
                    Style::default().fg(colors::TEXT_SOFT),
                )
            }
        } else {
            Span::styled("—".to_string(), Style::default().fg(colors::TEXT_SUBTLE))
        }
    } else {
        Span::styled("—".to_string(), Style::default().fg(colors::TEXT_SUBTLE))
    };

    Line::from(vec![
        Span::styled("Step: ", Style::default().fg(colors::TEXT_DIM)),
        progress,
    ])
}

/// Human-readable status (replaces technical state names)
fn get_simple_status_segment(app: &App) -> (String, Color) {
    use crate::state::ExecutionState;

    // V1.6: Check for active recovery first - show calm narration in Normal mode
    if !app.recovery_state.recovery_path.is_empty() {
        let recovery_summary = app.get_recovery_summary();
        if !recovery_summary.is_empty() {
            // Use warning color for recovery in progress, accent for completion
            let color = if app.recovery_state.recovery_in_progress {
                colors::WARNING
            } else {
                colors::ACCENT
            };
            return (recovery_summary, color);
        }
    }

    // First check for chain outcome
    if let Some(ref chain_id) = app.persistence.active_chain_id {
        if let Some(chain) = app.persistence.get_chain(chain_id) {
            if let Some(outcome) = chain.get_outcome() {
                return match outcome {
                    crate::persistence::ExecutionOutcome::Success => {
                        ("Ready".to_string(), colors::SUCCESS)
                    }
                    crate::persistence::ExecutionOutcome::SuccessWithWarnings => {
                        ("Done".to_string(), colors::WARNING)
                    }
                    crate::persistence::ExecutionOutcome::Blocked => {
                        ("Paused".to_string(), colors::WARNING)
                    }
                    crate::persistence::ExecutionOutcome::Failed => {
                        ("Stopped".to_string(), colors::ERROR)
                    }
                };
            }
        }
    }

    // Map execution state to human-readable
    match app.state.execution.state {
        ExecutionState::Idle => ("Ready".to_string(), colors::TEXT_MUTED),
        ExecutionState::Planning => ("Thinking...".to_string(), colors::WARNING),
        ExecutionState::Executing => ("Working...".to_string(), colors::ACCENT),
        ExecutionState::Validating => ("Checking...".to_string(), colors::WARNING),
        ExecutionState::Repairing => ("Fixing...".to_string(), colors::WARNING),
        ExecutionState::Responding => ("Replying...".to_string(), colors::ACCENT),
        ExecutionState::WaitingForApproval => ("Needs input".to_string(), colors::WARNING),
        ExecutionState::Done => ("Done".to_string(), colors::SUCCESS),
        ExecutionState::Failed => ("Stopped".to_string(), colors::ERROR),
        ExecutionState::Blocked => ("Paused".to_string(), colors::WARNING),
        ExecutionState::PreconditionFailed => ("Ready".to_string(), colors::TEXT_MUTED),
    }
}

/// Simple model indicator without alarmist DISCONNECTED state
fn get_simple_model_segment(app: &App) -> Line<'static> {
    let model_label = app
        .state
        .model
        .active
        .as_ref()
        .or(app.state.model.configured.as_ref())
        .map(|m| truncate_label(m, 14))
        .unwrap_or_else(|| "Local AI".to_string());

    let (indicator, color) = if app.state.ollama_connected {
        ("●", colors::SUCCESS)
    } else {
        // Use neutral color instead of error red for local-first product
        ("◌", colors::TEXT_DIM)
    };

    Line::from(vec![
        Span::styled(format!("{} ", indicator), Style::default().fg(color)),
        Span::styled(model_label, Style::default().fg(colors::TEXT_SOFT)),
    ])
}

#[cfg(test)]
mod tests {
    use super::{state_segment_text, step_progress_text};
    use crate::persistence::{
        ChainLifecycleStatus, ChainStepStatus, PersistentChain, PersistentChainStep,
    };
    use crate::state::ExecutionState;
    use chrono::Local;

    fn test_step(id: &str, status: ChainStepStatus) -> PersistentChainStep {
        PersistentChainStep {
            id: id.to_string(),
            description: format!("step {}", id),
            status,
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
        }
    }

    fn test_chain(
        status: ChainLifecycleStatus,
        steps: Vec<PersistentChainStep>,
        active_step: Option<usize>,
        total_steps_executed: u32,
    ) -> PersistentChain {
        let now = Local::now();
        PersistentChain {
            id: "chain-test".to_string(),
            name: "Test Chain".to_string(),
            objective: "Test objective".to_string(),
            raw_prompt: "Test objective".to_string(),
            status,
            steps,
            active_step,
            repo_path: None,
            conversation_id: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
            archived: false,
            total_steps_executed,
            total_steps_failed: 0,
            execution_outcome: None,
            force_override_used: false,
            objective_satisfaction: crate::state::ObjectiveSatisfaction::default(),
            selected_context_files: vec![],
            context_state: None,
            pending_checkpoint: None,
            git_grounding: None,
            audit_log: crate::state::AuditLog::new(),
        }
    }

    #[test]
    fn completed_chain_progress_never_advances_past_total() {
        let chain = test_chain(
            ChainLifecycleStatus::Complete,
            vec![
                test_step("1", ChainStepStatus::Completed),
                test_step("2", ChainStepStatus::Completed),
            ],
            None,
            2,
        );

        assert_eq!(step_progress_text(&chain), "2/2");
    }

    #[test]
    fn chain_progress_uses_next_pending_step() {
        let chain = test_chain(
            ChainLifecycleStatus::Halted,
            vec![
                test_step("1", ChainStepStatus::Completed),
                test_step("2", ChainStepStatus::Pending),
            ],
            None,
            1,
        );

        assert_eq!(step_progress_text(&chain), "2/2");
    }

    #[test]
    fn empty_chain_progress_is_truthful() {
        let chain = test_chain(ChainLifecycleStatus::Draft, vec![], None, 0);

        assert_eq!(step_progress_text(&chain), "0/0");
    }

    #[test]
    fn idle_status_bar_respects_completed_chain_status() {
        let (text, _) = state_segment_text(
            ExecutionState::Idle,
            Some(ChainLifecycleStatus::Complete),
            None, // No outcome set - falls back to lifecycle status
            false,
            false,
        );

        assert_eq!(text, "Done");
    }

    #[test]
    fn waiting_for_approval_status_bar_is_blocked() {
        let (text, _) = state_segment_text(
            ExecutionState::Idle,
            Some(ChainLifecycleStatus::WaitingForApproval),
            None, // No outcome set - falls back to lifecycle status
            false,
            false,
        );

        assert!(text.contains("Blocked"));
    }
}
