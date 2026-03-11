//! Simple test TUI for e2e testing
//!
//! A minimal ratatui-based TUI application that responds to keyboard input,
//! displays menus, and navigates between screens.
//!
//! Build with: cargo build -p cas-tui-test --bin test_tui --features test-tui
//! Run with: cargo run -p cas-tui-test --bin test_tui --features test-tui

#[cfg(feature = "test-tui")]
use std::io;
#[cfg(feature = "test-tui")]
use std::io::Stdout;

#[cfg(feature = "test-tui")]
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
#[cfg(feature = "test-tui")]
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

#[cfg(not(feature = "test-tui"))]
fn main() -> std::io::Result<()> {
    Ok(())
}

#[cfg(feature = "test-tui")]
fn main() -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let result = run_app(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

#[cfg(feature = "test-tui")]
fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let mut app = App::default();

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match app.screen {
                Screen::Menu => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
                    KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
                    KeyCode::Enter => app.menu_select(),
                    _ => {}
                },
                Screen::Counter => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('+') | KeyCode::Char('=') => app.counter += 1,
                    KeyCode::Char('-') | KeyCode::Char('_') => app.counter -= 1,
                    KeyCode::Esc | KeyCode::Backspace => app.go_back(),
                    _ => {}
                },
                Screen::Input => match key.code {
                    KeyCode::Char('q') if app.input_text.is_empty() => return Ok(()),
                    KeyCode::Char(c) => app.input_text.push(c),
                    KeyCode::Backspace => {
                        if app.input_text.is_empty() {
                            app.go_back();
                        } else {
                            app.input_text.pop();
                        }
                    }
                    KeyCode::Enter => {
                        app.submitted_text = Some(app.input_text.clone());
                        app.input_text.clear();
                    }
                    KeyCode::Esc => app.go_back(),
                    _ => {}
                },
                Screen::About => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter => app.go_back(),
                    _ => {}
                },
            }
        }
    }
}

#[cfg(feature = "test-tui")]
#[derive(Default)]
struct App {
    screen: Screen,
    menu_state: ListState,
    counter: i32,
    input_text: String,
    submitted_text: Option<String>,
}

#[cfg(feature = "test-tui")]
#[derive(Default, Clone, Copy, PartialEq)]
enum Screen {
    #[default]
    Menu,
    Counter,
    Input,
    About,
}

#[cfg(feature = "test-tui")]
impl App {
    fn menu_up(&mut self) {
        let i = match self.menu_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.menu_state.select(Some(i));
    }

    fn menu_down(&mut self) {
        let i = match self.menu_state.selected() {
            Some(i) => (i + 1).min(3), // 4 menu items (0-3)
            None => 0,
        };
        self.menu_state.select(Some(i));
    }

    fn menu_select(&mut self) {
        match self.menu_state.selected() {
            Some(0) => self.screen = Screen::Counter,
            Some(1) => self.screen = Screen::Input,
            Some(2) => self.screen = Screen::About,
            Some(3) => std::process::exit(0), // Quit
            _ => {}
        }
    }

    fn go_back(&mut self) {
        self.screen = Screen::Menu;
    }
}

#[cfg(feature = "test-tui")]
fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(10),   // Content
            Constraint::Length(3), // Status bar
        ])
        .split(f.area());

    // Title
    let title = match app.screen {
        Screen::Menu => "Test TUI - Main Menu",
        Screen::Counter => "Test TUI - Counter",
        Screen::Input => "Test TUI - Input",
        Screen::About => "Test TUI - About",
    };
    let title_block = Paragraph::new(title)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title_block, chunks[0]);

    // Content based on screen
    match app.screen {
        Screen::Menu => render_menu(f, app, chunks[1]),
        Screen::Counter => render_counter(f, app, chunks[1]),
        Screen::Input => render_input(f, app, chunks[1]),
        Screen::About => render_about(f, chunks[1]),
    }

    // Status bar
    let status = match app.screen {
        Screen::Menu => "↑/↓: Navigate | Enter: Select | q: Quit",
        Screen::Counter => "+/-: Change | Esc: Back | q: Quit",
        Screen::Input => "Type text | Enter: Submit | Esc: Back",
        Screen::About => "Enter/Esc: Back | q: Quit",
    };
    let status_bar = Paragraph::new(status)
        .style(Style::default().fg(Color::White).bg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL).title("Help"));
    f.render_widget(status_bar, chunks[2]);
}

#[cfg(feature = "test-tui")]
fn render_menu(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = vec![
        ListItem::new("  Counter   - Increment/decrement a value"),
        ListItem::new("  Input     - Text input example"),
        ListItem::new("  About     - About this application"),
        ListItem::new("  Quit      - Exit the application"),
    ];

    // Initialize selection if needed
    if app.menu_state.selected().is_none() {
        app.menu_state.select(Some(0));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Menu"))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, &mut app.menu_state);
}

#[cfg(feature = "test-tui")]
fn render_counter(f: &mut Frame, app: &App, area: Rect) {
    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Current Value: "),
            Span::styled(
                format!("{}", app.counter),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from("  Press + or - to change the counter"),
        Line::from("  Press Esc or Backspace to go back"),
    ];

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Counter"));
    f.render_widget(paragraph, area);
}

#[cfg(feature = "test-tui")]
fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let mut text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Input: "),
            Span::styled(&app.input_text, Style::default().fg(Color::Green)),
            Span::styled("█", Style::default().fg(Color::White)), // Cursor
        ]),
        Line::from(""),
    ];

    if let Some(ref submitted) = app.submitted_text {
        text.push(Line::from(vec![
            Span::raw("  Submitted: "),
            Span::styled(submitted, Style::default().fg(Color::Cyan)),
        ]));
    }

    text.push(Line::from(""));
    text.push(Line::from("  Type something and press Enter to submit"));

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(paragraph, area);
}

#[cfg(feature = "test-tui")]
fn render_about(f: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(""),
        Line::from("  Test TUI Application"),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Version: "),
            Span::styled("1.0.0", Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from("  A simple ratatui-based TUI for testing"),
        Line::from("  the cas-tui-test framework."),
        Line::from(""),
        Line::from("  Features:"),
        Line::from("    • Menu navigation with arrow keys"),
        Line::from("    • Counter with +/- controls"),
        Line::from("    • Text input field"),
        Line::from("    • Screen transitions"),
        Line::from(""),
        Line::from("  Press Enter or Esc to go back"),
    ];

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("About"));
    f.render_widget(paragraph, area);
}
