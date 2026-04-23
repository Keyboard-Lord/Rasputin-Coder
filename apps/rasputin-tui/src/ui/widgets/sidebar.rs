use crate::app::{App, ProjectEntry, SidebarPanel, UiAction};
use crate::persistence::PersistentConversation;
use crate::ui::colors;
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::collections::HashSet;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let shell = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(colors::BORDER_SUBTLE))
        .style(Style::default().bg(colors::BG_SIDEBAR));
    f.render_widget(shell, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let footer_y = area.y.saturating_add(area.height.saturating_sub(2));
    let content_bottom = footer_y;

    draw_brand(f, inner.x, inner.y, inner.width, content_bottom);

    let mut row = inner.y + 2;
    row = draw_button(
        f,
        app,
        ButtonSpec::new(
            "nav:new_chat",
            UiAction::NewChat,
            row,
            "+",
            "New chat",
            false,
        ),
        inner.x,
        inner.width,
        content_bottom,
    );
    if row >= content_bottom {
        draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
        return;
    }
    row = draw_button(
        f,
        app,
        ButtonSpec::new(
            "nav:projects",
            UiAction::Projects,
            row,
            ">",
            "Projects",
            app.active_panel == SidebarPanel::Projects,
        ),
        inner.x,
        inner.width,
        content_bottom,
    );
    if row >= content_bottom {
        draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
        return;
    }
    row = draw_button(
        f,
        app,
        ButtonSpec::new(
            "nav:search",
            UiAction::Search,
            row,
            "/",
            "Search",
            app.active_panel == SidebarPanel::Search,
        ),
        inner.x,
        inner.width,
        content_bottom,
    );
    if row >= content_bottom {
        draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
        return;
    }
    row = draw_button(
        f,
        app,
        ButtonSpec::new(
            "nav:plugins",
            UiAction::Plugins,
            row,
            "*",
            "Plugins",
            app.active_panel == SidebarPanel::Plugins,
        ),
        inner.x,
        inner.width,
        content_bottom,
    );
    if row >= content_bottom {
        draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
        return;
    }
    row = draw_button(
        f,
        app,
        ButtonSpec::new(
            "nav:automations",
            UiAction::Automations,
            row,
            "@",
            "Automations",
            app.active_panel == SidebarPanel::Automations,
        ),
        inner.x,
        inner.width,
        content_bottom,
    );
    if row >= content_bottom {
        draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
        return;
    }

    row += 2;
    draw_heading(
        f,
        Rect::new(inner.x, row, inner.width, 1),
        "Projects",
        content_bottom,
    );
    row += 1;
    if row >= content_bottom {
        draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
        return;
    }

    let projects = app.project_entries();
    if projects.is_empty() {
        if has_room(row, 1, content_bottom) {
            let empty = Paragraph::new(Line::from(vec![Span::styled(
                "No projects connected",
                Style::default().fg(colors::TEXT_DIM),
            )]))
            .style(Style::default().bg(colors::BG_SIDEBAR));
            f.render_widget(empty, Rect::new(inner.x, row, inner.width, 1));
            row += 2;
        } else {
            row = content_bottom;
        }
    } else {
        for project in projects.iter().take(3) {
            row = draw_project_button(
                f,
                app,
                Rect::new(inner.x, row, inner.width, 2),
                project,
                content_bottom,
            );
            if row >= content_bottom {
                draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
                return;
            }
        }
        row += 1;
    }

    draw_heading(
        f,
        Rect::new(inner.x, row, inner.width, 1),
        "Chats",
        content_bottom,
    );
    row += 1;
    if row >= content_bottom {
        draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
        return;
    }

    for conversation in sidebar_conversations(app).into_iter().take(4) {
        row = draw_conversation_button(
            f,
            app,
            inner.x,
            inner.width,
            row,
            &conversation,
            content_bottom,
        );
        if row >= content_bottom {
            draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
            return;
        }
    }

    let archived = sidebar_archived_conversations(app);
    if !archived.is_empty() {
        row += 1;
        draw_heading(
            f,
            Rect::new(inner.x, row, inner.width, 1),
            "Archived",
            content_bottom,
        );
        row += 1;
        if row >= content_bottom {
            draw_footer(f, Rect::new(inner.x, footer_y, inner.width, 1));
            return;
        }

        for conversation in archived.into_iter().take(2) {
            row = draw_conversation_button(
                f,
                app,
                inner.x,
                inner.width,
                row,
                &conversation,
                content_bottom,
            );
            if row >= content_bottom {
                break;
            }
        }
    }

    // Draw inspector and mode controls (above footer)
    row += 1;
    if row < footer_y.saturating_sub(2) {
        draw_heading(f, Rect::new(inner.x, row, inner.width, 1), "View", footer_y);
        row += 1;

        // Inspector toggle button
        row = draw_button(
            f,
            app,
            ButtonSpec::new(
                "view:inspector",
                UiAction::ToggleInspector,
                row,
                if app.show_inspector { "▼" } else { "▶" },
                if app.show_inspector {
                    "Hide inspector"
                } else {
                    "Show inspector"
                },
                app.show_inspector,
            ),
            inner.x,
            inner.width,
            footer_y,
        );

        // Experience mode toggle (always visible but styled differently)
        let _row = draw_button(
            f,
            app,
            ButtonSpec::new(
                "view:mode",
                UiAction::ToggleExperienceMode,
                row,
                if app.is_operator_mode() { "⚙" } else { "◦" },
                if app.is_operator_mode() {
                    "Operator mode"
                } else {
                    "Normal mode"
                },
                app.is_operator_mode(),
            ),
            inner.x,
            inner.width,
            footer_y,
        );
    }

    let footer = Rect::new(inner.x, footer_y, inner.width, 1);
    draw_footer(f, footer);
}

