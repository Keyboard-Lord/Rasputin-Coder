use crate::app::{App, UiAction};
use crate::state::{AppState, InspectorTab, RuntimeEvent, RuntimeStatus};
use crate::ui::colors;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
};

/// Draw the right-side inspector panel
pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    // Clone values we need before any mutable borrows
    // Clone values we need before any mutable borrows
    let repo_name = app.state.repo.name.clone();
    let show_diff = !app.state.diff_store.mutations.is_empty();
    let mut active_tab = app.state.active_inspector_tab;
    if active_tab == InspectorTab::Diff && !show_diff {
        active_tab = InspectorTab::Runtime;
        app.state.active_inspector_tab = InspectorTab::Runtime;
    }

    // Split into header, tabs, and content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    // Draw header with session and repo info
    let header_text = format!(
        " Session: {} | Repo: {}",
        app.conversation_id.split('-').next().unwrap_or("unknown"),
        repo_name
    );
    let header = Paragraph::new(Line::from(vec![Span::styled(
        header_text,
        Style::default().fg(colors::TEXT_SOFT),
    )]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(colors::BORDER_SUBTLE)),
    );
    f.render_widget(header, chunks[0]);

    let tab_inner = chunks[1].inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    // Check if we have context to show
    let has_context = app
        .persistence
        .get_active_chain()
        .map(|chain| chain.context_state.is_some() || !chain.selected_context_files.is_empty())
        .unwrap_or(false);

    let mut tab_defs = vec![
        ("Activity", InspectorTab::Runtime, "inspector:runtime"),
        ("Checks", InspectorTab::Validation, "inspector:validation"),
        ("Preview", InspectorTab::Preview, "inspector:preview"),
        ("Recovery", InspectorTab::Checkpoint, "inspector:checkpoint"),
    ];
    if show_diff {
        tab_defs.insert(3, ("Changes", InspectorTab::Diff, "inspector:diff"));
    }
    if app.is_operator_mode() {
        if has_context {
            tab_defs.push(("Context", InspectorTab::Steps, "inspector:context"));
        }
        tab_defs.push(("Logs", InspectorTab::Logs, "inspector:logs"));
        tab_defs.push(("Audit", InspectorTab::Audit, "inspector:audit"));
    }

    if !tab_defs.iter().any(|(_, tab, _)| *tab == active_tab) {
        active_tab = InspectorTab::Runtime;
        app.state.active_inspector_tab = active_tab;
    }
    let constraints = vec![Constraint::Ratio(1, tab_defs.len() as u32); tab_defs.len()];
    let tab_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(tab_inner);
    for (index, (_, tab, id)) in tab_defs.iter().enumerate() {
        app.register_ui_target(
            (*id).to_string(),
            tab_chunks[index],
            UiAction::SelectInspectorTab(*tab),
        );
    }

    // Draw tabs
    let titles = tab_defs
        .iter()
        .map(|(title, _, _)| *title)
        .collect::<Vec<_>>();
    let mode_label = if app.is_operator_mode() {
        " Inspector: Operator "
    } else {
        " Inspector "
    };
    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .title(mode_label)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER_SUBTLE))
                .style(Style::default().bg(colors::BG_SIDEBAR)),
        )
        .select(
            tab_defs
                .iter()
                .position(|(_, tab, _)| *tab == active_tab)
                .unwrap_or(0),
        )
        .highlight_style(
            Style::default()
                .fg(colors::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled(
            "│",
            Style::default().fg(colors::BORDER_SUBTLE),
        ));

    f.render_widget(tabs, chunks[1]);

    // Draw tab content
    match active_tab {
        InspectorTab::Runtime => draw_runtime_tab(f, app, chunks[2]),
        InspectorTab::Validation => draw_validation_tab(f, app, chunks[2]),
        InspectorTab::Logs => draw_logs_tab(f, app, chunks[2]),
        InspectorTab::Preview => draw_preview_tab(f, app, chunks[2]),
        InspectorTab::Diff => draw_diff_tab(f, app, chunks[2]),
        InspectorTab::Steps => draw_context_tab(f, app, chunks[2]), // Context assembly view
        InspectorTab::Timeline => draw_runtime_tab(f, app, chunks[2]), // TODO: implement timeline view
        InspectorTab::Failure => draw_runtime_tab(f, app, chunks[2]), // TODO: implement failure view
        InspectorTab::PlannerTrace => draw_runtime_tab(f, app, chunks[2]), // TODO: implement planner trace view
        InspectorTab::Replay => draw_runtime_tab(f, app, chunks[2]), // TODO: implement replay view
        InspectorTab::DebugBundle => draw_runtime_tab(f, app, chunks[2]), // TODO: implement debug bundle view
        InspectorTab::Audit => draw_audit_tab(f, app, chunks[2]),
        InspectorTab::Checkpoint => draw_checkpoint_tab(f, app, chunks[2]),
        InspectorTab::Recovery => draw_runtime_tab(f, app, chunks[2]), // TODO: implement recovery tab view
    }
}

