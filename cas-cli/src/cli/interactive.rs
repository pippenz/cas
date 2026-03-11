//! Interactive prompts for CLI commands

use std::io::{self, BufRead, Write};

use crossterm::event::KeyModifiers;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode},
};

/// Read a line from stdin with a prompt
pub fn prompt(message: &str) -> io::Result<String> {
    write!(io::stdout(), "{message}: ")?;
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

/// Read a line from stdin with a prompt and default value
pub fn prompt_default(message: &str, default: &str) -> io::Result<String> {
    write!(io::stdout(), "{message} [{default}]: ")?;
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input.to_string())
    }
}

/// Read optional input (empty string allowed)
pub fn prompt_optional(message: &str) -> io::Result<Option<String>> {
    write!(io::stdout(), "{message} (optional): ")?;
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input.to_string()))
    }
}

/// Read multi-line content until empty line
pub fn prompt_multiline(message: &str) -> io::Result<String> {
    writeln!(io::stdout(), "{message}:")?;
    writeln!(
        io::stdout(),
        "(Enter text, then press Enter twice to finish)"
    )?;

    let mut lines = Vec::new();
    let stdin = io::stdin();

    loop {
        let mut input = String::new();
        stdin.lock().read_line(&mut input)?;
        let line = input.trim_end_matches('\n').trim_end_matches('\r');

        if line.is_empty() && !lines.is_empty() {
            break;
        }

        lines.push(line.to_string());
    }

    Ok(lines.join("\n").trim().to_string())
}

/// Present options and get selection
pub fn select(message: &str, options: &[&str]) -> io::Result<usize> {
    writeln!(io::stdout(), "{message}:")?;
    for (i, option) in options.iter().enumerate() {
        writeln!(io::stdout(), "  {}. {}", i + 1, option)?;
    }

    loop {
        write!(io::stdout(), "Select (1-{}): ", options.len())?;
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;

        if let Ok(n) = input.trim().parse::<usize>() {
            if n >= 1 && n <= options.len() {
                return Ok(n - 1);
            }
        }

        writeln!(io::stdout(), "Invalid selection, please try again.")?;
    }
}

/// Prompt for a number with default value
pub fn prompt_number(message: &str, default: usize) -> io::Result<usize> {
    let input = prompt_default(message, &default.to_string())?;
    Ok(input.parse().unwrap_or(default))
}

/// Prompt for an i32 number with default value
pub fn prompt_i32(message: &str, default: i32) -> io::Result<i32> {
    let input = prompt_default(message, &default.to_string())?;
    Ok(input.parse().unwrap_or(default))
}

/// Confirm yes/no
pub fn confirm(message: &str, default: bool) -> io::Result<bool> {
    let prompt_text = if default {
        format!("{message} [Y/n]")
    } else {
        format!("{message} [y/N]")
    };

    write!(io::stdout(), "{prompt_text}: ")?;
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    Ok(match input.as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        "" => default,
        _ => default,
    })
}

/// A styled column value within a picker row.
pub struct PickerTag {
    pub text: String,
    pub color: Color,
}

/// An item (row) in the interactive picker.
pub struct PickerItem {
    /// Main label (first column, highlighted when selected)
    pub label: String,
    /// Additional columns rendered after the label, each with its own color
    pub tags: Vec<PickerTag>,
}

/// Pre-computed column widths for aligned rendering.
struct ColumnWidths {
    label: usize,
    tags: Vec<usize>,
}

impl ColumnWidths {
    fn compute(items: &[PickerItem]) -> Self {
        let label = items.iter().map(|i| i.label.len()).max().unwrap_or(0);
        let max_tags = items.iter().map(|i| i.tags.len()).max().unwrap_or(0);
        let mut tags = vec![0usize; max_tags];
        for item in items {
            for (col, tag) in item.tags.iter().enumerate() {
                tags[col] = tags[col].max(tag.text.len());
            }
        }
        Self { label, tags }
    }
}

