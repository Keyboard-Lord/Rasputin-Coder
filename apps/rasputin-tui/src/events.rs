use crate::app::{App, UiAction};
use crate::state::ExecutionMode;
use crate::state::InputMode;
use anyhow::Result;
use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;
use tracing::{debug, trace};

pub struct EventHandler {
    pub tick_rate: u64, // milliseconds
}

impl EventHandler {
    pub fn new(tick_rate: u64) -> Self {
        Self { tick_rate }
    }

    pub fn poll(&self, timeout: u64) -> Result<bool> {
        Ok(event::poll(Duration::from_millis(timeout))?)
    }

    pub fn next(&self) -> Result<Option<Event>> {
        match event::read()? {
            CrosstermEvent::Key(key) => {
                trace!("Key event: {:?}", key);
                Ok(Some(Event::Key(key)))
            }
            CrosstermEvent::Mouse(mouse) => {
                trace!("Mouse event: {:?}", mouse);
                Ok(Some(Event::Mouse(mouse)))
            }
            CrosstermEvent::Resize(width, height) => {
                debug!("Resize event: {}x{}", width, height);
                Ok(Some(Event::Resize(width, height)))
            }
            _ => Ok(None),
        }
    }
}

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(event::MouseEvent),
    Resize(u16, u16),
}

pub fn handle_event(event: Event, app: &mut App) -> Result<bool> {
    match event {
        Event::Key(key) => handle_key_event(key, app),
        Event::Mouse(mouse) => app.handle_mouse_event(mouse),
        Event::Resize(width, height) => {
            app.layout.resize(width, height);
            Ok(false)
        }
    }
}

fn handle_key_event(key: KeyEvent, app: &mut App) -> Result<bool> {
    match app.state.input_mode {
        InputMode::Normal => handle_normal_mode(key, app),
        InputMode::Editing => handle_editing_mode(key, app),
    }
}

fn handle_normal_mode(key: KeyEvent, app: &mut App) -> Result<bool> {
    match key.code {
        // Quit
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.quit();
            return Ok(true);
        }

        // Enter editing mode
        KeyCode::Char('i') => {
            app.set_input_mode(InputMode::Editing);
        }

        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.can_switch_execution_mode() {
                let target_mode = match app.state.execution.mode {
                    ExecutionMode::Task => ExecutionMode::Edit,
                    ExecutionMode::Edit => ExecutionMode::Chat,
                    ExecutionMode::Chat => ExecutionMode::Task,
                };
                let _ = app.perform_ui_action(UiAction::SetExecutionMode(target_mode));
            }
        }

        // BUG FIX: Chat scroll keys - always scroll chat, not sidebar
        // Arrow keys scroll chat history
        KeyCode::Up => {
            if app
                .state
                .focus_state
                .has_focus(crate::app::FocusTarget::ChatPane)
            {
                app.state.chat_scroll.scroll(3);
            } else {
                app.focus_previous_ui();
            }
        }
        KeyCode::Down => {
            if app
                .state
                .focus_state
                .has_focus(crate::app::FocusTarget::ChatPane)
            {
                app.state.chat_scroll.scroll(-3);
            } else {
                app.focus_next_ui();
            }
        }

        // Page keys scroll chat
        KeyCode::PageUp => {
            app.state.chat_scroll.page_up();
        }
        KeyCode::PageDown => {
            app.state.chat_scroll.page_down();
        }

        // Jump shortcuts
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.state.chat_scroll.jump_to_first();
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.state.chat_scroll.jump_to_latest();
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.state.chat_scroll.jump_to_latest();
        }

        // Phase 2: Copy from chat
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app
                .state
                .focus_state
                .has_focus(crate::app::FocusTarget::ChatPane)
            {
                app.copy_current_message();
            }
        }

        KeyCode::Enter => {
            return app.activate_focused_ui();
        }

        // Cycle focus (Shift+Tab goes backwards)
        KeyCode::Tab if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.state.focus_state.cycle_next();
        }
        KeyCode::BackTab | KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.state.focus_state.cycle_prev();
        }

        // Open most recent browser preview
        KeyCode::Char('o') => {
            if let Some(server) = app.state.preview_servers.last() {
                let _ = app.perform_ui_action(crate::app::UiAction::OpenBrowserPreview {
                    url: server.url.clone(),
                });
            }
        }

        _ => {}
    }

    Ok(false)
}

fn handle_editing_mode(key: KeyEvent, app: &mut App) -> Result<bool> {
    match key.code {
        // Quit
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
            return Ok(true);
        }

        // Submit message
        KeyCode::Enter => {
            if app.state.input_buffer.trim().is_empty() && app.has_focused_ui() {
                return app.activate_focused_ui();
            }
        }

        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.can_switch_execution_mode() {
                let target_mode = match app.state.execution.mode {
                    ExecutionMode::Task => ExecutionMode::Edit,
                    ExecutionMode::Edit => ExecutionMode::Chat,
                    ExecutionMode::Chat => ExecutionMode::Task,
                };
                let _ = app.perform_ui_action(UiAction::SetExecutionMode(target_mode));
            }
        }

        // Exit editing mode
        KeyCode::Esc => {
            if app.cancel_project_create_workflow() {
                return Ok(false);
            }
            app.set_input_mode(InputMode::Normal);
        }

        // Character input
        KeyCode::Char(c) => {
            app.handle_input(c);
        }

        // Backspace
        KeyCode::Backspace => {
            app.handle_backspace();
        }

        // Delete
        KeyCode::Delete => {
            app.handle_delete();
        }

        // Cursor movement
        KeyCode::Left => app.move_cursor_left(),
        KeyCode::Right => app.move_cursor_right(),
        KeyCode::Home => app.move_cursor_home(),
        KeyCode::End => app.move_cursor_end(),

        // Scroll chat while editing
        KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_up();
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_down();
        }

        KeyCode::Up if app.state.input_buffer.is_empty() => {
            app.focus_previous_ui();
        }
        KeyCode::Down if app.state.input_buffer.is_empty() => {
            app.focus_next_ui();
        }

        // Toggle inspector
        KeyCode::Tab => app.toggle_inspector(),

        _ => {}
    }

    Ok(false)
}