fn draw_brand(f: &mut Frame, x: u16, y: u16, width: u16, bottom: u16) {
    if !has_room(y, 1, bottom) {
        return;
    }

    let line = Line::from(vec![Span::styled(
        "Rasputin",
        Style::default()
            .fg(colors::TEXT_SOFT)
            .add_modifier(Modifier::BOLD),
    )]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(colors::BG_SIDEBAR));
    f.render_widget(paragraph, Rect::new(x, y, width, 1));
}

fn draw_button(
    f: &mut Frame,
    app: &mut App,
    spec: ButtonSpec,
    x: u16,
    width: u16,
    bottom: u16,
) -> u16 {
    if !has_room(spec.row, 1, bottom) {
        return bottom;
    }

    let rect = Rect::new(x, spec.row, width, 1);
    app.register_ui_target(spec.id.clone(), rect, spec.action.clone());
    let focused = app.is_ui_focused(&spec.id);

    let icon_style = if spec.emphasized || spec.active {
        Style::default().fg(colors::ACCENT)
    } else {
        Style::default().fg(colors::TEXT_SUBTLE)
    };
    let label_style = if focused || spec.emphasized || spec.active {
        Style::default()
            .fg(colors::TEXT_SOFT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(colors::TEXT_MUTED)
    };

    let line = Line::from(vec![
        Span::styled(format!("{} ", spec.icon), icon_style),
        Span::styled(spec.label.to_string(), label_style),
    ]);

    let paragraph = Paragraph::new(line).style(button_bg(focused || spec.active));
    f.render_widget(paragraph, rect);
    spec.row + 1
}

fn draw_heading(f: &mut Frame, area: Rect, title: &str, bottom: u16) {
    if !has_room(area.y, area.height.max(1), bottom) {
        return;
    }

    let line = Line::from(vec![Span::styled(
        title.to_string(),
        Style::default()
            .fg(colors::TEXT_DIM)
            .add_modifier(Modifier::BOLD),
    )]);
    let paragraph = Paragraph::new(line).style(Style::default().bg(colors::BG_SIDEBAR));
    f.render_widget(paragraph, area);
}

fn draw_project_button(
    f: &mut Frame,
    app: &mut App,
    area: Rect,
    project: &ProjectEntry,
    bottom: u16,
) -> u16 {
    if !has_room(area.y, area.height.max(1), bottom) {
        return bottom;
    }

    let id = format!("project:{}", project.path);
    app.register_ui_target(
        id.clone(),
        area,
        UiAction::OpenProject(project.path.clone()),
    );
    let focused = app.is_ui_focused(&id);

    let lines = vec![
        Line::from(vec![
            Span::styled(
                if project.active { "> " } else { "  " },
                Style::default().fg(if project.active {
                    colors::ACCENT
                } else {
                    colors::TEXT_SUBTLE
                }),
            ),
            Span::styled(
                truncate(&project.name, 14),
                Style::default()
                    .fg(if project.active || focused {
                        colors::TEXT_SOFT
                    } else {
                        colors::TEXT_MUTED
                    })
                    .add_modifier(if project.active || focused {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]),
        Line::from(vec![Span::styled(
            truncate(&project.display_path, 20),
            Style::default().fg(colors::TEXT_SUBTLE),
        )]),
    ];

    let paragraph = Paragraph::new(lines).style(button_bg(focused || project.active));
    f.render_widget(paragraph, area);
    area.y + area.height
}

fn draw_conversation_button(
    f: &mut Frame,
    app: &mut App,
    x: u16,
    width: u16,
    row: u16,
    conversation: &SidebarConversation,
    bottom: u16,
) -> u16 {
    if !has_room(row, 2, bottom) {
        return bottom;
    }

    let id = format!("chat:{}", conversation.id);
    let archive_id = format!("archive:{}", conversation.id);
    let focused = app.is_ui_focused(&id);
    let archive_focused = app.is_ui_focused(&archive_id);

    // Split width: chat label takes most, archive button at end
    let archive_width = 3u16;
    let label_width = width.saturating_sub(archive_width + 1);

    // Register click targets
    let label_rect = Rect::new(x, row, label_width, 2);
    let archive_rect = Rect::new(x + label_width + 1, row, archive_width, 1);
    app.register_ui_target(
        id.clone(),
        label_rect,
        UiAction::OpenConversation(conversation.id.clone()),
    );
    app.register_ui_target(
        archive_id.clone(),
        archive_rect,
        if conversation.archived {
            UiAction::UnarchiveConversation(conversation.id.clone())
        } else {
            UiAction::ArchiveConversation(conversation.id.clone())
        },
    );

    // Draw label
    let label_lines = vec![
        Line::from(vec![Span::styled(
            conversation.label.clone(),
            Style::default()
                .fg(if conversation.active || focused {
                    colors::TEXT_SOFT
                } else {
                    colors::TEXT_MUTED
                })
                .add_modifier(if conversation.active || focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )]),
        Line::from(vec![
            Span::styled(
                truncate(
                    &format!("{} • {}", conversation.mode, conversation.state),
                    16,
                ),
                Style::default().fg(colors::TEXT_SUBTLE),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                truncate(&conversation.last_seen, 12),
                Style::default().fg(colors::TEXT_DIM),
            ),
        ]),
    ];
    let label_para = Paragraph::new(label_lines).style(button_bg(focused || conversation.active));
    f.render_widget(label_para, label_rect);

    // Draw archive button [A] or restore button [R]
    let archive_text = if conversation.archived { "[R]" } else { "[A]" };
    let archive_style = if archive_focused {
        Style::default()
            .fg(if conversation.archived {
                colors::SUCCESS
            } else {
                colors::ERROR
            })
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(colors::TEXT_DIM)
    };
    let archive_line = Line::from(vec![Span::styled(archive_text, archive_style)]);
    let archive_para = Paragraph::new(archive_line);
    f.render_widget(archive_para, archive_rect);

    row + 2
}

fn draw_footer(f: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(colors::TEXT_SOFT)),
        Span::styled(" navigate", Style::default().fg(colors::TEXT_DIM)),
        Span::styled("  ", Style::default()),
        Span::styled("enter", Style::default().fg(colors::TEXT_SOFT)),
        Span::styled(" select", Style::default().fg(colors::TEXT_DIM)),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(colors::BG_SIDEBAR));
    f.render_widget(paragraph, area);
}

fn button_bg(active: bool) -> Style {
    Style::default().bg(if active {
        colors::BG_PANEL
    } else {
        colors::BG_SIDEBAR
    })
}

fn has_room(row: u16, height: u16, bottom: u16) -> bool {
    row < bottom && row.saturating_add(height) <= bottom
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let mut out = text
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>();
        out.push_str("...");
        out
    }
}

fn sidebar_conversations(app: &App) -> Vec<SidebarConversation> {
    let mut entries = vec![SidebarConversation {
        id: app.conversation_id.clone(),
        label: truncate(
            &format!(
                "{} (current)",
                crate::ui::widgets::chat_thread::current_conversation_title(app)
            ),
            20,
        ),
        mode: app.state.execution.mode.as_str().to_string(),
        state: app.state.execution.state.as_str().to_string(),
        last_seen: "active now".to_string(),
        active: true,
        archived: false,
    }];

    let mut seen = HashSet::new();
    seen.insert(app.conversation_id.clone());
    let current_repo = app.state.repo.path.clone();

    for conversation in app.persistence.active_conversations() {
        if seen.contains(&conversation.id) {
            continue;
        }

        if app.state.repo.path != "~"
            && conversation.repo_path.as_deref() != Some(current_repo.as_str())
        {
            continue;
        }

        let label = conversation_label(conversation);
        if label == "Conversation" {
            continue;
        }

        entries.push(SidebarConversation {
            id: conversation.id.clone(),
            label,
            mode: conversation.mode.clone(),
            state: conversation.execution.state.clone(),
            last_seen: relative_time(conversation.updated_at),
            active: false,
            archived: false,
        });
        seen.insert(conversation.id.clone());
    }

    entries
}

fn sidebar_archived_conversations(app: &App) -> Vec<SidebarConversation> {
    let current_repo = app.state.repo.path.clone();
    app.persistence
        .archived_conversations()
        .into_iter()
        .filter(|conversation| conversation.id != app.conversation_id)
        .filter(|conversation| {
            app.state.repo.path == "~"
                || conversation.repo_path.as_deref() == Some(current_repo.as_str())
        })
        .filter_map(|conversation| {
            let label = conversation_label(conversation);
            if label == "Conversation" {
                None
            } else {
                Some(SidebarConversation {
                    id: conversation.id.clone(),
                    label,
                    mode: conversation.mode.clone(),
                    state: conversation.execution.state.clone(),
                    last_seen: relative_time(conversation.updated_at),
                    active: false,
                    archived: true,
                })
            }
        })
        .collect()
}

fn conversation_label(conversation: &PersistentConversation) -> String {
    if conversation.title != "New Conversation" {
        return truncate(&conversation.title, 20);
    }

    if let Some(message) = conversation
        .messages
        .iter()
        .find(|message| message.role == "user")
        .or_else(|| conversation.messages.first())
    {
        return truncate(message.content.lines().next().unwrap_or("Conversation"), 20);
    }

    "Conversation".to_string()
}

#[derive(Clone)]
struct SidebarConversation {
    id: String,
    label: String,
    mode: String,
    state: String,
    last_seen: String,
    active: bool,
    archived: bool,
}

fn relative_time(updated_at: chrono::DateTime<chrono::Local>) -> String {
    let diff = chrono::Local::now().signed_duration_since(updated_at);
    if diff.num_minutes() < 1 {
        "just now".to_string()
    } else if diff.num_minutes() < 60 {
        format!("{}m ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else {
        format!("{}d ago", diff.num_days())
    }
}

struct ButtonSpec {
    id: String,
    action: UiAction,
    row: u16,
    icon: &'static str,
    label: &'static str,
    emphasized: bool,
    active: bool,
}

impl ButtonSpec {
    fn new(
        id: &str,
        action: UiAction,
        row: u16,
        icon: &'static str,
        label: &'static str,
        active: bool,
    ) -> Self {
        Self {
            id: id.to_string(),
            action,
            row,
            icon,
            label,
            emphasized: id == "nav:new_chat",
            active,
        }
    }
}
