use crate::app::{
    App, ComposerMode, PanelStatusLevel, ProjectEntry, SearchPreview, SidebarPanel, UiAction,
};
use crate::ollama::{DEFAULT_CODER_14B_MODEL, FALLBACK_PLANNER_MODEL, normalize_requested_model};
use crate::state::{InspectorTab, Message, MessageRole, RunCard, RuntimeStatus};
use crate::ui::colors;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub fn draw_header(f: &mut Frame, app: &mut App, area: Rect) {
    // GUARD: Don't render if area is too small
    if area.height == 0 || area.width == 0 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(22)])
        .split(area);

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(colors::BORDER_SUBTLE))
        .style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(block, area);

    let left = sections[0].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let right = sections[1].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });

    let state = &app.state;
    let runtime_color = match state.execution.state {
        crate::state::ExecutionState::Idle => colors::TEXT_MUTED,
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

    let panel_label = if app.show_inspector {
        state.active_inspector_tab.as_str().to_string()
    } else {
        state.repo.name.clone()
    };

    // Build status line with chain and context info
    let chain_label = app.chain_status_label();
    let context_label = app.context_status_label();

    let title_lines = vec![
        Line::from(vec![Span::styled(
            app.panel_title(),
            Style::default()
                .fg(colors::TEXT_SOFT)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("session ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.short_session_id(),
                Style::default().fg(colors::TEXT_MUTED),
            ),
            Span::styled("  |  ", Style::default().fg(colors::BORDER_SUBTLE)),
            Span::styled("project ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                truncate_inline(&state.repo.name, 14),
                Style::default().fg(colors::TEXT_MUTED),
            ),
            Span::styled("  |  ", Style::default().fg(colors::BORDER_SUBTLE)),
            Span::styled("mode ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.execution_mode_label(),
                Style::default().fg(colors::ACCENT),
            ),
            Span::styled("  |  ", Style::default().fg(colors::BORDER_SUBTLE)),
            Span::styled("state ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.execution_state_label(),
                Style::default()
                    .fg(runtime_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("chain ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                truncate_inline(&chain_label, 32),
                Style::default().fg(colors::TEXT_MUTED),
            ),
            Span::styled("  |  ", Style::default().fg(colors::BORDER_SUBTLE)),
            Span::styled("context ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(context_label, Style::default().fg(colors::TEXT_MUTED)),
            Span::styled("  |  ", Style::default().fg(colors::BORDER_SUBTLE)),
            Span::styled("objective ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                truncate_inline(&app.execution_objective_label(), 24),
                Style::default().fg(colors::TEXT_MUTED),
            ),
        ]),
    ];

    let title_paragraph =
        Paragraph::new(Text::from(title_lines)).style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(title_paragraph, left);

    let toggle_id = "header:toggle_inspector";
    let toggle_focused = app.is_ui_focused(toggle_id);
    app.register_ui_target(toggle_id, right, UiAction::ToggleInspector);
    let panel_lines = vec![
        Line::from(vec![Span::styled(
            panel_label,
            Style::default()
                .fg(if toggle_focused {
                    colors::ACCENT
                } else {
                    colors::TEXT_SOFT
                })
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(
                "[Tab]",
                Style::default()
                    .fg(if toggle_focused {
                        colors::TEXT_SOFT
                    } else {
                        colors::ACCENT
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if app.show_inspector {
                    " hide inspector"
                } else {
                    " show inspector"
                },
                Style::default().fg(if toggle_focused {
                    colors::TEXT_MUTED
                } else {
                    colors::TEXT_SUBTLE
                }),
            ),
        ]),
    ];

    let panel_paragraph = Paragraph::new(Text::from(panel_lines))
        .alignment(Alignment::Right)
        .style(Style::default().bg(if toggle_focused {
            colors::BG_PANEL
        } else {
            colors::BG_PAGE
        }));
    f.render_widget(panel_paragraph, right);
}

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    match app.active_panel {
        SidebarPanel::Chat => draw_chat(f, app, area),
        SidebarPanel::Projects => draw_projects_panel(f, app, area),
        SidebarPanel::Search => draw_search_panel(f, app, area),
        SidebarPanel::Plugins => draw_plugins_panel(f, app, area),
        SidebarPanel::Automations => draw_automations_panel(f, app, area),
    }
}

fn draw_chat(f: &mut Frame, app: &App, area: Rect) {
    let state = &app.state;

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(0)])
        .split(area);
    draw_objective_anchor(f, app, sections[0]);

    let transcript_area = sections[1];
    let content_area = transcript_area.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });

    // HARD BOUNDS: Maximum lines we can render without overflow
    let max_lines = content_area.height as usize;
    let mut text_lines: Vec<Line> = Vec::with_capacity(max_lines);

    // Update viewport for scroll state
    let mut scroll_state = state.chat_scroll.clone();
    scroll_state.update_viewport(transcript_area);

    // Calculate message line counts for virtualization FIRST
    let mut message_line_ranges: Vec<(usize, usize, &Message)> = Vec::new();
    let mut current_line = 0;

    for message in &state.messages {
        let msg_lines = estimate_message_lines(message, content_area.width);
        let start = current_line;
        let end = current_line + msg_lines;
        message_line_ranges.push((start, end, message));
        current_line = end;
    }

    // Update total lines in scroll state BEFORE calculating visible range
    scroll_state.update_total_lines(current_line);

    // NOW calculate visible range with correct total_content_lines
    let (visible_start, visible_end) = scroll_state.visible_range();
    let offset_from_bottom = scroll_state.offset_from_bottom;

    // Show scroll indicator when not at bottom (bounds checked)
    if offset_from_bottom > 0 && text_lines.len() < max_lines {
        text_lines.push(Line::from(vec![Span::styled(
            format!(
                "↑ {} earlier messages (scroll down for latest)",
                offset_from_bottom
            ),
            Style::default().fg(colors::TEXT_DIM),
        )]));
        if text_lines.len() < max_lines {
            text_lines.push(Line::from(""));
        }
    }

    // Render only messages in visible range (with hard bounds checking)
    for (start, _end, message) in message_line_ranges {
        // Skip if message is completely outside visible range
        if _end < visible_start || start > visible_end {
            continue;
        }

        // HARD BOUNDS GUARD: Stop if we're at capacity
        if text_lines.len() >= max_lines {
            break;
        }

        // Check if this is a run card message
        if let Some(ref run_card) = message.run_card {
            if !message.content.trim().is_empty() {
                append_message_block(&mut text_lines, message, content_area.width, max_lines);
            }
            append_run_card(&mut text_lines, run_card, content_area.width, max_lines);
            continue;
        }

        match message.role {
            MessageRole::User | MessageRole::Assistant => {
                append_message_block(&mut text_lines, message, content_area.width, max_lines);
            }
            MessageRole::System => {
                append_system_notice(&mut text_lines, message, content_area.width, max_lines);
            }
        }
    }

    // Show "new messages" indicator if scrolled up and new messages arrived (bounds checked)
    if scroll_state.has_pending_messages() && text_lines.len() < max_lines {
        text_lines.push(Line::from(vec![Span::styled(
            format!(
                "━━━ {} new messages ↓ ━━━",
                scroll_state.pending_message_count
            ),
            Style::default().fg(Color::Yellow),
        )]));
    }

    // CRITICAL FIX: Clamp scroll offset to prevent buffer overflow
    // The scroll offset must not exceed content height minus viewport height
    let content_height = text_lines.len();
    let viewport_height = transcript_area.height as usize;
    let _max_scroll = content_height.saturating_sub(viewport_height);
    let scroll_offset = 0u16; // Disable scrolling - we handle virtualization ourselves

    let paragraph = Paragraph::new(Text::from(text_lines))
        .block(Block::default().style(Style::default().bg(colors::BG_PAGE)))
        .style(Style::default().bg(colors::BG_PAGE))
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));

    f.render_widget(paragraph, transcript_area);
}

