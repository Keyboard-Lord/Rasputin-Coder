#![deny(unused_must_use)]
#![allow(dead_code)]

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::fs::{self, OpenOptions};
use std::io;
use tracing::{error, info, warn};
use tracing_subscriber::fmt::writer::BoxMakeWriter;

mod app;
mod autonomy;
mod bootstrap;
mod browser;
mod clipboard;
mod commands;
mod diff;
mod events;
mod forge_runtime;
mod goal_planner;
mod guidance;
mod host_actions;
mod interface_integration;
mod observability;
mod ollama;
mod persistence;
mod repo;
mod state;
mod syntax;
mod ui;
mod validation;
#[cfg(test)]
mod validation_tests;

use app::App;
use bootstrap::LaunchIntent;
use events::EventHandler;

fn init_logging() {
    let data_dir = persistence::PersistentState::data_dir();
    let log_path = data_dir.join("rasputin.log");

    let writer = if fs::create_dir_all(&data_dir).is_ok() {
        match OpenOptions::new().create(true).append(true).open(&log_path) {
            Ok(file) => BoxMakeWriter::new(move || {
                file.try_clone().expect("failed to clone Rasputin log file")
            }),
            Err(_) => BoxMakeWriter::new(io::sink),
        }
    } else {
        BoxMakeWriter::new(io::sink)
    };

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[tokio::main]
async fn main() -> Result<()> {
    let launch_intent = LaunchIntent::from_env_args();

    init_logging();
    info!("Rasputin TUI starting...");

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    // Create app (async - loads persistence)
    let mut app: app::App = App::new().await;
    let mut event_handler = EventHandler::new(250); // 250ms tick rate

    // Initialize app (restore session state, attach workspace, verify runtime)
    if let Err(e) = app.initialize(launch_intent.workspace_path()).await {
        error!("Failed to initialize app: {}", e);
        // Continue anyway - user can attach repo manually
    }

    // Run main loop
    let result = run_app(&mut app, terminal, &mut event_handler).await;

    // Cleanup
    cleanup_terminal()?;

    if let Err(e) = result {
        warn!("Application error: {}", e);
        eprintln!("Error: {}", e);
    }

    info!("Rasputin TUI shutting down...");
    Ok(())
}

async fn run_app<B: Backend>(
    app: &mut App,
    mut terminal: Terminal<B>,
    event_handler: &mut EventHandler,
) -> Result<()> {
    let mut last_tick = tokio::time::Instant::now();

    loop {
        // Draw UI
        terminal.draw(|f| ui::draw(f, app))?;

        if process_pending_command(app).await? {
            break;
        }

        // Poll execution runtime events
        if app.poll_execution_events() {
            // Events were processed, UI will show them on next draw

            // Phase C: Check for auto-resume after chain step completion
            // This runs asynchronously to allow proper state persistence
            if app.try_auto_resume_chain().await {
                // Auto-resume triggered - next loop iteration will process new events
            }
        }

        // Handle events with timeout for smooth rendering
        let timeout = event_handler
            .tick_rate
            .saturating_sub(last_tick.elapsed().as_millis() as u64);

        if event_handler.poll(timeout)?
            && let Some(event) = event_handler.next()?
        {
            // Capture the current input before handling Enter so the command can be queued.
            let pending_submit = match &event {
                events::Event::Key(key)
                    if key.code == crossterm::event::KeyCode::Enter
                        && !app.state.input_buffer.trim().is_empty() =>
                {
                    Some(app.state.input_buffer.clone())
                }
                _ => None,
            };

            if events::handle_event(event, app)? {
                break;
            }

            if pending_submit.is_some() {
                match app.submit_active_input().await {
                    Ok(should_quit) => {
                        if should_quit {
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Submit failed: {}", e);
                    }
                }
            }

            if process_pending_command(app).await? {
                break;
            }
        }

        // Handle time-based updates
        if last_tick.elapsed() >= tokio::time::Duration::from_millis(event_handler.tick_rate) {
            app.on_tick();
            last_tick = tokio::time::Instant::now();
        }
    }

    Ok(())
}

async fn process_pending_command(app: &mut App) -> Result<bool> {
    if let Some(command) = app.take_pending_command() {
        match app.handle_command(command).await {
            Ok(should_quit) => Ok(should_quit),
            Err(e) => {
                error!("Command failed: {}", e);
                Ok(false)
            }
        }
    } else {
        Ok(false)
    }
}

fn cleanup_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}
