use crate::app::{App, UiAction};
use crate::state::ExecutionMode;
use crate::state::InputMode;
use crate::ui::colors;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Draw the bottom composer using the web app's textarea + metadata footer.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    let divider = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(colors::BORDER_SUBTLE))
        .style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(divider, sections[0]);

    let editor_area = sections[1].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    app.register_ui_target("composer:input", editor_area, UiAction::FocusInput);
    let input_focused = app.is_ui_focused("composer:input");
    let composer_enabled = app.composer_is_editable();
    let input_buffer = app.state.input_buffer.clone();
    let input_mode = app.state.input_mode;
    let cursor_position = app.state.cursor_position;
    // Clean dark theme - subtle border, no orange accent
    let editor_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if input_focused && composer_enabled {
            colors::BORDER_LIGHT // Subtle light border when focused
        } else {
            colors::BORDER_SUBTLE // Very subtle when not focused
        }))
        .style(Style::default().bg(colors::BG_PANEL));
    let editor_inner = editor_area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    let input_line = if input_buffer.is_empty() {
        Line::from(vec![Span::styled(
            app.composer_placeholder(),
            Style::default().fg(colors::TEXT_DIM),
        )])
    } else {
        Line::from(vec![Span::styled(
            input_buffer.clone(),
            Style::default().fg(colors::TEXT_SOFT),
        )])
    };

    let input_paragraph = Paragraph::new(input_line)
        .block(editor_block)
        .style(Style::default().bg(colors::BG_PANEL));
    f.render_widget(input_paragraph, editor_area);

    let footer = sections[2].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let footer_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22),
            Constraint::Length(28),
            Constraint::Min(0),
            Constraint::Length(40),
        ])
        .split(footer);

    let model_label = app
        .state
        .model
        .active
        .as_ref()
        .cloned()
        .or_else(|| app.state.model.configured.clone())
        .unwrap_or_else(|| "No model".to_string());
    let model_color = if app.state.model.active.is_some() {
        colors::SUCCESS
    } else if app.state.model.configured.is_some() {
        colors::ACCENT
    } else {
        colors::TEXT_DIM
    };
    let model_dot = model_status_dot(&app.state);

    let model_line = Line::from(vec![
        Span::styled("◇ ", Style::default().fg(colors::TEXT_SUBTLE)),
        Span::styled(model_dot, Style::default().fg(model_color)),
        Span::styled(" ", Style::default()),
        Span::styled(
            truncate_label(&model_label, 12),
            Style::default().fg(colors::TEXT_SOFT),
        ),
    ]);
    let model_view = Paragraph::new(model_line).style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(model_view, footer_chunks[0]);

    draw_mode_toggle(f, app, footer_chunks[1]);

    let mode_status = Paragraph::new(Line::from(vec![
        Span::styled("□ ", Style::default().fg(colors::TEXT_SUBTLE)),
        Span::styled(
            app.composer_mode_label(),
            Style::default().fg(colors::TEXT_MUTED),
        ),
    ]))
    .style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(mode_status, footer_chunks[2]);

    let hint_line = Line::from(vec![Span::styled(
        app.composer_hint_text(),
        Style::default().fg(if app.chat_blocked {
            colors::ERROR
        } else {
            colors::TEXT_DIM
        }),
    )]);
    let footer_right = Paragraph::new(hint_line)
        .alignment(Alignment::Right)
        .style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(footer_right, footer_chunks[3]);

    if input_mode == InputMode::Editing && composer_enabled {
        let cursor_byte = crate::text::clamp_to_char_boundary(&input_buffer, cursor_position);
        let cursor_col = input_buffer[..cursor_byte].chars().count() as u16;
        let cursor_x = editor_inner.x + cursor_col.min(editor_inner.width.saturating_sub(1));
        let cursor_y = editor_inner.y;
        f.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

fn draw_mode_toggle(f: &mut Frame, app: &mut App, area: Rect) {
    // In Normal mode, hide the technical CHAT/EDIT/TASK toggle entirely
    // and show a simple, friendly hint about what the user can do
    if !app.is_operator_mode() {
        // Work-first hints - Task is default, Chat is secondary
        let hint = match app.state.execution.mode {
            ExecutionMode::Task => "Describe work",
            ExecutionMode::Edit => "Describe changes",
            ExecutionMode::Chat => "Ask questions",
        };
        let simple_hint = Paragraph::new(Line::from(vec![Span::styled(
            format!("◦ {}", hint),
            Style::default().fg(colors::TEXT_DIM),
        )]))
        .style(Style::default().bg(colors::BG_PAGE));
        f.render_widget(simple_hint, area);
        return;
    }

    // Operator mode: show full technical mode toggle
    if !app.can_switch_execution_mode() {
        let fallback = Paragraph::new(Line::from(vec![Span::styled(
            app.composer_mode_label(),
            Style::default().fg(colors::TEXT_MUTED),
        )]))
        .style(Style::default().bg(colors::BG_PAGE));
        f.render_widget(fallback, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
        ])
        .split(area);

    let label = Paragraph::new(Line::from(vec![Span::styled(
        "MODE",
        Style::default().fg(colors::TEXT_DIM),
    )]))
    .style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(label, chunks[0]);

    draw_mode_button(f, app, chunks[1], ExecutionMode::Chat);
    draw_mode_button(f, app, chunks[2], ExecutionMode::Edit);
    draw_mode_button(f, app, chunks[3], ExecutionMode::Task);
}

fn draw_mode_button(f: &mut Frame, app: &mut App, area: Rect, mode: ExecutionMode) {
    let active = app.state.execution.mode == mode;
    let id = format!("composer:mode:{}", mode.as_str().to_lowercase());
    app.register_ui_target(id, area, UiAction::SetExecutionMode(mode));

    let text = format!("[{}]", mode.as_str());
    let style = if active {
        Style::default()
            .fg(colors::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(colors::TEXT_MUTED)
    };

    let button = Paragraph::new(Line::from(vec![Span::styled(text, style)]))
        .style(Style::default().bg(colors::BG_PAGE));
    f.render_widget(button, area);
}

fn model_status_dot(state: &crate::state::AppState) -> &'static str {
    if state.model.active.is_some() {
        "●"
    } else if state.model.configured.is_some() {
        "○"
    } else {
        "◌"
    }
}

pub fn truncate_label(text: &str, max_len: usize) -> String {
    crate::text::truncate_chars(text, max_len)
}