/// Bubbletea-style inline picker. Returns the selected index, or None if cancelled.
///
/// Renders inline (no alternate screen) with aligned columns, supports
/// arrow keys / j/k, enter to confirm, q/esc/ctrl-c to cancel.
/// Cleans up after itself and prints a completed-form summary line.
pub fn pick(title: &str, items: &[PickerItem]) -> io::Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }

    let mut stdout = io::stdout();
    let mut selected: usize = 0;
    let cols = ColumnWidths::compute(items);

    // Reserve vertical space so the terminal scrolls enough for our content.
    let total_lines = items.len() + 3; // title + blank + items + help
    for _ in 0..total_lines {
        writeln!(stdout)?;
    }
    stdout.flush()?;

    let (_, cur_row) = crossterm::cursor::position()?;
    let origin_row = cur_row.saturating_sub(total_lines as u16);

    enable_raw_mode()?;
    execute!(stdout, Hide)?;

    render_picker(&mut stdout, title, items, selected, origin_row, &cols)?;

    let result = loop {
        if !event::poll(std::time::Duration::from_millis(50))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                break None;
            }

            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = if selected == 0 {
                        items.len() - 1
                    } else {
                        selected - 1
                    };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = if selected >= items.len() - 1 {
                        0
                    } else {
                        selected + 1
                    };
                }
                KeyCode::Enter => break Some(selected),
                KeyCode::Esc | KeyCode::Char('q') => break None,
                _ => continue,
            }
            render_picker(&mut stdout, title, items, selected, origin_row, &cols)?;
        }
    };

    disable_raw_mode()?;
    execute!(stdout, Show)?;

    // Clear picker area
    for r in 0..total_lines {
        execute!(
            stdout,
            MoveTo(0, origin_row + r as u16),
            Clear(ClearType::CurrentLine),
        )?;
    }
    execute!(stdout, MoveTo(0, origin_row))?;

    // Completed-form summary line
    match result {
        Some(idx) => {
            execute!(
                stdout,
                SetForegroundColor(Color::Green),
                Print("  ✓ "),
                ResetColor,
                SetAttribute(Attribute::Bold),
                Print(title),
                SetAttribute(Attribute::Reset),
                SetForegroundColor(Color::DarkGrey),
                Print(": "),
                ResetColor,
                Print(&items[idx].label),
                Print("\n"),
            )?;
        }
        None => {
            execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print("  ✗ "),
                Print(title),
                Print(": cancelled"),
                ResetColor,
                Print("\n"),
            )?;
        }
    }

    Ok(result)
}

/// Pad `s` with spaces to `width`.
fn pad(s: &str, width: usize) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - s.len()))
    }
}

fn render_picker(
    stdout: &mut io::Stdout,
    title: &str,
    items: &[PickerItem],
    selected: usize,
    origin_row: u16,
    cols: &ColumnWidths,
) -> io::Result<()> {
    let mut row = origin_row;

    // Title
    queue!(
        stdout,
        MoveTo(0, row),
        Clear(ClearType::CurrentLine),
        SetAttribute(Attribute::Bold),
        Print(format!("  {title}")),
        SetAttribute(Attribute::Reset),
    )?;
    row += 1;

    // Blank line
    queue!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    row += 1;

    // Rows
    for (i, item) in items.iter().enumerate() {
        queue!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;

        let label_padded = pad(&item.label, cols.label);

        if i == selected {
            // Cursor + bold label column
            queue!(
                stdout,
                SetForegroundColor(Color::Magenta),
                SetAttribute(Attribute::Bold),
                Print("  ❯ "),
                SetAttribute(Attribute::Reset),
                SetAttribute(Attribute::Bold),
                Print(&label_padded),
                SetAttribute(Attribute::Reset),
            )?;
            // Tag columns in their own colors, padded
            for (col, tag) in item.tags.iter().enumerate() {
                let w = cols.tags.get(col).copied().unwrap_or(0);
                queue!(
                    stdout,
                    Print("  "),
                    SetForegroundColor(tag.color),
                    Print(pad(&tag.text, w)),
                    ResetColor,
                )?;
            }
        } else {
            // Dimmed label column
            queue!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print(format!("    {label_padded}")),
            )?;
            // Dimmed tag columns, padded
            for (col, tag) in item.tags.iter().enumerate() {
                let w = cols.tags.get(col).copied().unwrap_or(0);
                queue!(stdout, Print(format!("  {}", pad(&tag.text, w))))?;
            }
            queue!(stdout, ResetColor)?;
        }

        row += 1;
    }

    // Help line
    queue!(
        stdout,
        MoveTo(0, row),
        Clear(ClearType::CurrentLine),
        SetForegroundColor(Color::DarkGrey),
        Print("  ↑/↓ navigate • enter select • q cancel"),
        ResetColor,
    )?;

    stdout.flush()?;
    Ok(())
}