fn draw_objective_anchor(f: &mut Frame, app: &App, area: Rect) {
    // GUARD: Don't render if area is too small to be useful
    if area.height == 0 || area.width == 0 {
        return;
    }

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(colors::BORDER_SUBTLE))
        .style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(block, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });

    // GUARD: Ensure we don't try to render more lines than fit in inner area
    let max_lines = inner.height as usize;

    let status =
        if app.state.execution.step_index.is_some() && app.state.execution.step_total.is_some() {
            format!(
                "{} ({}/{})",
                app.execution_state_label(),
                app.state.execution.step_index.unwrap_or(0),
                app.state.execution.step_total.unwrap_or(0)
            )
        } else {
            app.execution_state_label().to_string()
        };

    // V1.5 CLEANUP: Determine if we should show recovery details (block_fix)
    // Only show FIX line when outcome is Blocked or Failed
    let show_recovery_details = if let Some(ref chain_id) = app.persistence.active_chain_id {
        if let Some(chain) = app.persistence.get_chain(chain_id) {
            if let Some(outcome) = chain.get_outcome() {
                matches!(
                    outcome,
                    crate::persistence::ExecutionOutcome::Blocked
                        | crate::persistence::ExecutionOutcome::Failed
                )
            } else {
                // No outcome yet - show block_fix if execution is blocked
                app.chat_blocked
                    || matches!(
                        app.state.execution.state,
                        crate::state::ExecutionState::Blocked
                    )
            }
        } else {
            false
        }
    } else {
        // Non-chain execution - show block_fix if execution is blocked
        app.chat_blocked
            || matches!(
                app.state.execution.state,
                crate::state::ExecutionState::Blocked
            )
    };

    let mut lines = vec![
        Line::from(vec![Span::styled(
            "ACTIVE OBJECTIVE",
            Style::default()
                .fg(colors::TEXT_DIM)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            app.execution_objective_label(),
            Style::default().fg(colors::TEXT_SOFT),
        )]),
        Line::from(vec![
            Span::styled("STATUS ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(status, Style::default().fg(colors::ACCENT)),
            Span::styled("  MODE ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.execution_mode_label(),
                Style::default().fg(colors::TEXT_MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("STEP ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.execution_step_label(),
                Style::default().fg(colors::TEXT_MUTED),
            ),
        ]),
    ];

    // V1.5 CLEANUP: Only show FIX line when recovery details are relevant
    if show_recovery_details {
        lines.push(Line::from(vec![
            Span::styled("FIX ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.state.execution.block_fix.as_deref().unwrap_or("none"),
                Style::default().fg(colors::TEXT_MUTED),
            ),
        ]));
    }

    // CRITICAL: Truncate lines to fit within available height to prevent buffer overflow
    if lines.len() > max_lines {
        lines.truncate(max_lines);
    }

    let paragraph = Paragraph::new(Text::from(lines)).style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(paragraph, inner);
}

fn draw_projects_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    let has_status = app.panel_status.is_some();
    let constraints = if has_status {
        vec![
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(0),
        ]
    } else {
        vec![
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(0),
        ]
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut index = 0;
    draw_panel_intro(
        f,
        sections[index],
        "Open or create a local project workspace.",
        Some(format!(
            "Choose a folder, paste a path, or drop a folder path into the composer. Default root: {}",
            current_project_root(app)
        )),
    );
    index += 1;

    let action_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(sections[index]);
    draw_panel_button(
        f,
        app,
        action_row[0],
        PanelButton::new(
            "projects:choose",
            "Choose folder",
            "Open the native folder picker",
            UiAction::PickProjectFolder,
            false,
        ),
    );
    draw_panel_button(
        f,
        app,
        action_row[1],
        PanelButton::new(
            "projects:create",
            "Create folder",
            "Make and attach a workspace",
            UiAction::StartProjectCreate,
            app.composer_mode == ComposerMode::ProjectCreate,
        ),
    );
    draw_panel_button(
        f,
        app,
        action_row[2],
        PanelButton::new(
            "projects:connect",
            "Paste path",
            "Attach from typed or dropped path",
            UiAction::StartProjectConnect,
            app.composer_mode == ComposerMode::ProjectConnect,
        ),
    );
    index += 1;

    if let Some(status) = &app.panel_status {
        draw_status_banner(f, sections[index], status.level, &status.message);
        index += 1;
    }

    draw_attached_project(f, app, sections[index]);
    index += 1;

    draw_recent_projects(f, app, sections[index]);
}

fn draw_search_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    let has_status = app.panel_status.is_some();
    let constraints = if has_status {
        vec![
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(0),
        ]
    } else {
        vec![Constraint::Length(4), Constraint::Min(0)]
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let query_text = app
        .search_query
        .as_ref()
        .map(|query| format!("Last query: {}", query))
        .unwrap_or_else(|| "Type a query below and press Enter.".to_string());
    draw_panel_intro(
        f,
        sections[0],
        "Search code, configs, and text across connected projects.",
        Some(format!(
            "{}  |  Scope: {}  |  Click a result to preview it.",
            query_text,
            app.search_scope_label()
        )),
    );

    let mut content_index = 1;
    if let Some(status) = &app.panel_status {
        draw_status_banner(f, sections[1], status.level, &status.message);
        content_index = 2;
    }

    if app.search_preview.is_some() {
        let content = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(7)])
            .split(sections[content_index]);
        draw_search_results(f, app, content[0]);
        draw_search_preview(f, app.search_preview.as_ref(), content[1]);
    } else {
        draw_search_results(f, app, sections[content_index]);
    }
}

