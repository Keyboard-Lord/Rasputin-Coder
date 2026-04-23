use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::Style,
    widgets::Block,
};

pub mod layout;
pub mod widgets;

use widgets::{chat_thread, input_bar, inspector, sidebar, status_bar};

/// Color palette matching the web UI's dark, minimal aesthetic
pub mod colors {
    use ratatui::style::Color;

    pub const BG_PAGE: Color = Color::Rgb(11, 11, 13);
    pub const BG_SIDEBAR: Color = Color::Rgb(14, 15, 18);
    pub const BG_PANEL: Color = Color::Rgb(21, 22, 26);
    pub const BG_RAISED: Color = Color::Rgb(26, 28, 33);
    pub const TEXT_SOFT: Color = Color::Rgb(216, 218, 224);
    pub const TEXT_MUTED: Color = Color::Rgb(143, 147, 156);
    pub const TEXT_SUBTLE: Color = Color::Rgb(90, 94, 102);
    pub const TEXT_DIM: Color = Color::Rgb(58, 62, 70);

    pub const BORDER_SUBTLE: Color = Color::Rgb(30, 32, 38);
    pub const BORDER_LIGHT: Color = Color::Rgb(38, 40, 46);

    pub const ACCENT: Color = Color::Rgb(196, 92, 62);
    pub const ACCENT_SOFT: Color = Color::Rgb(82, 42, 33);

    pub const SUCCESS: Color = Color::Rgb(90, 143, 110);
    pub const WARNING: Color = Color::Rgb(180, 140, 80);
    pub const ERROR: Color = Color::Rgb(143, 74, 74);
}

/// Main draw function - coordinates all UI rendering
pub fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();
    app.begin_frame();

    // Background color
    f.render_widget(
        Block::default().style(Style::default().bg(colors::BG_PAGE)),
        size,
    );

    // Root layout: sidebar + main + optional inspector
    let shell = if app.show_inspector {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(24),
                Constraint::Min(40),
                Constraint::Length(36),
            ])
            .split(size)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(48)])
            .split(size)
    };
    sidebar::draw(f, app, shell[0]);

    // Main area: header + chat + input + status bar
    let main_area = shell[1];
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Conversation header
            Constraint::Min(0),    // Chat thread
            Constraint::Length(5), // Input composer
            Constraint::Length(1), // Status bar
        ])
        .split(main_area);

    // Draw conversation header
    chat_thread::draw_header(f, app, main_chunks[0]);

    // Draw chat thread
    chat_thread::draw(f, app, main_chunks[1]);

    // Draw input bar
    input_bar::draw(f, app, main_chunks[2]);

    // Draw global status bar
    status_bar::draw(f, app, main_chunks[3]);

    // Draw inspector if visible
    if app.show_inspector && shell.len() > 2 {
        inspector::draw(f, app, shell[2]);
    }
}
