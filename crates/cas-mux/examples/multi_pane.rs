//! Multi-pane multiplexer example
//!
//! Demonstrates the Mux with multiple panes and rendering.
//! Run with: cargo run -p cas-mux --example multi_pane

use cas_mux::{Mux, Pane, Renderer};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use std::io;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create a multiplexer
    let mut mux = Mux::new(24, 80);

    // Add some director panes (no PTY, just for layout demo)
    mux.add_pane(Pane::director("worker-1", 24, 25)?);
    mux.add_pane(Pane::director("worker-2", 24, 25)?);
    mux.add_pane(Pane::director("supervisor", 24, 25)?);

    // Feed some content to the panes
    if let Some(pane) = mux.get_mut("worker-1") {
        pane.feed(b"Worker 1 Output\n\x1b[32mGreen text\x1b[0m")?;
    }
    if let Some(pane) = mux.get_mut("worker-2") {
        pane.feed(b"Worker 2 Output\n\x1b[31mRed text\x1b[0m")?;
    }
    if let Some(pane) = mux.get_mut("supervisor") {
        pane.feed(b"Supervisor watching...\n\x1b[1mBold text\x1b[0m")?;
    }

    // Create renderer
    let renderer = Renderer::new();

    // Run the UI loop
    let result = run_app(&mut terminal, &mut mux, &renderer);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    if let Err(e) = result {
        eprintln!("Error: {e}");
    }

    println!("\n✓ Multi-pane demo complete!");
    println!("\nPane info:");
    for id in mux.pane_ids() {
        let pane = mux.get(id).unwrap();
        println!(
            "  - {} ({:?}) {}",
            id,
            pane.kind(),
            if pane.is_focused() { "[FOCUSED]" } else { "" }
        );
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mux: &mut Mux,
    renderer: &Renderer,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| {
            renderer.render(frame, mux);
        })?;

        // Poll for events with timeout
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Tab => mux.focus_next(),
                KeyCode::BackTab => mux.focus_prev(),
                KeyCode::Char('1') => {
                    mux.focus("worker-1");
                }
                KeyCode::Char('2') => {
                    mux.focus("worker-2");
                }
                KeyCode::Char('3') => {
                    mux.focus("supervisor");
                }
                _ => {}
            }
        }
    }
}