fn draw_plugins_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let has_status = app.panel_status.is_some();
    let constraints = if has_status {
        vec![
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ]
    } else {
        vec![
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ]
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    draw_panel_intro(
        f,
        sections[0],
        "Plugins manages the runtime adapters and model surfaces Rasputin depends on.",
        Some("Refresh Ollama, switch models, and jump straight into runtime logs.".to_string()),
    );

    let row_one = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sections[1]);
    draw_panel_button(
        f,
        app,
        row_one[0],
        PanelButton::new(
            "plugins:refresh",
            "Refresh runtime",
            "Recheck Ollama and the active model",
            UiAction::RefreshRuntime,
            false,
        ),
    );
    draw_panel_button(
        f,
        app,
        row_one[1],
        PanelButton::new(
            "plugins:logs",
            "Open logs",
            "Jump to the inspector log stream",
            UiAction::SelectInspectorTab(InspectorTab::Logs),
            app.show_inspector && app.state.active_inspector_tab == InspectorTab::Logs,
        ),
    );

    let row_two_index = 2;
    let row_two = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sections[row_two_index]);
    draw_panel_button(
        f,
        app,
        row_two[0],
        PanelButton::new(
            "plugins:model:14b",
            "Prefer 14B",
            "Use the coder-first 14B model family",
            UiAction::UseModel(DEFAULT_CODER_14B_MODEL.to_string()),
            model_family_active(app, DEFAULT_CODER_14B_MODEL),
        ),
    );
    draw_panel_button(
        f,
        app,
        row_two[1],
        PanelButton::new(
            "plugins:model:fallback",
            "Fallback 3.5",
            "Use the lighter qwen3.5 planner fallback",
            UiAction::UseModel(FALLBACK_PLANNER_MODEL.to_string()),
            model_family_active(app, FALLBACK_PLANNER_MODEL),
        ),
    );

    if let Some(status) = &app.panel_status {
        draw_status_banner(f, sections[3], status.level, &status.message);
    }

    let available_models = if app.state.model.available.is_empty() {
        "No models cached yet".to_string()
    } else {
        app.state
            .model
            .available
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    let lines = vec![
        plugin_line(
            "Ollama",
            if app.state.ollama_connected {
                "Connected"
            } else {
                "Disconnected"
            },
            if app.state.ollama_connected {
                colors::SUCCESS
            } else {
                colors::ERROR
            },
        ),
        plugin_line(
            "Security",
            app.ollama.security_posture(),
            colors::TEXT_MUTED,
        ),
        plugin_line(
            "Model",
            app.state
                .model
                .active
                .as_deref()
                .or(app.state.model.configured.as_deref())
                .unwrap_or("Not configured"),
            if app.state.model.active.is_some() {
                colors::SUCCESS
            } else if app.state.model.configured.is_some() {
                colors::ACCENT
            } else {
                colors::TEXT_DIM
            },
        ),
        plugin_line(
            "Execution runtime",
            if app.active_execution_runtime.is_some() {
                "Active"
            } else {
                "Idle"
            },
            if app.active_execution_runtime.is_some() {
                colors::ACCENT
            } else {
                colors::TEXT_MUTED
            },
        ),
        plugin_line(
            "Validation pipeline",
            &format!("{} stages", app.state.validation_stages.len()),
            colors::TEXT_MUTED,
        ),
        plugin_line(
            "Connected projects",
            &app.project_entries().len().to_string(),
            colors::TEXT_MUTED,
        ),
        plugin_line("Installed models", &available_models, colors::TEXT_MUTED),
        plugin_line("Mouse navigation", "Enabled", colors::SUCCESS),
    ];

    let paragraph = Paragraph::new(lines)
        .block(panel_block(" Plugin Status "))
        .style(Style::default().bg(colors::BG_PANEL));
    let status_index = if has_status { 4 } else { 3 };
    f.render_widget(paragraph, sections[status_index]);
}