fn draw_runtime_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let state = &app.state;

    let mut lines = vec![];

    // Status section
    lines.push(Line::from(vec![Span::styled(
        "Execution Status:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    let status_text = state.execution.state.as_str();
    let status_color = match state.execution.state {
        crate::state::ExecutionState::Idle => colors::TEXT_DIM,
        crate::state::ExecutionState::Planning
        | crate::state::ExecutionState::Executing
        | crate::state::ExecutionState::Responding => colors::ACCENT,
        crate::state::ExecutionState::Validating | crate::state::ExecutionState::Repairing => {
            colors::WARNING
        }
        crate::state::ExecutionState::WaitingForApproval => colors::WARNING,
        crate::state::ExecutionState::Done => colors::SUCCESS,
        crate::state::ExecutionState::Failed
        | crate::state::ExecutionState::Blocked
        | crate::state::ExecutionState::PreconditionFailed => colors::ERROR,
    };

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("State", Style::default().fg(colors::TEXT_DIM)),
        Span::styled(
            format!(" {}", status_text),
            Style::default().fg(colors::TEXT_SOFT),
        ),
        Span::styled("  Mode", Style::default().fg(colors::TEXT_DIM)),
        Span::styled(
            format!(" {}", state.execution.mode.as_str()),
            Style::default().fg(status_color),
        ),
    ]));
    lines.push(detail_line(
        "Objective",
        state
            .execution
            .active_objective
            .as_deref()
            .unwrap_or("none"),
    ));
    if let Some(step) = state.execution.current_step.as_deref() {
        lines.push(detail_line("Current", step));
    }
    if let Some(tool) = state.execution.active_tool.as_deref() {
        lines.push(detail_line("Action", tool));
    }

    // UNIFIED EXECUTION PLAN - Show steps if available
    if let Some(plan) = state.execution.current_plan.as_ref() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("▶ {}", plan.objective),
            Style::default()
                .fg(colors::ACCENT)
                .add_modifier(Modifier::BOLD),
        )]));

        // RLEF TRANSPARENCY - Show if plan was influenced by learning
        if let Some(ref transparency) = state.rlef_transparency
            && transparency.influenced
        {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "Plan influenced by prior execution feedback:",
                Style::default().fg(colors::TEXT_DIM),
            )]));

            for hint_trans in &transparency.applied_hints {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  Step {} ", hint_trans.step_index + 1),
                        Style::default().fg(colors::TEXT_DIM),
                    ),
                    Span::styled(
                        format!("→ {}", hint_trans.influence_description),
                        Style::default().fg(colors::SUCCESS),
                    ),
                ]));
                lines.push(Line::from(vec![Span::styled(
                    format!("     💡 {}", hint_trans.hint.guidance),
                    Style::default().fg(colors::TEXT_SOFT),
                )]));
            }

            lines.push(Line::from(""));
        }

        for (i, step) in plan.steps.iter().enumerate() {
            let (indicator, color) = match step.status {
                crate::state::ExecutionStepStatus::Pending => ("○", colors::TEXT_DIM),
                crate::state::ExecutionStepStatus::Running => ("▶", colors::ACCENT),
                crate::state::ExecutionStepStatus::Completed => ("✓", colors::SUCCESS),
                crate::state::ExecutionStepStatus::Failed => ("✗", colors::ERROR),
            };

            // Action type icon
            let action_icon = match step.action {
                crate::state::StepAction::None => "",
                crate::state::StepAction::Parse => "[∷]",
                crate::state::StepAction::Validate => "[✓]",
                crate::state::StepAction::CreateDirectory { .. } => "[📁]",
                crate::state::StepAction::WriteFile { .. } => "[📝]",
                crate::state::StepAction::ReadFile { .. } => "[📄]",
                crate::state::StepAction::PatchFile { .. } => "[✎]",
                crate::state::StepAction::RunCommand { .. } => "[⚡]",
                crate::state::StepAction::Plan => "[◈]",
                crate::state::StepAction::Chat => "[◉]",
                crate::state::StepAction::Search { .. } => "[🔍]",
                crate::state::StepAction::ValidateProject => "[✓]",
                crate::state::StepAction::Install { .. } => "[📦]",
                crate::state::StepAction::StartServer { .. } => "[▶]",
                crate::state::StepAction::Build => "[🔨]",
                crate::state::StepAction::Test => "[🧪]",
                crate::state::StepAction::Git { .. } => "[⎇]",
                crate::state::StepAction::Fix { .. } => "[🔧]",
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", indicator), Style::default().fg(color)),
                Span::styled(
                    format!("{} ", action_icon),
                    Style::default().fg(colors::TEXT_DIM),
                ),
                Span::styled(
                    format!("{}.{}", i + 1, step.description),
                    Style::default().fg(
                        if matches!(step.status, crate::state::ExecutionStepStatus::Running) {
                            colors::TEXT_SOFT
                        } else {
                            colors::TEXT_MUTED
                        },
                    ),
                ),
            ]));

            // Show step result metadata (proof of completion)
            if let Some(result) = step.result.as_ref() {
                let result_text = if !result.affected_paths.is_empty() {
                    format!(
                        "→ {}",
                        result
                            .affected_paths
                            .join(", ")
                            .chars()
                            .take(45)
                            .collect::<String>()
                    )
                } else if !result.artifact_urls.is_empty() {
                    format!("→ {}", result.artifact_urls.join(", "))
                } else if let Some(code) = result.exit_code {
                    format!("→ exit {}", code)
                } else if let Some(bytes) = result.bytes_affected {
                    if bytes < 1024 {
                        format!("→ {} bytes", bytes)
                    } else {
                        format!("→ {:.1} KB", bytes as f64 / 1024.0)
                    }
                } else if let Some(ref val) = result.validation_result {
                    format!("→ {}", val)
                } else {
                    result.success_summary()
                };

                if !result_text.is_empty() && result_text != "completed" {
                    lines.push(Line::from(vec![Span::styled(
                        format!("     {}", result_text),
                        Style::default().fg(colors::SUCCESS),
                    )]));
                }
            }

            // Show step output if completed and different from result
            if let Some(output) = step.output.as_ref()
                && !output.is_empty()
                && step.result.is_none()
            {
                lines.push(Line::from(vec![Span::styled(
                    format!(
                        "     {}",
                        output
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(50)
                            .collect::<String>()
                    ),
                    Style::default().fg(colors::TEXT_DIM),
                )]));
            }
        }

        // Show final result if complete
        if let Some(result) = plan.final_result.as_ref() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("Result: {}", result),
                Style::default().fg(colors::SUCCESS),
            )]));
        }
    }

    lines.push(Line::from(""));

    // Activity section
    lines.push(Line::from(vec![Span::styled(
        "Activity Log:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    if state.execution.planner_output.is_empty() && state.execution.tool_calls.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  Waiting for execution...",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    } else {
        for item in state.execution.planner_output.iter().rev() {
            lines.push(detail_line("Plan", item));
        }
        for item in state.execution.tool_calls.iter().rev() {
            lines.push(detail_line("Exec", item));
        }
    }

    lines.push(Line::from(""));

    // Live Events section
    lines.push(Line::from(vec![Span::styled(
        "System Events:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    if state.runtime_events.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  Waiting for runtime activity...",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    } else {
        // Show all events with icons and colors (scrollable)
        for event in state.runtime_events.iter().rev() {
            let time = state.format_time(&event.timestamp);
            let (indicator, color) = runtime_indicator(&event.stage);

            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} {} ", time, indicator),
                    Style::default().fg(colors::TEXT_DIM),
                ),
                Span::styled(
                    display_runtime_stage(&event.stage),
                    Style::default().fg(color),
                ),
            ]));
        }
    }

    // Show execution output buffer if active
    if !app.execution_output_buffer.is_empty() {
        lines.push(Line::from(vec![]));
        lines.push(Line::from(vec![Span::styled(
            "Execution Output:",
            Style::default()
                .fg(colors::TEXT_MUTED)
                .add_modifier(Modifier::BOLD),
        )]));
        for line in app.execution_output_buffer.iter().rev().take(20) {
            let truncated: String = line.chars().take(60).collect::<String>();
            lines.push(Line::from(vec![Span::styled(
                format!("  › {}", truncated),
                Style::default().fg(colors::TEXT_SOFT),
            )]));
        }
    }

    // Inspector Interaction Layer - Add actionable hints
    lines.push(Line::from(vec![]));
    lines.push(Line::from(vec![Span::styled(
        "Inspector Commands:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    // Get active chain info for contextual hints
    let has_active_chain = app.persistence.active_chain_id.is_some();
    let has_failed_steps = app
        .state
        .execution
        .current_plan
        .as_ref()
        .map(|p| {
            p.steps
                .iter()
                .any(|s| matches!(s.status, crate::state::ExecutionStepStatus::Failed))
        })
        .unwrap_or(false);

    if has_active_chain {
        lines.push(Line::from(vec![
            Span::styled("  /chain status  ", Style::default().fg(colors::ACCENT)),
            Span::styled("View chain details", Style::default().fg(colors::TEXT_DIM)),
        ]));

        if has_failed_steps {
            lines.push(Line::from(vec![
                Span::styled("  /replay        ", Style::default().fg(colors::ACCENT)),
                Span::styled(
                    "Inspect execution replay",
                    Style::default().fg(colors::TEXT_DIM),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  /replay diff N ", Style::default().fg(colors::ACCENT)),
                Span::styled(
                    "Compare step N for divergence",
                    Style::default().fg(colors::TEXT_DIM),
                ),
            ]));
        }

        lines.push(Line::from(vec![
            Span::styled("  /git status    ", Style::default().fg(colors::ACCENT)),
            Span::styled(
                "Show repository state",
                Style::default().fg(colors::TEXT_DIM),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  /chains        ", Style::default().fg(colors::ACCENT)),
            Span::styled(
                "List available chains",
                Style::default().fg(colors::TEXT_DIM),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  /plan          ", Style::default().fg(colors::ACCENT)),
            Span::styled(
                "Create a new task plan",
                Style::default().fg(colors::TEXT_DIM),
            ),
        ]));
    }

    // Calculate content height for scrollbar
    let content_height = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize; // Account for borders

    // Create scrollable paragraph with current scroll offset
    let scroll_offset = state
        .runtime_tab_scroll
        .min(content_height.saturating_sub(1));
    let paragraph = Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll_offset as u16, 0))
        .block(
            Block::default()
                .title(" Runtime ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER_SUBTLE))
                .style(Style::default().bg(colors::BG_PANEL)),
        );

    f.render_widget(paragraph, area);

    // Render scrollbar if content exceeds visible area
    if content_height > visible_height {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(scroll_offset)
            .content_length(content_height)
            .viewport_content_length(visible_height);

        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}

/// Draw the Context Assembly tab showing V3 authority metadata
fn draw_context_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let mut lines = vec![];

    // Get active chain context info
    let chain_info = app.persistence.get_active_chain().map(|chain| {
        let context_summary = if let Some(ref context) = chain.context_state {
            let included = context.files.iter().filter(|f| f.included).count();
            let trimmed = context.files.len() - included;
            let status_icon = match context.validation.status {
                crate::persistence::ContextValidationStatus::Valid => "✓",
                crate::persistence::ContextValidationStatus::Warning => "⚠",
                crate::persistence::ContextValidationStatus::Invalid => "✗",
            };
            Some((
                format!("{} {} files ({} trimmed)", status_icon, included, trimmed),
                format!(
                    "{}/{} tokens",
                    context.budget.tokens_used, context.budget.max_tokens
                ),
                context.summary.clone(),
                included,
                trimmed,
            ))
        } else if !chain.selected_context_files.is_empty() {
            Some((
                format!("{} files (V2)", chain.selected_context_files.len()),
                "unknown tokens".to_string(),
                "Context V2 assembly".to_string(),
                chain.selected_context_files.len(),
                0usize,
            ))
        } else {
            None
        };
        (chain.name.clone(), chain.status, context_summary)
    });

    // Header section
    lines.push(Line::from(vec![Span::styled(
        "Context Assembly:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    if let Some((chain_name, chain_status, context_opt)) = chain_info {
        let status_icon = match chain_status {
            crate::persistence::ChainLifecycleStatus::Running => "▶",
            crate::persistence::ChainLifecycleStatus::Complete => "✓",
            crate::persistence::ChainLifecycleStatus::Failed => "✗",
            crate::persistence::ChainLifecycleStatus::Halted => "⏸",
            _ => "○",
        };

        lines.push(detail_line(
            "Chain",
            &format!("{} {}", status_icon, chain_name),
        ));

        if let Some((summary, budget, description, included, trimmed)) = context_opt {
            lines.push(detail_line("Files", &summary));
            lines.push(detail_line("Budget", &budget));
            lines.push(detail_line("Summary", &description));
            lines.push(Line::from(""));

            // Show file list from V3 state
            if let Some(ref chain) = app.persistence.get_active_chain() {
                if let Some(ref context) = chain.context_state {
                    if included > 0 {
                        lines.push(Line::from(vec![Span::styled(
                            "Selected Files:",
                            Style::default()
                                .fg(colors::TEXT_MUTED)
                                .add_modifier(Modifier::BOLD),
                        )]));

                        for file in context.files.iter().filter(|f| f.included).take(20) {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  • [{}] ", file.priority),
                                    Style::default().fg(colors::TEXT_DIM),
                                ),
                                Span::styled(
                                    truncate_inline(&file.path, 40),
                                    Style::default().fg(colors::TEXT_SOFT),
                                ),
                            ]));
                            // Show reason on next line if space permits
                            if !file.reason.is_empty()
                                && file.reason != "Selected by context assembly"
                            {
                                lines.push(Line::from(vec![Span::styled(
                                    format!("    → {}", truncate_inline(&file.reason, 45)),
                                    Style::default().fg(colors::TEXT_DIM),
                                )]));
                            }
                        }

                        if included > 20 {
                            lines.push(Line::from(vec![Span::styled(
                                format!("  ... and {} more files", included - 20),
                                Style::default().fg(colors::TEXT_DIM),
                            )]));
                        }
                    }

                    // Show trimmed files if any
                    if trimmed > 0 {
                        lines.push(Line::from(""));
                        lines.push(Line::from(vec![Span::styled(
                            "Trimmed Files:",
                            Style::default()
                                .fg(colors::WARNING)
                                .add_modifier(Modifier::BOLD),
                        )]));

                        for file in context
                            .files
                            .iter()
                            .filter(|f| !f.included && f.trimmed_reason.is_some())
                            .take(10)
                        {
                            lines.push(Line::from(vec![
                                Span::styled("  • ", Style::default().fg(colors::TEXT_DIM)),
                                Span::styled(
                                    truncate_inline(&file.path, 35),
                                    Style::default().fg(colors::TEXT_DIM),
                                ),
                                Span::styled(
                                    format!(" → {}", file.trimmed_reason.as_ref().unwrap()),
                                    Style::default().fg(colors::WARNING),
                                ),
                            ]));
                        }
                    }

                    // Show validation warnings/errors
                    if !context.validation.warnings.is_empty() {
                        lines.push(Line::from(""));
                        lines.push(Line::from(vec![Span::styled(
                            "Warnings:",
                            Style::default()
                                .fg(colors::WARNING)
                                .add_modifier(Modifier::BOLD),
                        )]));
                        for warning in &context.validation.warnings {
                            lines.push(Line::from(vec![Span::styled(
                                format!("  ⚠ {}", warning),
                                Style::default().fg(colors::WARNING),
                            )]));
                        }
                    }

                    if !context.validation.errors.is_empty() {
                        lines.push(Line::from(""));
                        lines.push(Line::from(vec![Span::styled(
                            "Errors:",
                            Style::default()
                                .fg(colors::ERROR)
                                .add_modifier(Modifier::BOLD),
                        )]));
                        for error in &context.validation.errors {
                            lines.push(Line::from(vec![Span::styled(
                                format!("  ✗ {}", error),
                                Style::default().fg(colors::ERROR),
                            )]));
                        }
                    }
                } else {
                    // V2 fallback - just show file list
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![Span::styled(
                        "Selected Files (V2):",
                        Style::default()
                            .fg(colors::TEXT_MUTED)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    for file in &chain.selected_context_files {
                        lines.push(Line::from(vec![
                            Span::styled("  • ", Style::default().fg(colors::TEXT_DIM)),
                            Span::styled(
                                truncate_inline(file, 50),
                                Style::default().fg(colors::TEXT_SOFT),
                            ),
                        ]));
                    }
                }
            }
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "Context not yet assembled for this chain.",
                Style::default().fg(colors::TEXT_DIM),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "Start a task to trigger context assembly.",
                Style::default().fg(colors::TEXT_DIM),
            )]));
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "No active chain.",
            Style::default().fg(colors::TEXT_DIM),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "Create a chain with /task or select one with /chains.",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    }

    // Calculate content height for scrollbar
    let _content_height = lines.len();
    let _visible_height = area.height.saturating_sub(2) as usize;

    // Create scrollable paragraph
    let paragraph = Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(
            Block::default()
                .title(" Context ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER_SUBTLE))
                .style(Style::default().bg(colors::BG_PANEL)),
        );

    f.render_widget(paragraph, area);
}

/// V1.6 AUDIT: Draw the audit timeline tab showing execution history
fn draw_audit_tab(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::state::AuditEventType;

    let mut lines = vec![];

    // Header
    lines.push(Line::from(vec![Span::styled(
        "Execution Audit Timeline",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Get active chain and its audit log
    if let Some(chain_id) = app.persistence.active_chain_id.as_ref() {
        if let Some(chain) = app.persistence.get_chain(chain_id) {
            let audit_log = &chain.audit_log;

            if audit_log.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    "No audit events recorded for this chain.",
                    Style::default().fg(colors::TEXT_DIM),
                )]));
            } else {
                // Show recent events (last 50)
                let events = audit_log.get_last_n(50);
                let total_events = audit_log.len();

                lines.push(Line::from(vec![Span::styled(
                    format!(
                        "Showing {} of {} events (newest first)",
                        events.len(),
                        total_events
                    ),
                    Style::default().fg(colors::TEXT_DIM),
                )]));
                lines.push(Line::from(""));

                for event in events {
                    // Format timestamp
                    let ts = event.timestamp.format("%H:%M:%S%.3f");

                    // Get event styling based on type
                    let (icon, style) = match event.event_type {
                        AuditEventType::StateTransitionApplied => {
                            ("→", Style::default().fg(colors::SUCCESS))
                        }
                        AuditEventType::StateTransitionRejected => {
                            ("✗", Style::default().fg(colors::ERROR))
                        }
                        AuditEventType::StateTransitionNormalized => {
                            ("~", Style::default().fg(colors::WARNING))
                        }
                        AuditEventType::OutcomeFinalized => (
                            "★",
                            Style::default()
                                .fg(colors::ACCENT)
                                .add_modifier(Modifier::BOLD),
                        ),
                        AuditEventType::StepStarted => {
                            ("▶", Style::default().fg(colors::TEXT_SOFT))
                        }
                        AuditEventType::StepCompleted => {
                            ("✓", Style::default().fg(colors::SUCCESS))
                        }
                        AuditEventType::ApprovalRequested => {
                            ("⏸", Style::default().fg(colors::WARNING))
                        }
                        AuditEventType::ApprovalResolved => {
                            ("✓", Style::default().fg(colors::SUCCESS))
                        }
                        AuditEventType::RepairTriggered => {
                            ("🔧", Style::default().fg(colors::WARNING))
                        }
                        AuditEventType::RepairCompleted => {
                            ("✓", Style::default().fg(colors::SUCCESS))
                        }
                        AuditEventType::ValidationStarted => {
                            ("▶", Style::default().fg(colors::TEXT_SOFT))
                        }
                        AuditEventType::ValidationCompleted => {
                            ("✓", Style::default().fg(colors::SUCCESS))
                        }
                        _ => ("•", Style::default().fg(colors::TEXT_DIM)),
                    };

                    // Build main line with icon and event type
                    let event_name = format!("{:?}", event.event_type);
                    let event_name = event_name.split("(").next().unwrap_or(&event_name);
                    let event_name = event_name.replace("AuditEventType::", "");

                    let mut spans = vec![
                        Span::styled(format!("[{}] ", ts), Style::default().fg(colors::TEXT_DIM)),
                        Span::styled(format!("{} ", icon), style),
                        Span::styled(event_name, style),
                    ];

                    // Add state transition info if present
                    if let (Some(prev), Some(next)) = (event.previous_state, event.next_state) {
                        let prev_str = format!("{:?}", prev).replace("ExecutionState::", "");
                        let next_str = format!("{:?}", next).replace("ExecutionState::", "");
                        spans.push(Span::styled(
                            format!("  {} → {}", prev_str, next_str),
                            Style::default().fg(colors::TEXT_SOFT),
                        ));
                    }

                    lines.push(Line::from(spans));

                    // Add triggering event on next line if present
                    if let Some(trigger) = &event.triggering_event {
                        lines.push(Line::from(vec![
                            Span::styled("       Trigger: ", Style::default().fg(colors::TEXT_DIM)),
                            Span::styled(trigger.clone(), Style::default().fg(colors::TEXT_SOFT)),
                        ]));
                    }

                    // Add reason on next line if present
                    if let Some(reason) = &event.reason {
                        lines.push(Line::from(vec![
                            Span::styled("       Reason: ", Style::default().fg(colors::TEXT_DIM)),
                            Span::styled(reason.clone(), Style::default().fg(colors::WARNING)),
                        ]));
                    }

                    // Add step/task context if present
                    if event.step_id.is_some() || event.task.is_some() {
                        let mut context_parts = vec![];
                        if let Some(step) = &event.step_id {
                            context_parts
                                .push(format!("step={}", crate::text::take_chars(step, 8)));
                        }
                        if let Some(task) = &event.task {
                            context_parts
                                .push(format!("task=\"{}\"", crate::text::take_chars(task, 30)));
                        }
                        lines.push(Line::from(vec![
                            Span::styled("       Context: ", Style::default().fg(colors::TEXT_DIM)),
                            Span::styled(
                                context_parts.join(", "),
                                Style::default().fg(colors::TEXT_DIM),
                            ),
                        ]));
                    }

                    // Add blank line between events
                    lines.push(Line::from(""));
                }

                // Show transition summary
                let transitions = audit_log.get_transition_history();
                if !transitions.is_empty() {
                    lines.push(Line::from(vec![Span::styled(
                        "Transition Summary:",
                        Style::default()
                            .fg(colors::TEXT_MUTED)
                            .add_modifier(Modifier::BOLD),
                    )]));

                    for t in transitions.iter().rev().take(10) {
                        if let (Some(prev), Some(next)) = (t.previous_state, t.next_state) {
                            let prev_str = format!("{:?}", prev).replace("ExecutionState::", "");
                            let next_str = format!("{:?}", next).replace("ExecutionState::", "");
                            let ts = t.timestamp.format("%H:%M:%S");

                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("[{}] ", ts),
                                    Style::default().fg(colors::TEXT_DIM),
                                ),
                                Span::styled(
                                    format!("{} → {}", prev_str, next_str),
                                    Style::default().fg(colors::TEXT_SOFT),
                                ),
                            ]));
                        }
                    }
                }
            }
        } else {
            lines.push(Line::from(vec![Span::styled(
                "No active chain found.",
                Style::default().fg(colors::TEXT_DIM),
            )]));
        }
    } else {
        lines.push(Line::from(vec![Span::styled(
            "No active chain. Create a chain with /task or /plan to see audit timeline.",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    }

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .scroll((app.state.audit_tab_scroll as u16, 0))
        .style(Style::default().bg(colors::BG_PANEL));

    f.render_widget(paragraph, area);
}

fn draw_validation_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let state = &app.state;

    let mut lines = vec![];

    lines.push(Line::from(vec![Span::styled(
        "Validation Status:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(detail_line(
        "STATE",
        state
            .execution
            .validation_summary
            .as_deref()
            .unwrap_or("none"),
    ));
    lines.push(detail_line(
        "BLOCK",
        if state.execution.state == crate::state::ExecutionState::Failed {
            "blocking"
        } else {
            "not blocking"
        },
    ));
    lines.push(Line::from(""));

    // Show validation stages with icons
    lines.push(Line::from(vec![Span::styled(
        "Validation Pipeline:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    for stage in &state.validation_stages {
        let (indicator, color) = match stage.status {
            RuntimeStatus::Idle => ("[--]", colors::TEXT_DIM),
            RuntimeStatus::Running => ("[run]", colors::ACCENT),
            RuntimeStatus::Completed => ("[OK]", colors::SUCCESS),
            RuntimeStatus::Error => ("[FAIL]", colors::ERROR),
        };

        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", indicator), Style::default().fg(color)),
            Span::styled(
                format!("{:<10} ", display_validation_stage_name(&stage.name)),
                Style::default().fg(colors::TEXT_SOFT),
            ),
            Span::styled(
                validation_stage_description(&stage.name),
                Style::default().fg(colors::TEXT_DIM),
            ),
        ]));

        if let Some(detail) = stage.detail.as_ref() {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(detail, Style::default().fg(colors::TEXT_DIM)),
            ]));
        }
    }

    lines.push(Line::from(vec![]));

    // Show last validation result
    lines.push(Line::from(vec![Span::styled(
        "Last Result:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    let last_validation = latest_validation_event(state);

    if let Some(event) = last_validation {
        let status_str = match event.status {
            RuntimeStatus::Idle => "idle",
            RuntimeStatus::Running => "running",
            RuntimeStatus::Completed => "passed",
            RuntimeStatus::Error => "failed",
        };
        let stage_key = event
            .stage
            .rsplit('/')
            .next()
            .unwrap_or(event.stage.as_str());
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {}: {}",
                display_validation_stage_name(stage_key),
                status_str
            ),
            Style::default().fg(colors::TEXT_SOFT),
        )]));
        if let Some(detail) = state
            .validation_stages
            .iter()
            .find(|stage| stage.name == stage_key)
            .and_then(|stage| stage.detail.as_ref())
        {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(detail, Style::default().fg(colors::TEXT_DIM)),
            ]));
        }
    } else {
        lines.push(Line::from(vec![Span::styled(
            "  Waiting for validation...",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    }

    // Calculate content height for scrollbar
    let content_height = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;

    // Create scrollable paragraph with current scroll offset
    let scroll_offset = state
        .validation_tab_scroll
        .min(content_height.saturating_sub(1));
    let paragraph = Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll_offset as u16, 0))
        .block(
            Block::default()
                .title(" Validation ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER_SUBTLE))
                .style(Style::default().bg(colors::BG_PANEL)),
        );

    f.render_widget(paragraph, area);

    // Render scrollbar if content exceeds visible area
    if content_height > visible_height {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(scroll_offset)
            .content_length(content_height)
            .viewport_content_length(visible_height);

        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}

fn latest_validation_event(state: &AppState) -> Option<&RuntimeEvent> {
    state
        .runtime_events
        .iter()
        .rev()
        .find(|event| event.stage.starts_with("validation/"))
}

fn runtime_indicator(stage: &str) -> (&'static str, ratatui::style::Color) {
    if stage.starts_with("init") || stage.starts_with("preflight") {
        ("[start]", colors::ACCENT)
    } else if stage.starts_with("planner") {
        ("[plan]", colors::ACCENT_SOFT)
    } else if stage.starts_with("validation") {
        ("[check]", colors::WARNING)
    } else if stage.starts_with("tool/") {
        ("[tool]", colors::ACCENT)
    } else if stage.starts_with("mutation") {
        ("[edit]", colors::TEXT_SOFT)
    } else if stage.starts_with("commit") {
        ("[save]", colors::SUCCESS)
    } else if stage.starts_with("completion") || stage.starts_with("finished") {
        ("[done]", colors::SUCCESS)
    } else if stage.starts_with("failure") {
        ("[FAIL]", colors::ERROR)
    } else if stage.starts_with("repair") {
        ("[retry]", colors::WARNING)
    } else {
        ("[--]", colors::TEXT_DIM)
    }
}

fn display_runtime_stage(stage: &str) -> String {
    stage.replace('/', " / ")
}

fn display_validation_stage_name(name: &str) -> String {
    match name {
        "protocol" => "Protocol".to_string(),
        "validation" => "Runtime".to_string(),
        _ => {
            let mut chars = name.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
    }
}

fn validation_stage_description(name: &str) -> &'static str {
    match name {
        "protocol" => "planner contract",
        "validation" => "runtime checks",
        "syntax" => "parser",
        "lint" => "style",
        "build" => "compiler",
        "test" => "runner",
        _ => "stage",
    }
}

fn draw_logs_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let state = &app.state;

    let mut lines = vec![];

    lines.push(Line::from(vec![Span::styled(
        "Execution Feeds:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));
    if state.execution.planner_output.is_empty()
        && state.execution.tool_calls.is_empty()
        && state.execution.file_writes.is_empty()
    {
        lines.push(Line::from(vec![Span::styled(
            "  No planner output, tool calls, or file writes yet",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    } else {
        for item in state.execution.planner_output.iter().rev() {
            lines.push(detail_line("planner", item));
        }
        for item in state.execution.tool_calls.iter().rev() {
            lines.push(detail_line("tool", item));
        }
        for item in state.execution.file_writes.iter().rev() {
            lines.push(detail_line("write", item));
        }
    }

    lines.push(Line::from(""));

    if state.logs.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "No logs yet",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    } else {
        // Show all logs (scrollable)
        for log in state.logs.iter().rev() {
            let time = state.format_time(&log.timestamp);
            let level_color = match log.level {
                crate::state::LogLevel::Debug => colors::TEXT_DIM,
                crate::state::LogLevel::Info => colors::TEXT_MUTED,
                crate::state::LogLevel::Warn => colors::WARNING,
                crate::state::LogLevel::Error => colors::ERROR,
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("[{}] ", time),
                    Style::default().fg(colors::TEXT_DIM),
                ),
                Span::styled(
                    format!("[{}]", log.level.as_str()),
                    Style::default().fg(level_color),
                ),
                Span::styled(
                    format!(" {}: {}", log.source, log.message),
                    Style::default().fg(colors::TEXT_MUTED),
                ),
            ]));
        }
    }

    // Calculate content height for scrollbar
    let content_height = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;

    // Create scrollable paragraph with current scroll offset
    let scroll_offset = state.logs_tab_scroll.min(content_height.saturating_sub(1));
    let paragraph = Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll_offset as u16, 0))
        .block(
            Block::default()
                .title(" Logs ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER_SUBTLE))
                .style(Style::default().bg(colors::BG_PANEL)),
        );

    f.render_widget(paragraph, area);

    // Render scrollbar if content exceeds visible area
    if content_height > visible_height {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(scroll_offset)
            .content_length(content_height)
            .viewport_content_length(visible_height);

        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}

fn draw_checkpoint_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let mut lines = vec![];
    lines.push(Line::from(vec![Span::styled(
        "Checkpoint Validation",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    if let Some(report) = app.checkpoint_inspector_report.as_ref() {
        let status_style = match report.final_status {
            crate::persistence::CheckpointOperatorStatus::Valid => {
                Style::default().fg(colors::SUCCESS)
            }
            crate::persistence::CheckpointOperatorStatus::Stale
            | crate::persistence::CheckpointOperatorStatus::Divergent => {
                Style::default().fg(colors::WARNING)
            }
            crate::persistence::CheckpointOperatorStatus::Corrupted
            | crate::persistence::CheckpointOperatorStatus::Missing => {
                Style::default().fg(colors::ERROR)
            }
        };

        lines.push(Line::from(vec![
            Span::styled("  Status   ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                format!(
                    "{} {}",
                    report.final_status.icon(),
                    report.final_status.label()
                ),
                status_style.add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(detail_line(
            "Resume",
            if report.resume_allowed {
                "allowed"
            } else {
                "blocked"
            },
        ));
        lines.push(detail_line("Chain", &report.chain_id));
        lines.push(detail_line(
            "Checkpoint",
            report.checkpoint_id.as_deref().unwrap_or("(missing)"),
        ));
        if let Some(timestamp) = report.checkpoint_timestamp {
            lines.push(detail_line(
                "Created",
                &timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            ));
        }
        let step = report
            .active_step
            .map(|idx| idx.to_string())
            .unwrap_or_else(|| "(none)".to_string());
        lines.push(detail_line("Step", &step));
        lines.push(detail_line(
            "Step Desc",
            report.step_description.as_deref().unwrap_or("(unknown)"),
        ));
        lines.push(detail_line(
            "Cursor",
            &format!(
                "{} / {}",
                report
                    .audit_cursor
                    .map(|cursor| cursor.to_string())
                    .unwrap_or_else(|| "(none)".to_string()),
                report.audit_log_len
            ),
        ));
        lines.push(detail_line(
            "Hash",
            &report.workspace_hash.as_deref().map_or_else(
                || "(none)".to_string(),
                |hash| crate::text::take_chars(hash, 16),
            ),
        ));
        lines.push(Line::from(""));
        lines.push(detail_line(
            "Workspace",
            &format!(
                "{} - {}",
                report.workspace_result.status.label(),
                report.workspace_result.detail
            ),
        ));
        lines.push(detail_line(
            "Replay",
            &format!(
                "{} - {}",
                report.replay_result.status.label(),
                report.replay_result.detail
            ),
        ));
        lines.push(Line::from(""));
        lines.push(detail_line("Next", &report.smallest_safe_next_action));
    } else {
        lines.push(Line::from(vec![Span::styled(
            "  Run /checkpoint status or /chain status to inspect checkpoint truth.",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    }

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Checkpoint")
                .border_style(Style::default().fg(colors::BORDER_SUBTLE)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<9}", label),
            Style::default().fg(colors::TEXT_DIM),
        ),
        Span::styled(value.to_string(), Style::default().fg(colors::TEXT_SOFT)),
    ])
}

fn draw_preview_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let state = &app.state;

    let mut lines = vec![];

    lines.push(Line::from(vec![Span::styled(
        "Structured Results:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    if state.structured_outputs.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  No structured output routed yet",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    } else {
        for output in state.structured_outputs.iter().rev().take(3) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", output.kind.label()),
                    Style::default().fg(colors::ACCENT),
                ),
                Span::styled(
                    crate::text::truncate_chars(&output.title, 72),
                    Style::default().fg(colors::TEXT_SOFT),
                ),
            ]));
            for content_line in output.content.lines().take(12) {
                if content_line.trim().is_empty() {
                    lines.push(Line::from(""));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(
                            crate::text::truncate_chars(content_line, 96),
                            Style::default().fg(colors::TEXT_MUTED),
                        ),
                    ]));
                }
            }
            if output.content.lines().count() > 12 {
                lines.push(Line::from(vec![Span::styled(
                    "    ...",
                    Style::default().fg(colors::TEXT_DIM),
                )]));
            }
            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(""));

    // Preview servers section
    lines.push(Line::from(vec![Span::styled(
        "Active Preview Servers:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));

    // Collect all needed data upfront to avoid borrow issues
    let is_empty = state.preview_servers.is_empty();
    let scroll_offset_val = state.preview_tab_scroll;

    if is_empty {
        lines.push(Line::from(vec![Span::styled(
            "  No active preview servers",
            Style::default().fg(colors::TEXT_DIM),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Use browser_preview tool to start a server",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    } else {
        // Collect server data first
        let server_data: Vec<(String, String, u16, String)> = state
            .preview_servers
            .iter()
            .map(|s| {
                (
                    state.format_time(&s.started_at),
                    s.url.clone(),
                    s.port(),
                    s.directory().to_string(),
                )
            })
            .collect();

        // Drop immutable borrow of state before mutable borrow of app
        // Server data collected to avoid borrow issues

        for (idx, (time, url, port, directory)) in server_data.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  [{}] ", time),
                    Style::default().fg(colors::TEXT_DIM),
                ),
                Span::styled("● ", Style::default().fg(colors::SUCCESS)),
                Span::styled(
                    format!("{} ", url),
                    Style::default()
                        .fg(colors::ACCENT)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled("(click to open)", Style::default().fg(colors::TEXT_DIM)),
            ]));
            lines.push(Line::from(vec![
                Span::raw("     "),
                Span::styled(
                    format!("Port: {}, Dir: {}", port, directory),
                    Style::default().fg(colors::TEXT_SOFT),
                ),
            ]));
            lines.push(Line::from(""));

            // Register clickable UI target for this URL
            let line_y = area.y + 1 + (idx * 3) as u16;
            if line_y < area.y + area.height {
                let url_area = Rect {
                    x: area.x + 2,
                    y: line_y,
                    width: area.width.saturating_sub(4),
                    height: 1,
                };
                app.register_ui_target(
                    format!("preview:url:{}", idx),
                    url_area,
                    UiAction::OpenBrowserPreview {
                        url: url.clone().to_string(),
                    },
                );
            }
        }

        // Add hint at the bottom
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("💡 ", Style::default().fg(colors::ACCENT)),
            Span::styled(
                "Click a URL above to open in browser (or press 'o' key)".to_string(),
                Style::default().fg(colors::TEXT_DIM),
            ),
        ]));
    }

    // Calculate content height for scrollbar
    let content_height = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;

    // Create scrollable paragraph with current scroll offset (using cloned value)
    let scroll_offset = scroll_offset_val.min(content_height.saturating_sub(1));
    let paragraph = Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll_offset as u16, 0))
        .block(
            Block::default()
                .title(" Results / Preview ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER_SUBTLE))
                .style(Style::default().bg(colors::BG_PANEL)),
        );

    f.render_widget(paragraph, area);

    // Render scrollbar if content exceeds visible area
    if content_height > visible_height {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(scroll_offset)
            .content_length(content_height)
            .viewport_content_length(visible_height);

        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}

fn draw_diff_tab(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::diff::unified_diff;

    let state = &app.state;
    let scroll_offset_val = state.diff_tab_scroll;

    let mut lines = vec![];

    // Header
    lines.push(Line::from(vec![Span::styled(
        "Recent File Changes:",
        Style::default()
            .fg(colors::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    if state.diff_store.mutations.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  No file changes yet",
            Style::default().fg(colors::TEXT_DIM),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "File mutations will appear here when the planner makes changes",
            Style::default().fg(colors::TEXT_DIM),
        )]));
    } else {
        // Show latest mutation with full diff
        if let Some(latest) = state.diff_store.latest() {
            lines.push(Line::from(vec![
                Span::styled("Latest: ", Style::default().fg(colors::TEXT_MUTED)),
                Span::styled(&latest.path, Style::default().add_modifier(Modifier::BOLD)),
            ]));
            lines.push(Line::from(""));

            // Generate unified diff
            let diff_lines = unified_diff(latest, 3);
            for line in diff_lines {
                lines.push(line);
            }
        }

        // Show syntax highlighted preview of the latest file
        if let Some(latest) = state.diff_store.latest() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "Highlighted Preview:",
                Style::default()
                    .fg(colors::TEXT_MUTED)
                    .add_modifier(Modifier::BOLD),
            )]));

            // Show first 20 lines with syntax highlighting
            let highlighted = crate::syntax::highlight_file(&latest.path, &latest.after);
            for line in highlighted.iter().take(20) {
                let mut prefixed = vec![Span::styled("  ", Style::default())];
                prefixed.extend(line.spans.clone());
                lines.push(Line::from(prefixed));
            }
            if highlighted.len() > 20 {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled("... (truncated)", Style::default().fg(colors::TEXT_DIM)),
                ]));
            }
        }

        // List all mutations
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "All Changes:",
            Style::default()
                .fg(colors::TEXT_MUTED)
                .add_modifier(Modifier::BOLD),
        )]));

        for mutation in &state.diff_store.mutations {
            let summary = crate::diff::compact_diff_summary(mutation);
            let indicator = if mutation.before.is_none() {
                Span::styled("[NEW] ", Style::default().fg(colors::SUCCESS))
            } else if mutation.after.is_empty() {
                Span::styled("[DEL] ", Style::default().fg(colors::ERROR))
            } else {
                Span::styled("[MOD] ", Style::default().fg(colors::WARNING))
            };

            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                indicator,
                Span::styled(summary, Style::default().fg(colors::TEXT_SOFT)),
            ]));
        }
    }

    // Calculate content height for scrollbar
    let content_height = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;

    // Create scrollable paragraph with current scroll offset
    let scroll_offset = scroll_offset_val.min(content_height.saturating_sub(1));
    let paragraph = Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll_offset as u16, 0))
        .block(
            Block::default()
                .title(" File Diffs ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::BORDER_SUBTLE))
                .style(Style::default().bg(colors::BG_PANEL)),
        );

    f.render_widget(paragraph, area);

    // Render scrollbar if content exceeds visible area
    if content_height > visible_height {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(scroll_offset)
            .content_length(content_height)
            .viewport_content_length(visible_height);

        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}

/// Truncate text inline with ellipsis if it exceeds max_len
fn truncate_inline(text: &str, max_len: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_len {
        trimmed.to_string()
    } else {
        let mut out = trimmed
            .chars()
            .take(max_len.saturating_sub(3))
            .collect::<String>();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, ValidationStage};
    use chrono::Local;

    #[test]
    fn latest_validation_event_prefers_most_recent_entry() {
        let mut state = AppState::new();
        state.runtime_events.push(RuntimeEvent {
            timestamp: Local::now(),
            stage: "validation/protocol".to_string(),
            status: RuntimeStatus::Completed,
        });
        state.runtime_events.push(RuntimeEvent {
            timestamp: Local::now(),
            stage: "validation/runtime".to_string(),
            status: RuntimeStatus::Error,
        });

        let event = latest_validation_event(&state).expect("latest validation event");
        assert_eq!(event.stage, "validation/runtime");
        assert_eq!(event.status, RuntimeStatus::Error);
    }

    #[test]
    fn validation_stage_labels_are_human_readable() {
        assert_eq!(display_validation_stage_name("protocol"), "Protocol");
        assert_eq!(display_validation_stage_name("validation"), "Runtime");
        assert_eq!(display_validation_stage_name("syntax"), "Syntax");
        assert_eq!(validation_stage_description("validation"), "runtime checks");

        let stages = state_with_stage_names();
        assert_eq!(stages[0].name, "protocol");
        assert_eq!(stages[1].name, "validation");
    }

    fn state_with_stage_names() -> Vec<ValidationStage> {
        AppState::new().validation_stages
    }
}
