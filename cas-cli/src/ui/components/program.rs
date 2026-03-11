//! Program runner — event loop for interactive components (inline mode)
//!
//! The Program drives a Component through its lifecycle:
//! 1. Render the initial view
//! 2. Read keyboard events from the terminal
//! 3. Map events to component messages via a user-provided handler
//! 4. Call update, re-render
//! 5. Exit on Action::Quit
//!
//! This operates in "inline mode" — it does not enter alternate screen.
//! Components render in-place in the terminal scroll buffer.

use std::io::{self, IsTerminal, Write, stdout};
use std::time::Duration;

use crossterm::cursor::{MoveToColumn, MoveUp, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode};

use super::formatter::{Formatter, emit};
use super::traits::{Action, Component};
use crate::ui::theme::ActiveTheme;

/// Configuration for the Program runner.
pub struct ProgramConfig {
    /// Poll timeout for keyboard events (default: 100ms)
    pub tick_rate: Duration,
    /// Theme to use for rendering
    pub theme: ActiveTheme,
}

impl Default for ProgramConfig {
    fn default() -> Self {
        Self {
            tick_rate: Duration::from_millis(100),
            theme: ActiveTheme::default(),
        }
    }
}

impl ProgramConfig {
    /// Set the theme.
    pub fn with_theme(mut self, theme: ActiveTheme) -> Self {
        self.theme = theme;
        self
    }

    /// Set the tick rate.
    pub fn with_tick_rate(mut self, duration: Duration) -> Self {
        self.tick_rate = duration;
        self
    }
}

/// Run an interactive component in inline mode.
///
/// The `key_handler` maps raw keyboard events to component messages.
/// Return `Some(msg)` to deliver a message, or `None` to ignore the event.
///
/// Returns the component's output after it quits.
///
/// ```ignore
/// use crossterm::event::{KeyCode, KeyEvent};
///
/// let output = run(
///     &mut my_component,
///     ProgramConfig::default().with_theme(theme),
///     |key| match key.code {
///         KeyCode::Up => Some(MyMsg::Up),
///         KeyCode::Down => Some(MyMsg::Down),
///         KeyCode::Enter => Some(MyMsg::Confirm),
///         KeyCode::Esc => Some(MyMsg::Cancel),
///         _ => None,
///     },
/// )?;
/// ```
pub fn run<C, F>(component: &mut C, config: ProgramConfig, key_handler: F) -> io::Result<C::Output>
where
    C: Component,
    F: Fn(&KeyEvent) -> Option<C::Msg>,
{
    let mut out = stdout();
    enable_raw_mode()?;

    // Render initial view and count lines
    let mut prev_lines = render_view(component, &config.theme, &mut out)?;

    let result: io::Result<()> = loop {
        if event::poll(config.tick_rate)? {
            if let Event::Key(key) = event::read()? {
                // Ctrl+C always exits
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break Ok(());
                }

                if let Some(msg) = key_handler(&key) {
                    let action = component.update(msg);

                    match action {
                        Action::None => {}
                        Action::Cmd(cmd) => {
                            let msg = cmd();
                            component.update(msg);
                        }
                        Action::Batch(cmds) => {
                            for cmd in cmds {
                                let msg = cmd();
                                component.update(msg);
                            }
                        }
                        Action::Quit => break Ok(()),
                    }

                    // Clear previous output and re-render
                    clear_lines(&mut out, prev_lines)?;
                    prev_lines = render_view(component, &config.theme, &mut out)?;
                }
            }
        }
    };

    disable_raw_mode()?;
    emit(&mut out, &Show)?;
    out.flush()?;

    result?;
    Ok(component.output())
}

/// Render a component's view inline to stdout.
/// Returns the number of lines written.
///
/// Use this for driving Component rendering outside the full Program event loop,
/// e.g., progress indicators in synchronous loops.
pub fn render_inline_view<C: Component>(component: &C, theme: &ActiveTheme) -> io::Result<u16> {
    let mut out = stdout();
    render_view(component, theme, &mut out)
}

/// Clear previous inline output and re-render a component.
/// Returns the new line count.
///
/// In non-TTY mode, skips clearing (no cursor movement on pipes).
pub fn rerender_inline<C: Component>(
    component: &C,
    prev_lines: u16,
    theme: &ActiveTheme,
) -> io::Result<u16> {
    let mut out = stdout();
    if out.is_terminal() {
        clear_lines(&mut out, prev_lines)?;
    }
    render_view(component, theme, &mut out)
}