fn draw_automations_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let has_status = app.panel_status.is_some();
    let constraints = if has_status {
        vec![
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ]
    } else {
        vec![
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ]
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    draw_panel_intro(
        f,
        sections[0],
        "Automations is the maintenance surface for runtime checks and validation.",
        Some(
            "Run the validator, reset stale state, or clear noisy logs from one place.".to_string(),
        ),
    );

    let inspector_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(sections[1]);
    draw_panel_button(
        f,
        app,
        inspector_row[0],
        PanelButton::new(
            "automation:runtime",
            "Runtime",
            "Open runtime tab",
            UiAction::SelectInspectorTab(InspectorTab::Runtime),
            app.show_inspector && app.state.active_inspector_tab == InspectorTab::Runtime,
        ),
    );
    draw_panel_button(
        f,
        app,
        inspector_row[1],
        PanelButton::new(
            "automation:validation",
            "Validation",
            "Open validation tab",
            UiAction::SelectInspectorTab(InspectorTab::Validation),
            app.show_inspector && app.state.active_inspector_tab == InspectorTab::Validation,
        ),
    );
    draw_panel_button(
        f,
        app,
        inspector_row[2],
        PanelButton::new(
            "automation:logs",
            "Logs",
            "Open log tab",
            UiAction::SelectInspectorTab(InspectorTab::Logs),
            app.show_inspector && app.state.active_inspector_tab == InspectorTab::Logs,
        ),
    );

    let actions_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sections[2]);
    draw_panel_button(
        f,
        app,
        actions_row[0],
        PanelButton::new(
            "automation:run_validation",
            "Run validation",
            "Execute the local validation pipeline",
            UiAction::RunValidation,
            false,
        ),
    );
    draw_panel_button(
        f,
        app,
        actions_row[1],
        PanelButton::new(
            "automation:refresh_runtime",
            "Refresh runtime",
            "Recheck Ollama and runtime health",
            UiAction::RefreshRuntime,
            false,
        ),
    );

    let status_row_index = if has_status { 4 } else { 3 };
    let cleanup_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sections[status_row_index]);
    draw_panel_button(
        f,
        app,
        cleanup_row[0],
        PanelButton::new(
            "automation:clear_logs",
            "Clear logs",
            "Reset the inspector log stream",
            UiAction::ClearLogs,
            false,
        ),
    );
    draw_panel_button(
        f,
        app,
        cleanup_row[1],
        PanelButton::new(
            "automation:reset_validation",
            "Reset validation",
            "Clear stale stage state and start fresh",
            UiAction::ResetValidation,
            false,
        ),
    );

    if let Some(status) = &app.panel_status {
        draw_status_banner(f, sections[3], status.level, &status.message);
    }

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Active project: ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(&app.state.repo.name, Style::default().fg(colors::TEXT_SOFT)),
        ]),
        Line::from(vec![
            Span::styled("Logs captured: ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.state.logs.len().to_string(),
                Style::default().fg(colors::TEXT_SOFT),
            ),
        ]),
        Line::from(vec![
            Span::styled("Validation stages: ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.state.validation_stages.len().to_string(),
                Style::default().fg(colors::TEXT_SOFT),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Recent validation state:",
            Style::default()
                .fg(colors::TEXT_MUTED)
                .add_modifier(Modifier::BOLD),
        )]),
    ];

    for stage in app.state.validation_stages.iter().take(4) {
        let color = match stage.status {
            RuntimeStatus::Completed => colors::SUCCESS,
            RuntimeStatus::Running => colors::ACCENT,
            RuntimeStatus::Error => colors::ERROR,
            RuntimeStatus::Idle => colors::TEXT_DIM,
        };

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(stage.name.clone(), Style::default().fg(colors::TEXT_SOFT)),
            Span::styled("  ", Style::default()),
            Span::styled(stage.status.as_str(), Style::default().fg(color)),
        ]));
    }

    let paragraph = Paragraph::new(lines)
        .block(panel_block(" Maintenance Summary "))
        .style(Style::default().bg(colors::BG_PANEL));
    let summary_index = if has_status { 5 } else { 4 };
    f.render_widget(paragraph, sections[summary_index]);
}

