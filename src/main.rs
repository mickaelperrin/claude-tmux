mod app;
mod completion;
mod detection;
mod git;
mod input;
mod scroll_state;
mod session;
mod tmux;
mod ui;

use std::io::{self, stdout};

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;

use crate::app::App;

fn main() -> Result<()> {
    // Set up terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    // Run the app
    let result = run(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    // Fast initialization - UI appears immediately
    let mut app = App::new_fast()?;

    // Start background loading of instances and git contexts
    app.start_background_loading();

    loop {
        // Poll for background loading updates (non-blocking)
        app.poll_loading();

        // Draw the UI
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        // Check if we should quit
        if app.should_quit {
            break;
        }

        // Faster poll during loading for responsiveness (~60fps)
        // Normal poll when loading is complete to reduce CPU usage
        let poll_ms = if app.is_loading() { 16 } else { 100 };

        // Handle events
        if event::poll(std::time::Duration::from_millis(poll_ms))? {
            if let Event::Key(key) = event::read()? {
                input::handle_key(&mut app, key);
            }
        }
    }

    Ok(())
}