/// Clear N previously rendered inline lines.
///
/// No-op in non-TTY mode.
pub fn clear_inline(prev_lines: u16) -> io::Result<()> {
    let mut out = stdout();
    if out.is_terminal() {
        clear_lines(&mut out, prev_lines)?;
        out.flush()?;
    }
    Ok(())
}

/// Render a component's view and return the number of lines written.
fn render_view<C: Component>(
    component: &C,
    theme: &ActiveTheme,
    out: &mut dyn Write,
) -> io::Result<u16> {
    let mut buf = Vec::new();
    {
        let mut fmt = Formatter::stdout(&mut buf, theme.clone());
        component.view(&mut fmt)?;
    }

    out.write_all(&buf)?;
    out.flush()?;

    // Count newlines to know how many lines to clear on re-render
    let line_count = buf.iter().filter(|&&b| b == b'\n').count() as u16;
    Ok(line_count)
}

/// Clear N lines above the cursor position (move up and clear each line).
fn clear_lines(out: &mut dyn Write, lines: u16) -> io::Result<()> {
    if lines > 0 {
        emit(out, &MoveUp(lines))?;
        emit(out, &MoveToColumn(0))?;
        for _ in 0..lines {
            emit(out, &Clear(ClearType::CurrentLine))?;
            writeln!(out)?;
        }
        emit(out, &MoveUp(lines))?;
        emit(out, &MoveToColumn(0))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::formatter::Formatter;
    use crate::ui::components::traits::{Action, Component};

    /// A minimal component for testing the program runner logic.
    struct Toggle {
        on: bool,
        toggle_count: u32,
    }

    #[allow(dead_code)]
    enum ToggleMsg {
        Flip,
        Quit,
    }

    impl Component for Toggle {
        type Msg = ToggleMsg;
        type Output = (bool, u32);

        fn update(&mut self, msg: ToggleMsg) -> Action<ToggleMsg> {
            match msg {
                ToggleMsg::Flip => {
                    self.on = !self.on;
                    self.toggle_count += 1;
                    Action::None
                }
                ToggleMsg::Quit => Action::Quit,
            }
        }

        fn view(&self, fmt: &mut Formatter) -> io::Result<()> {
            let state = if self.on { "ON" } else { "OFF" };
            fmt.write_primary(&format!("Toggle: {state}"))?;
            fmt.newline()?;
            fmt.write_muted(&format!("Flipped {} times", self.toggle_count))?;
            fmt.newline()
        }

        fn output(&self) -> (bool, u32) {
            (self.on, self.toggle_count)
        }
    }

    #[test]
    fn test_component_renders_to_buffer() {
        let component = Toggle {
            on: false,
            toggle_count: 0,
        };

        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            component.view(&mut fmt).unwrap();
        }

        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("Toggle: OFF"));
        assert!(output.contains("Flipped 0 times"));
    }

    #[test]
    fn test_component_updates_state() {
        let mut component = Toggle {
            on: false,
            toggle_count: 0,
        };

        component.update(ToggleMsg::Flip);
        assert!(component.on);
        assert_eq!(component.toggle_count, 1);

        component.update(ToggleMsg::Flip);
        assert!(!component.on);
        assert_eq!(component.toggle_count, 2);

        assert_eq!(component.output(), (false, 2));
    }

    #[test]
    fn test_render_view_counts_lines() {
        let component = Toggle {
            on: true,
            toggle_count: 5,
        };

        let theme = ActiveTheme::default_dark();
        let mut buf = Vec::new();
        let lines = render_view(&component, &theme, &mut buf).unwrap();

        // Toggle view writes 2 lines (2 newlines)
        assert_eq!(lines, 2);
    }

    #[test]
    fn test_program_config_defaults() {
        let config = ProgramConfig::default();
        assert_eq!(config.tick_rate, Duration::from_millis(100));
    }

    #[test]
    fn test_program_config_builder() {
        let theme = ActiveTheme::default_dark();
        let config = ProgramConfig::default()
            .with_theme(theme)
            .with_tick_rate(Duration::from_millis(50));

        assert_eq!(config.tick_rate, Duration::from_millis(50));
    }

    #[test]
    fn test_clear_lines_writes_ansi() {
        let mut buf = Vec::new();
        clear_lines(&mut buf, 3).unwrap();
        let output = String::from_utf8(buf).unwrap();
        // Should contain cursor movement sequences
        assert!(output.contains("\x1b["));
    }

    #[test]
    fn test_clear_lines_zero_is_noop() {
        let mut buf = Vec::new();
        clear_lines(&mut buf, 0).unwrap();
        assert!(buf.is_empty());
    }
}