fn draw_search_results(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel_block(" Search Results ");
    f.render_widget(block, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    if app.search_results.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(vec![Span::styled(
                "No search results yet.",
                Style::default().fg(colors::TEXT_DIM),
            )]),
            Line::from(vec![Span::styled(
                "Searches run across your connected project folders.",
                Style::default().fg(colors::TEXT_SUBTLE),
            )]),
        ])
        .style(Style::default().bg(colors::BG_PANEL))
        .wrap(Wrap { trim: false });
        f.render_widget(empty, inner);
        return;
    }

    let visible_results = app
        .search_results
        .iter()
        .take(inner.height as usize)
        .cloned()
        .collect::<Vec<_>>();

    for (index, result) in visible_results.iter().enumerate() {
        let row = inner.y + index as u16;
        if row >= inner.y.saturating_add(inner.height) {
            break;
        }
        let rect = Rect::new(inner.x, row, inner.width, 1);
        let id = format!("search:result:{}", index);
        let selected = app.selected_search_result == Some(index);
        let focused = app.is_ui_focused(&id);
        app.register_ui_target(id, rect, UiAction::OpenSearchResult(index));

        let line = Line::from(vec![
            Span::styled(
                if selected { "> " } else { "  " },
                Style::default().fg(if selected {
                    colors::ACCENT
                } else {
                    colors::TEXT_SUBTLE
                }),
            ),
            Span::styled(
                truncate_inline(
                    &format!(
                        "{}:{}  {}",
                        result.display_path, result.line_number, result.preview
                    ),
                    rect.width.saturating_sub(2) as usize,
                ),
                Style::default()
                    .fg(if selected || focused {
                        colors::TEXT_SOFT
                    } else {
                        colors::TEXT_MUTED
                    })
                    .add_modifier(if selected || focused {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]);

        let paragraph = Paragraph::new(line).style(Style::default().bg(if selected || focused {
            colors::BG_RAISED
        } else {
            colors::BG_PANEL
        }));
        f.render_widget(paragraph, rect);
    }
}

fn draw_search_preview(f: &mut Frame, preview: Option<&SearchPreview>, area: Rect) {
    let block = panel_block(" Preview ");
    f.render_widget(block, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    let Some(preview) = preview else {
        let empty = Paragraph::new(Line::from(vec![Span::styled(
            "Select a result to preview it here.",
            Style::default().fg(colors::TEXT_DIM),
        )]))
        .style(Style::default().bg(colors::BG_PANEL));
        f.render_widget(empty, inner);
        return;
    };

    let mut lines = vec![
        Line::from(vec![Span::styled(
            truncate_inline(&preview.title, inner.width as usize),
            Style::default()
                .fg(colors::TEXT_SOFT)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            truncate_inline(&preview.file_path, inner.width as usize),
            Style::default().fg(colors::TEXT_SUBTLE),
        )]),
        Line::from(""),
    ];

    for line in &preview.lines {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>4} ", line.number),
                Style::default().fg(if line.highlighted {
                    colors::ACCENT
                } else {
                    colors::TEXT_DIM
                }),
            ),
            Span::styled(
                truncate_inline(&line.content, inner.width.saturating_sub(5) as usize),
                Style::default()
                    .fg(if line.highlighted {
                        colors::TEXT_SOFT
                    } else {
                        colors::TEXT_MUTED
                    })
                    .add_modifier(if line.highlighted {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]));
    }

    let paragraph = Paragraph::new(lines)
        .style(Style::default().bg(colors::BG_PANEL))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

fn draw_panel_intro(f: &mut Frame, area: Rect, title: &str, subtitle: Option<String>) {
    let mut lines = vec![Line::from(vec![Span::styled(
        title,
        Style::default()
            .fg(colors::TEXT_SOFT)
            .add_modifier(Modifier::BOLD),
    )])];

    if let Some(subtitle) = subtitle {
        lines.push(Line::from(vec![Span::styled(
            subtitle,
            Style::default().fg(colors::TEXT_SUBTLE),
        )]));
    }

    let paragraph = Paragraph::new(lines)
        .block(panel_block(" Overview "))
        .style(Style::default().bg(colors::BG_PANEL));
    f.render_widget(paragraph, area);
}

fn draw_status_banner(f: &mut Frame, area: Rect, level: PanelStatusLevel, message: &str) {
    let (label, fg, bg) = match level {
        PanelStatusLevel::Info => ("Info", colors::TEXT_MUTED, colors::BG_RAISED),
        PanelStatusLevel::Success => ("Ready", colors::SUCCESS, colors::BG_RAISED),
        PanelStatusLevel::Error => ("Error", colors::ERROR, colors::BG_RAISED),
    };

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", label),
            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", truncate_inline(message, 80)),
            Style::default().fg(colors::TEXT_SOFT),
        ),
    ]))
    .block(panel_block(" Status "))
    .style(Style::default().bg(colors::BG_PANEL));
    f.render_widget(paragraph, area);
}

fn draw_panel_button(f: &mut Frame, app: &mut App, area: Rect, button: PanelButton<'_>) {
    app.register_ui_target(button.id, area, button.action);
    let focused = app.is_ui_focused(button.id);
    let border_color = if focused || button.active {
        colors::ACCENT
    } else {
        colors::BORDER_LIGHT
    };
    let background = if focused || button.active {
        colors::BG_RAISED
    } else {
        colors::BG_PANEL
    };

    let paragraph = Paragraph::new(vec![
        Line::from(vec![Span::styled(
            button.label.to_string(),
            Style::default()
                .fg(colors::TEXT_SOFT)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            truncate_inline(button.detail, area.width.saturating_sub(4) as usize),
            Style::default().fg(colors::TEXT_SUBTLE),
        )]),
    ])
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(background)),
    )
    .style(Style::default().bg(background));
    f.render_widget(paragraph, area);
}

struct PanelButton<'a> {
    id: &'a str,
    label: &'a str,
    detail: &'a str,
    action: UiAction,
    active: bool,
}

impl<'a> PanelButton<'a> {
    fn new(id: &'a str, label: &'a str, detail: &'a str, action: UiAction, active: bool) -> Self {
        Self {
            id,
            label,
            detail,
            action,
            active,
        }
    }
}

fn model_family_active(app: &App, expected: &str) -> bool {
    app.state
        .model
        .configured
        .as_deref()
        .or(app.state.model.active.as_deref())
        .is_some_and(|model| {
            let current = normalize_requested_model(model);
            let expected = normalize_requested_model(expected);
            if expected == DEFAULT_CODER_14B_MODEL {
                current.starts_with(DEFAULT_CODER_14B_MODEL)
            } else {
                current == expected
            }
        })
}

fn draw_attached_project(f: &mut Frame, app: &App, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled(
                truncate_inline(&app.state.repo.name, 20),
                Style::default()
                    .fg(colors::TEXT_SOFT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                if app.state.repo.git_detected {
                    "git"
                } else {
                    "folder"
                },
                Style::default().fg(colors::TEXT_DIM),
            ),
        ]),
        Line::from(vec![Span::styled(
            truncate_inline(&app.state.repo.display_path, 72),
            Style::default().fg(colors::TEXT_MUTED),
        )]),
        Line::from(vec![
            Span::styled("Model: ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(
                app.state
                    .model
                    .configured
                    .as_deref()
                    .unwrap_or("Not configured"),
                Style::default().fg(colors::TEXT_SOFT),
            ),
        ]),
    ];

    let paragraph = Paragraph::new(lines)
        .block(panel_block(" Attached Project "))
        .style(Style::default().bg(colors::BG_PANEL));
    f.render_widget(paragraph, area);
}

fn draw_recent_projects(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel_block(" Recent Projects ");
    f.render_widget(block, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let projects = app.project_entries();

    if projects.is_empty() {
        let empty = Paragraph::new(Line::from(vec![Span::styled(
            "No connected projects yet.",
            Style::default().fg(colors::TEXT_DIM),
        )]))
        .style(Style::default().bg(colors::BG_PANEL));
        f.render_widget(empty, inner);
        return;
    }

    let mut row = inner.y;
    for (index, project) in projects.iter().take(5).enumerate() {
        if row >= inner.y.saturating_add(inner.height) {
            break;
        }
        let rect = Rect::new(inner.x, row, inner.width, 1);
        draw_recent_project_row(f, app, rect, index, project);
        row += 1;
    }
}

fn draw_recent_project_row(
    f: &mut Frame,
    app: &mut App,
    area: Rect,
    index: usize,
    project: &ProjectEntry,
) {
    let id = format!("projects:list:{}", index);
    app.register_ui_target(
        id.clone(),
        area,
        UiAction::OpenProject(project.path.clone()),
    );
    let focused = app.is_ui_focused(&id);

    let line = Line::from(vec![
        Span::styled(
            if project.active { "> " } else { "  " },
            Style::default().fg(if project.active {
                colors::ACCENT
            } else {
                colors::TEXT_SUBTLE
            }),
        ),
        Span::styled(
            truncate_inline(
                &format!("{}  {}", project.name, project.display_path),
                area.width.saturating_sub(2) as usize,
            ),
            Style::default()
                .fg(if focused || project.active {
                    colors::TEXT_SOFT
                } else {
                    colors::TEXT_MUTED
                })
                .add_modifier(if focused || project.active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(if focused {
        colors::BG_RAISED
    } else {
        colors::BG_PANEL
    }));
    f.render_widget(paragraph, area);
}

fn append_message_block(lines: &mut Vec<Line>, message: &Message, width: u16, max_lines: usize) {
    // New minimal format:
    // ▸ You
    // plain message text
    //
    // ◆ Rasputin
    // plain response text

    let (marker, marker_fg, author, author_fg, content_fg) = match message.role {
        MessageRole::User => (
            "▸",
            colors::ACCENT,
            "You",
            colors::TEXT_SOFT,
            colors::TEXT_SOFT,
        ),
        MessageRole::Assistant => (
            "◆",
            colors::TEXT_MUTED,
            "Rasputin",
            colors::TEXT_SOFT,
            colors::TEXT_MUTED,
        ),
        MessageRole::System => unreachable!(),
    };

    // Title line with marker and author (bounds checked)
    if lines.len() >= max_lines {
        return;
    }
    lines.push(Line::from(vec![
        Span::styled(
            marker,
            Style::default().fg(marker_fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(
            author,
            Style::default().fg(author_fg).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Content with syntax highlighting for code blocks
    let content_width = width.saturating_sub(2) as usize;
    let mut in_code_block = false;
    let mut code_block_content = String::new();
    let mut code_block_language = None;

    for line in message.content.lines() {
        // BOUNDS CHECK: Stop if at capacity
        if lines.len() >= max_lines {
            return;
        }

        if line.starts_with("```") {
            if in_code_block {
                // End of code block - highlight and add it
                if !code_block_content.is_empty() {
                    let highlighted = crate::syntax::highlight_code(
                        &code_block_content,
                        code_block_language.as_deref(),
                    );
                    for hl_line in highlighted {
                        if lines.len() >= max_lines {
                            return;
                        }
                        let mut prefixed = vec![Span::styled("  ", Style::default())];
                        prefixed.extend(hl_line.spans);
                        lines.push(Line::from(prefixed));
                    }
                }
                in_code_block = false;
                code_block_content.clear();
                code_block_language = None;
            } else {
                // Start of code block
                in_code_block = true;
                let lang = line.trim_start_matches('`').trim();
                code_block_language = if lang.is_empty() {
                    None
                } else {
                    Some(lang.to_string())
                };
            }
        } else if in_code_block {
            code_block_content.push_str(line);
            code_block_content.push('\n');
        } else {
            // Regular text
            if line.is_empty() {
                lines.push(Line::from(""));
            } else {
                // Hard wrap long lines
                let wrapped = wrap_line(line, content_width);
                for wrapped_line in wrapped {
                    if lines.len() >= max_lines {
                        return;
                    }
                    lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(wrapped_line, Style::default().fg(content_fg)),
                    ]));
                }
            }
        }
    }

    // Handle unclosed code block
    if in_code_block && !code_block_content.is_empty() {
        let highlighted =
            crate::syntax::highlight_code(&code_block_content, code_block_language.as_deref());
        for hl_line in highlighted {
            if lines.len() >= max_lines {
                return;
            }
            let mut prefixed = vec![Span::styled("  ", Style::default())];
            prefixed.extend(hl_line.spans);
            lines.push(Line::from(prefixed));
        }
    }

    // Single blank line after message (bounds checked)
    if lines.len() < max_lines {
        lines.push(Line::from(""));
    }
}

fn append_system_notice(lines: &mut Vec<Line>, message: &Message, width: u16, max_lines: usize) {
    // Compact one-line system notice format:
    // ○ Repo attached: Rasputin-1
    // ○ Model verified: qwen3.5:latest

    // BOUNDS CHECK: Don't add if at capacity
    if lines.len() >= max_lines {
        return;
    }

    let content = message.content.trim();
    let max_width = width.saturating_sub(4) as usize;

    // Truncate if needed
    let display_content = if content.chars().count() > max_width && max_width > 3 {
        crate::text::truncate_chars(content, max_width)
    } else {
        content.to_string()
    };

    if lines.len() < max_lines {
        lines.push(Line::from(vec![
            Span::styled("[info] ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(display_content, Style::default().fg(colors::TEXT_MUTED)),
        ]));
    }
}

fn append_run_card(lines: &mut Vec<Line>, run_card: &RunCard, _width: u16, max_lines: usize) {
    let is_completed = run_card.finished_at.is_some();
    let (header_marker, header_fg) = if is_completed {
        if run_card.status == RuntimeStatus::Completed {
            ("[done]", colors::SUCCESS)
        } else {
            ("[ERR]", colors::ERROR)
        }
    } else {
        ("[run]", colors::ACCENT)
    };

    let status_text = run_card.status.as_str().to_lowercase();
    let model_info = run_card.model.as_deref().unwrap_or("unknown");

    // Header line (bounds checked)
    if lines.len() >= max_lines {
        return;
    }
    lines.push(Line::from(vec![
        Span::styled(
            header_marker,
            Style::default().fg(header_fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(
            format!("Forge: {}", run_card.task),
            Style::default()
                .fg(colors::TEXT_SOFT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Status subline (bounds checked)
    if lines.len() >= max_lines {
        return;
    }
    let subline = if is_completed {
        let duration = run_card.duration_secs();
        format!(
            "{} | {} iteration{} | {}s",
            status_text,
            run_card.iterations,
            if run_card.iterations == 1 { "" } else { "s" },
            duration
        )
    } else {
        format!("{} | {} | {}", status_text, run_card.phase, model_info)
    };

    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(subline, Style::default().fg(colors::TEXT_MUTED)),
    ]));

    if let Some(step) = run_card.current_step.as_ref() {
        if lines.len() >= max_lines {
            return;
        }
        lines.push(Line::from(vec![
            Span::styled("  step  ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(step.clone(), Style::default().fg(colors::TEXT_SOFT)),
        ]));
    }

    if let Some(tool) = run_card.active_tool.as_ref() {
        if lines.len() >= max_lines {
            return;
        }
        lines.push(Line::from(vec![
            Span::styled("  tool  ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(tool.clone(), Style::default().fg(colors::ACCENT)),
        ]));
    }

    if let Some(validation) = run_card.validation_summary.as_ref() {
        if lines.len() >= max_lines {
            return;
        }
        lines.push(Line::from(vec![
            Span::styled("  check ", Style::default().fg(colors::TEXT_DIM)),
            Span::styled(validation.clone(), Style::default().fg(colors::TEXT_MUTED)),
        ]));
    }

    // Event list (only show if there are events and we're still running, or for completed)
    if !run_card.events.is_empty() {
        if lines.len() >= max_lines {
            return;
        }
        lines.push(Line::from(""));
        for event in &run_card.events {
            if lines.len() >= max_lines {
                return;
            }
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(event.clone(), Style::default().fg(colors::TEXT_SUBTLE)),
            ]));
        }
    }

    // Result message if completed
    if let Some(ref result) = run_card.result_message {
        if lines.len() >= max_lines {
            return;
        }
        lines.push(Line::from(""));
        if lines.len() < max_lines {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(result.clone(), Style::default().fg(colors::TEXT_MUTED)),
            ]));
        }
    }

    // Spacing after run card (bounds checked)
    if lines.len() < max_lines {
        lines.push(Line::from(""));
    }
}

/// Hard wrap a line of text to fit within max_width
fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    crate::text::wrap_line_chars(line, max_width)
}

fn plugin_line(label: &str, value: &str, color: ratatui::style::Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<18}", label),
            Style::default().fg(colors::TEXT_DIM),
        ),
        Span::styled(value.to_string(), Style::default().fg(color)),
    ])
}

fn current_project_root(app: &App) -> String {
    if app.state.repo.path == "~" || app.state.repo.path.is_empty() {
        ".".to_string()
    } else {
        std::path::Path::new(&app.state.repo.path)
            .parent()
            .and_then(|path| path.to_str())
            .map(shorten_home)
            .unwrap_or_else(|| shorten_home(&app.state.repo.path))
    }
}

fn panel_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::BORDER_SUBTLE))
        .style(Style::default().bg(colors::BG_PANEL))
}

pub fn current_conversation_title(app: &App) -> String {
    app.conversation_title()
}

fn shorten_home(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        path.replacen(&home, "~", 1)
    } else {
        path.to_string()
    }
}

fn truncate_inline(text: &str, max_len: usize) -> String {
    crate::text::truncate_chars(text.trim(), max_len)
}

/// Estimate how many terminal lines a message will occupy
fn estimate_message_lines(message: &Message, width: u16) -> usize {
    // Guard against zero width to prevent division by zero
    let width = width.max(1) as usize;

    // Count lines in content
    let content_lines = message.content.lines().count();

    // Add header line (role + timestamp)
    let header_lines = 1;

    // Add spacing
    let spacing = 1;

    // Estimate wrapped lines (rough approximation)
    let wrapped_lines: usize = message
        .content
        .lines()
        .map(|line| (line.chars().count() / width) + 1)
        .sum();

    header_lines + wrapped_lines.max(content_lines) + spacing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_curly_quotes_em_dash_and_long_numbered_markdown_without_panic() {
        let line = "1. “Unicode-heavy” structured output — with a verylongtoken“inside”";
        let wrapped = wrap_line(line, 12);

        assert!(wrapped.len() > 1);
        assert!(wrapped.iter().all(|line| line.is_char_boundary(line.len())));
    }

    #[test]
    fn truncates_inline_unicode_without_byte_boundary_panic() {
        let text = "Next goal — inspect “docs” and validate";
        let truncated = truncate_inline(text, 18);

        assert!(truncated.ends_with("..."));
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
