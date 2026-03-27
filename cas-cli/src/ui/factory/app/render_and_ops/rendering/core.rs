use crate::ui::factory::app::imports::*;

impl FactoryApp {
    fn sync_selected_worker_tab_with_focus(&mut self) {
        let Some(focused) = self.mux.focused() else {
            return;
        };
        if focused.kind() != &PaneKind::Worker {
            return;
        }
        if let Some(idx) = self
            .worker_names
            .iter()
            .position(|name| name == focused.id())
        {
            self.selected_worker_tab = idx;
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        use crate::ui::factory::renderer::FactoryViewMode;
        match self.factory_view_mode {
            FactoryViewMode::Panes => self.render_panes_view(frame),
            FactoryViewMode::MissionControl => self.render_mission_control_view(frame),
        }
    }

    /// Render the standard Panes view (workers + supervisor + sidecar).
    fn render_panes_view(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.sync_selected_worker_tab_with_focus();

        // Calculate layout using all worker names (real + pending)
        let all_names = self.layout_worker_names();
        let layout = FactoryLayout::calculate_from_names_with_header_rows(
            area,
            &all_names,
            self.tabbed_workers,
            self.sidecar_collapsed,
            self.layout_sizes,
            0,
        );

        // Store layout areas for click detection
        self.worker_tab_bar_area = layout.worker_tab_bar;
        self.worker_content_area = layout.worker_content;
        self.worker_areas = layout.worker_areas.clone();
        self.supervisor_area = Some(layout.supervisor_area);
        self.sidecar_area = Some(layout.sidecar_area);

        // Update pane grid if tabbed mode changed
        if self.is_tabbed != layout.is_tabbed {
            self.is_tabbed = layout.is_tabbed;
            self.pane_grid =
                PaneGrid::new(&self.worker_names, &self.supervisor_name, layout.is_tabbed);
        }

        // Render worker panes (stacked vertically)
        self.render_workers(frame, &layout);

        // Render supervisor pane
        self.render_supervisor(frame, &layout);

        // Render sidecar panels (Tasks, Agents, Changes, Activity)
        self.render_sidecar(frame, &layout);

        // Render status bar
        StatusBar::render(frame, layout.status_bar, self);
        self.render_error_banner(frame, layout.status_bar);

        self.render_overlays(frame);
    }

    /// Render the Mission Control dashboard view.
    ///
    /// Shows status strip, WORKERS panel and live TASKS, CHANGES, ACTIVITY columns.
    /// PTY panes keep running in the background (not rendered).
    fn render_mission_control_view(&mut self, frame: &mut Frame) {
        use crate::ui::factory::director::{mission_epic, mission_workers};
        use crate::ui::factory::layout::MissionControlLayout;
        use crate::ui::factory::renderer::MissionControlFocus;

        let area = frame.area();

        let worker_count = self.layout_worker_names().len();
        let mc = MissionControlLayout::calculate(area, worker_count);
        let focus = self.mc_focus;

        // Store MC panel areas for click detection
        self.mc_workers_area = mc.workers_area;
        self.mc_tasks_area = mc.tasks_area;
        self.mc_changes_area = mc.changes_area;
        self.mc_activity_area = mc.activity_area;

        // Status strip — compact epic progress + task counts + worker chips
        mission_epic::render_status_strip(frame, mc.status_strip, &self.director_data, &self.theme);

        // WORKERS panel — live content from DirectorData (focused border when selected)
        mission_workers::render_workers_panel_with_focus(
            frame,
            mc.workers_area,
            &self.director_data,
            &self.theme,
            focus == MissionControlFocus::Workers,
            if focus == MissionControlFocus::Workers {
                self.panels.factory.list_state.selected()
            } else {
                None
            },
        );

        // TASKS column — reuse the director tasks renderer
        crate::ui::factory::director::tasks::render_with_focus(
            frame,
            mc.tasks_area,
            &self.director_data,
            &self.theme,
            focus == MissionControlFocus::Tasks,
            None,
            None,
            false,
            &self.collapsed_epics,
            Some(&mut self.panels.tasks.list_state),
        );

        // CHANGES column — reuse the director changes renderer
        self.changes_item_types = crate::ui::factory::director::changes::render_with_focus(
            frame,
            mc.changes_area,
            &self.director_data,
            &self.theme,
            focus == MissionControlFocus::Changes,
            None,
            false,
            Some(&mut self.panels.changes.list_state),
            &self.collapsed_dirs,
        );

        // ACTIVITY column — reuse the director activity renderer
        crate::ui::factory::director::activity::render_with_focus(
            frame,
            mc.activity_area,
            &self.director_data,
            &self.theme,
            focus == MissionControlFocus::Activity,
            None,
            false,
        );

        // Status bar
        StatusBar::render(frame, mc.status_bar, self);
        self.render_error_banner(frame, mc.status_bar);

        self.render_overlays(frame);
    }

    /// Render modal overlays (inject, help, dialogs).
    fn render_overlays(&mut self, frame: &mut Frame) {
        // Render inject dialog if in inject mode
        if matches!(self.input_mode, InputMode::Inject) {
            self.render_inject_dialog(frame);
        }

        // Render file changes dialog if visible
        if self.show_changes_dialog {
            self.render_changes_dialog(frame);
        }

        // Render task detail dialog if visible
        if self.show_task_dialog {
            self.render_task_dialog(frame);
        }

        // Render reminder detail dialog if visible
        if self.show_reminder_dialog {
            self.render_reminder_dialog(frame);
        }

        // Render terminal dialog if visible
        if self.show_terminal_dialog {
            self.render_terminal_dialog(frame);
        }

        // Render feedback dialog if visible
        if self.show_feedback_dialog {
            self.render_feedback_dialog(frame);
        }

        // Render help overlay if visible
        if self.show_help {
            self.render_help_overlay(frame);
        }
    }

    /// Render compact view for phone/narrow terminals.
    ///
    /// Layout: 1-line status bar at top, supervisor pane filling the rest.
    /// The supervisor pane is rendered without borders to maximize content area,
    /// since Claude Code has its own input UI.
    pub fn render_compact(&mut self, frame: &mut Frame) {
        use ratatui::layout::{Constraint, Layout};
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::Paragraph;

        let area = frame.area();

        // Split: 1-row status bar + rest for supervisor
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3)])
            .split(area);

        let status_area = chunks[0];
        let supervisor_area = chunks[1];

        // === Compact status bar ===
        let palette = &self.theme().palette;
        let styles = &self.theme().styles;
        let mut spans = Vec::new();

        // CAS badge
        spans.push(Span::styled(
            " CAS ",
            Style::default()
                .fg(palette.text_primary)
                .bg(palette.accent_dim)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));

        // Epic progress
        match self.epic_state() {
            EpicState::Idle => {
                spans.push(Span::styled("no epic", styles.text_muted));
            }
            EpicState::Active { epic_id, .. } | EpicState::Completing { epic_id, .. } => {
                // Get task counts from director data
                let data = self.director_data();
                let total =
                    data.epic_tasks.len() + data.ready_tasks.len() + data.in_progress_tasks.len();
                let done = data
                    .epic_tasks
                    .iter()
                    .filter(|t| t.status == cas_types::TaskStatus::Closed)
                    .count();

                let is_completing = matches!(self.epic_state(), EpicState::Completing { .. });

                spans.push(Span::styled(
                    epic_id.to_string(),
                    if is_completing {
                        styles.text_warning.add_modifier(Modifier::BOLD)
                    } else {
                        styles.text_success.add_modifier(Modifier::BOLD)
                    },
                ));
                spans.push(Span::raw(" "));

                // Progress bar
                let bar_width = 8usize;
                let filled = if total > 0 {
                    (done * bar_width) / total
                } else {
                    0
                };
                let empty = bar_width - filled;
                spans.push(Span::styled("\u{2588}".repeat(filled), styles.text_success));
                spans.push(Span::styled("\u{2591}".repeat(empty), styles.text_muted));
                spans.push(Span::styled(format!(" {done}/{total}"), styles.text_muted));
            }
        }

        // Worker status dots (right-aligned)
        let mut worker_spans = Vec::new();
        for name in self.worker_names() {
            let is_active = self
                .director_data()
                .agents
                .iter()
                .any(|a| a.name == *name && matches!(a.status, cas_types::AgentStatus::Active));

            let dot = if is_active { "\u{25cf}" } else { "\u{25cb}" };
            let short_name: String = name.chars().take(3).collect();
            worker_spans.push(Span::styled(
                format!("{dot}{short_name} "),
                if is_active {
                    styles.text_success
                } else {
                    styles.text_muted
                },
            ));
        }

        // Calculate padding between left and right
        let left_width: usize = spans.iter().map(|s| s.content.len()).sum();
        let right_width: usize = worker_spans.iter().map(|s| s.content.len()).sum();
        let padding = (status_area.width as usize).saturating_sub(left_width + right_width);

        spans.push(Span::raw(" ".repeat(padding)));
        spans.extend(worker_spans);

        let status_line = Paragraph::new(Line::from(spans)).style(styles.bg_elevated);
        frame.render_widget(status_line, status_area);

        // === Supervisor pane (no borders, full area) ===
        if let Some(pane) = self.mux.get(&self.supervisor_name) {
            let lines: Vec<Line> = (0..supervisor_area.height)
                .map(|row| pane.row_as_line(row).unwrap_or_default())
                .collect();

            let content = Paragraph::new(lines);
            frame.render_widget(content, supervisor_area);
        }
    }

    fn render_workers(&self, frame: &mut Frame, layout: &FactoryLayout) {
        let all_names = self.layout_worker_names();
        if layout.is_tabbed {
            // Tabbed mode: render tab bar + selected worker content
            if let Some(tab_bar_area) = layout.worker_tab_bar {
                self.render_worker_tab_bar(frame, tab_bar_area);
            }

            if let Some(content_area) = layout.worker_content {
                if let Some(name) = all_names.get(self.selected_worker_tab) {
                    if self.is_pending_worker(name) {
                        self.render_booting_pane(frame, content_area, name);
                    } else {
                        self.render_single_worker(frame, content_area, name);
                    }
                }
            }
        } else {
            // Side-by-side mode: render all workers in their own areas
            for (i, name) in all_names.iter().enumerate() {
                if let Some(worker_area) = layout.worker_areas.get(i) {
                    if self.is_pending_worker(name) {
                        self.render_booting_pane(frame, *worker_area, name);
                    } else {
                        self.render_single_worker(frame, *worker_area, name);
                    }
                }
            }
        }
    }

    /// Render a single worker pane
    fn render_single_worker(&self, frame: &mut Frame, area: Rect, name: &str) {
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

        if let Some(pane) = self.mux.get(name) {
            let palette = &self.theme().palette;
            let _agent_color = get_agent_color(name);
            let is_pane_select = self.input_mode.is_pane_select();
            let is_focused = pane.is_focused();

            // Get status indicator
            let status_icon = self.get_worker_status_icon(name);
            let title = format!(" {name}{status_icon} [worker] ");

            // Determine border style based on mode
            let (border_color, border_type) = if is_pane_select {
                if is_focused {
                    // Focused pane in PaneSelect: bright highlight with thick border
                    (palette.accent, BorderType::Thick)
                } else {
                    // Other panes in PaneSelect: visible but not focused
                    (palette.accent_dim, BorderType::Rounded)
                }
            } else {
                // Normal mode: muted border
                (palette.border_muted, BorderType::Rounded)
            };

            let block = Block::default()
                .title(title)
                .title_style(
                    Style::default()
                        .fg(border_color)
                        .add_modifier(Modifier::BOLD),
                )
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(border_color));

            // Get terminal content with selection highlighting
            let inner = block.inner(area);
            let has_selection = self.selection.pane_name == name && !self.selection.is_empty();
            let scroll_delta = if has_selection {
                pane.scroll_offset() as i32 - self.selection.scroll_offset as i32
            } else {
                0
            };
            let lines: Vec<Line> = (0..inner.height)
                .map(|row| {
                    let line = pane.row_as_line(row).unwrap_or_default();
                    if has_selection {
                        crate::ui::factory::selection::apply_selection_to_line(
                            line,
                            row,
                            &self.selection,
                            scroll_delta,
                        )
                    } else {
                        line
                    }
                })
                .collect();

            let content = Paragraph::new(lines).block(block);
            frame.render_widget(content, area);

            // Show new-lines indicator when user has scrolled up
            let new_below = pane.new_lines_below();
            if pane.is_user_scrolled() && new_below > 0 {
                let label = format!(" ↓ {} new lines ", new_below);
                let label_width = label.len() as u16;
                let indicator_area = Rect {
                    x: inner.x + inner.width.saturating_sub(label_width),
                    y: inner.y + inner.height.saturating_sub(1),
                    width: label_width.min(inner.width),
                    height: 1,
                };
                let indicator = Paragraph::new(Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(ratatui::style::Color::Black)
                        .bg(ratatui::style::Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                frame.render_widget(indicator, indicator_area);
            }
        }
    }

    /// Render the worker tab bar
    fn render_worker_tab_bar(&self, frame: &mut Frame, area: Rect) {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::Paragraph;

        let palette = &self.theme().palette;
        let all_names = self.layout_worker_names();

        // Braille spinner frames for pending workers
        const SPINNER_FRAMES: &[char] = &[
            '\u{2801}', '\u{2809}', '\u{2819}', '\u{2838}', '\u{283C}', '\u{2834}', '\u{2826}',
            '\u{2827}', '\u{2807}', '\u{280F}',
        ];

        // Tab bar background: darkest layer (bg_primary)
        let tab_bar_bg = palette.bg_primary;
        // Active tab: elevated surface to "pop" above the bar
        let active_tab_bg = palette.bg_elevated;
        // Inactive tab text: subdued
        let inactive_fg = palette.text_muted;
        // Border/separator color
        let border_color = palette.border_default;

        // -- Build the content line (middle row) --
        let mut content_spans: Vec<Span> = Vec::new();
        content_spans.push(Span::styled(" ", Style::default().bg(tab_bar_bg)));

        for (i, name) in all_names.iter().enumerate() {
            let is_selected = i == self.selected_worker_tab;
            let number = i + 1;
            let is_pending = self.is_pending_worker(name);

            let agent_color = if is_pending {
                Color::Rgb(255, 200, 80)
            } else {
                get_agent_color(name)
            };

            // Get status indicator from CAS data or spinner for pending
            let status_icon: String = if is_pending {
                let idx = (self
                    .pending_workers
                    .iter()
                    .find(|pw| pw.name == *name)
                    .map(|pw| pw.started_at.elapsed().as_millis() / 100)
                    .unwrap_or(0) as usize)
                    % SPINNER_FRAMES.len();
                format!(" {}", SPINNER_FRAMES[idx])
            } else {
                self.get_worker_status_icon(name).to_string()
            };

            let label = format!(" {number} {name}{status_icon} ");

            if is_selected {
                // Active tab: elevated bg, agent-colored text, bold
                content_spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(agent_color)
                        .bg(active_tab_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                // Inactive tab: no bg (inherits bar bg), dimmed text
                content_spans.push(Span::styled(
                    label,
                    Style::default().fg(inactive_fg).bg(tab_bar_bg),
                ));
            }

            // Thin separator between tabs
            if i < all_names.len() - 1 {
                content_spans.push(Span::styled(
                    " ",
                    Style::default().fg(border_color).bg(tab_bar_bg),
                ));
            }
        }

        // Fill remaining width with bar background
        let used: usize = content_spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum();
        let remaining = (area.width as usize).saturating_sub(used);
        if remaining > 0 {
            content_spans.push(Span::styled(
                " ".repeat(remaining),
                Style::default().bg(tab_bar_bg),
            ));
        }

        let content_line = Line::from(content_spans);

        let lines = if area.height >= 3 {
            // Top row: blank with bar bg
            let top_line = Line::from(Span::styled(
                " ".repeat(area.width as usize),
                Style::default().bg(tab_bar_bg),
            ));

            // Bottom row: border line with accent under active tab
            let mut bottom_spans: Vec<Span> = Vec::new();
            bottom_spans.push(Span::styled(
                "─",
                Style::default().fg(border_color).bg(tab_bar_bg),
            ));

            for (i, name) in all_names.iter().enumerate() {
                let is_selected = i == self.selected_worker_tab;
                let is_pending = self.is_pending_worker(name);

                let agent_color = if is_pending {
                    Color::Rgb(255, 200, 80)
                } else {
                    get_agent_color(name)
                };

                let status_icon: String = if is_pending {
                    let idx = (self
                        .pending_workers
                        .iter()
                        .find(|pw| pw.name == *name)
                        .map(|pw| pw.started_at.elapsed().as_millis() / 100)
                        .unwrap_or(0) as usize)
                        % SPINNER_FRAMES.len();
                    format!(" {}", SPINNER_FRAMES[idx])
                } else {
                    self.get_worker_status_icon(name).to_string()
                };
                let tab_label = format!(" {} {}{} ", i + 1, name, status_icon);
                let tab_width = tab_label.chars().count();

                if is_selected {
                    // Active tab gets a colored underline accent
                    bottom_spans.push(Span::styled(
                        "▀".repeat(tab_width),
                        Style::default().fg(agent_color).bg(tab_bar_bg),
                    ));
                } else {
                    // Inactive tabs get a thin border line
                    bottom_spans.push(Span::styled(
                        "─".repeat(tab_width),
                        Style::default().fg(border_color).bg(tab_bar_bg),
                    ));
                }

                // Separator width
                if i < all_names.len() - 1 {
                    bottom_spans.push(Span::styled(
                        "─",
                        Style::default().fg(border_color).bg(tab_bar_bg),
                    ));
                }
            }

            // Fill remaining bottom with border
            let bottom_used: usize = bottom_spans.iter().map(|s| s.content.chars().count()).sum();
            let bottom_remaining = (area.width as usize).saturating_sub(bottom_used);
            if bottom_remaining > 0 {
                bottom_spans.push(Span::styled(
                    "─".repeat(bottom_remaining),
                    Style::default().fg(border_color).bg(tab_bar_bg),
                ));
            }

            let bottom_line = Line::from(bottom_spans);
            vec![top_line, content_line, bottom_line]
        } else {
            vec![content_line]
        };

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, area);
    }

    /// Get a status icon for a worker based on their CAS task state
    fn get_worker_status_icon(&self, worker_name: &str) -> &'static str {
        // Check if worker has an in-progress task
        for task in &self.director_data.in_progress_tasks {
            if let Some(assignee) = &task.assignee {
                if assignee == worker_name {
                    return " ●"; // Working indicator
                }
            }
        }
        "" // No status indicator
    }

    /// Render a booting pane placeholder for a pending worker
    fn render_booting_pane(&self, frame: &mut Frame, area: Rect, name: &str) {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

        let pending = self.pending_workers.iter().find(|pw| pw.name == name);
        let elapsed = pending
            .map(|pw| pw.started_at.elapsed())
            .unwrap_or_default();
        let is_isolate = pending.map(|pw| pw.isolate).unwrap_or(false);
        let elapsed_ms = elapsed.as_millis() as usize;

        // Colors matching boot screen
        let purple = Color::Rgb(180, 130, 255);
        let green = Color::Rgb(80, 250, 120);
        let orange = Color::Rgb(255, 200, 80);
        let gray = Color::Rgb(120, 120, 130);

        // Animated braille spinner for title
        const SPINNER: &[char] = &[
            '\u{280B}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283C}', '\u{2834}', '\u{2826}',
            '\u{2827}', '\u{2807}', '\u{280F}',
        ];
        let spinner_idx = (elapsed_ms / 80) % SPINNER.len();
        let spinner_char = SPINNER[spinner_idx];

        let title = format!(" {spinner_char} {name} [booting] ");

        let block = Block::default()
            .title(title)
            .title_style(Style::default().fg(purple).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(purple));

        let inner = block.inner(area);
        let mut lines: Vec<Line> = Vec::new();

        // Boot steps with status indicators
        let steps: Vec<(&str, u64)> = if is_isolate {
            vec![
                ("Resolving name", 0),
                ("Creating worktree", 500),
                ("Initializing branch", 2000),
                ("Spawning agent", 4000),
            ]
        } else {
            vec![("Resolving name", 0), ("Spawning agent", 500)]
        };

        // Blank line at top
        lines.push(Line::default());

        let elapsed_secs_ms = elapsed.as_millis() as u64;
        for (i, (step_name, start_ms)) in steps.iter().enumerate() {
            let is_last = i == steps.len() - 1;
            let next_start = steps.get(i + 1).map(|(_, ms)| *ms).unwrap_or(u64::MAX);

            let (icon, icon_color) = if elapsed_secs_ms >= next_start {
                // Completed
                ("\u{2714}", green) // checkmark
            } else if elapsed_secs_ms >= *start_ms {
                // In progress - animated spinner
                let step_spinner_idx = ((elapsed_secs_ms - start_ms) / 80) as usize % SPINNER.len();
                let c = SPINNER[step_spinner_idx];
                // We need to convert char to &str; use a match on a few cases
                let s: &str = match c {
                    '\u{280B}' => "\u{280B}",
                    '\u{2819}' => "\u{2819}",
                    '\u{2839}' => "\u{2839}",
                    '\u{2838}' => "\u{2838}",
                    '\u{283C}' => "\u{283C}",
                    '\u{2834}' => "\u{2834}",
                    '\u{2826}' => "\u{2826}",
                    '\u{2827}' => "\u{2827}",
                    '\u{2807}' => "\u{2807}",
                    '\u{280F}' => "\u{280F}",
                    _ => "\u{280B}",
                };
                (s, orange)
            } else {
                // Not started
                ("\u{25CB}", gray) // circle
            };

            let text_color = if elapsed_secs_ms >= *start_ms {
                if elapsed_secs_ms >= next_start {
                    green
                } else {
                    Color::Rgb(220, 220, 230)
                }
            } else {
                gray
            };

            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(icon.to_string(), Style::default().fg(icon_color)),
                Span::raw(" "),
                Span::styled(step_name.to_string(), Style::default().fg(text_color)),
            ]));

            // Add blank line between steps (if space allows)
            if !is_last && inner.height > 8 {
                lines.push(Line::default());
            }
        }

        // Progress bar
        lines.push(Line::default());
        let bar_width = (inner.width as usize).saturating_sub(4).min(30);
        if bar_width > 0 {
            // Progress based on elapsed time (cap at ~6 seconds for visual)
            let progress = (elapsed_secs_ms as f64 / 6000.0).min(0.95);
            let filled = (progress * bar_width as f64) as usize;
            let empty = bar_width - filled;

            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("\u{2588}".repeat(filled), Style::default().fg(purple)),
                Span::styled("\u{2591}".repeat(empty), Style::default().fg(gray)),
            ]));
        }

        // Pad remaining lines
        while lines.len() < inner.height as usize {
            lines.push(Line::default());
        }
        lines.truncate(inner.height as usize);

        let content = Paragraph::new(lines).block(block);
        frame.render_widget(content, area);
    }

    fn render_supervisor(&self, frame: &mut Frame, layout: &FactoryLayout) {
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

        if let Some(pane) = self.mux.get(&self.supervisor_name) {
            let palette = &self.theme().palette;
            // Supervisor is only truly focused if mux says so AND sidecar is not focused
            let is_focused = pane.is_focused() && !self.sidecar_is_focused();
            let _agent_color = get_agent_color(&self.supervisor_name);
            let is_pane_select = self.input_mode.is_pane_select();

            // Determine border style based on mode
            let (border_color, border_type) = if is_pane_select {
                if is_focused {
                    // Focused pane in PaneSelect: bright highlight with thick border
                    (palette.accent, BorderType::Thick)
                } else {
                    // Other panes in PaneSelect: visible but not focused
                    (palette.accent_dim, BorderType::Rounded)
                }
            } else if is_focused {
                // Normal mode, focused: use theme border color
                (palette.border_focused, BorderType::Rounded)
            } else {
                // Normal mode, not focused: dimmed
                (palette.border_muted, BorderType::Rounded)
            };

            let title = format!(" {} [supervisor] ", self.supervisor_name);
            let block = Block::default()
                .title(title)
                .title_style(
                    Style::default()
                        .fg(border_color)
                        .add_modifier(Modifier::BOLD),
                )
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(border_color));

            // Get terminal content with selection highlighting
            let inner = block.inner(layout.supervisor_area);
            let has_selection =
                self.selection.pane_name == self.supervisor_name && !self.selection.is_empty();
            let scroll_delta = if has_selection {
                pane.scroll_offset() as i32 - self.selection.scroll_offset as i32
            } else {
                0
            };
            let lines: Vec<Line> = (0..inner.height)
                .map(|row| {
                    let line = pane.row_as_line(row).unwrap_or_default();
                    if has_selection {
                        crate::ui::factory::selection::apply_selection_to_line(
                            line,
                            row,
                            &self.selection,
                            scroll_delta,
                        )
                    } else {
                        line
                    }
                })
                .collect();

            let content = Paragraph::new(lines).block(block);
            frame.render_widget(content, layout.supervisor_area);

            // Show new-lines indicator when user has scrolled up
            let new_below = pane.new_lines_below();
            if pane.is_user_scrolled() && new_below > 0 {
                let label = format!(" ↓ {} new lines ", new_below);
                let label_width = label.len() as u16;
                let indicator_area = Rect {
                    x: inner.x + inner.width.saturating_sub(label_width),
                    y: inner.y + inner.height.saturating_sub(1),
                    width: label_width.min(inner.width),
                    height: 1,
                };
                let indicator = Paragraph::new(Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(ratatui::style::Color::Black)
                        .bg(ratatui::style::Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                frame.render_widget(indicator, indicator_area);
            }
        }
    }

    fn render_sidecar(&mut self, frame: &mut Frame, layout: &FactoryLayout) {
        match &self.view_mode {
            ViewMode::Overview => {
                let mut state = SidecarState {
                    focus: self.sidecar_focus,
                    tasks_state: &mut self.panels.tasks.list_state,
                    agents_state: &mut self.panels.factory.list_state,
                    reminders_state: &mut self.panels.reminders.list_state,
                    changes_state: &mut self.panels.changes.list_state,
                    activity_state: &mut self.panels.activity.list_state,
                    agent_filter: self.agent_filter.as_deref(),
                    factory_collapsed: self.panels.factory.collapsed,
                    tasks_collapsed: self.panels.tasks.collapsed,
                    reminders_collapsed: self.panels.reminders.collapsed,
                    changes_collapsed: self.panels.changes.collapsed,
                    activity_collapsed: self.panels.activity.collapsed,
                    collapsed_epics: &self.collapsed_epics,
                    collapsed_dirs: &self.collapsed_dirs,
                    changes_item_types: &mut self.changes_item_types,
                };
                let areas = render_with_state(
                    frame,
                    layout.sidecar_area,
                    &self.director_data,
                    &self.theme,
                    &self.supervisor_name,
                    Some(&mut state),
                );
                // Store panel areas for click detection
                self.panel_areas = areas;
            }
            ViewMode::TaskDetail(task_id) => {
                let task_id = task_id.clone();
                self.render_task_detail(frame, layout.sidecar_area, &task_id);
            }
            ViewMode::ActivityLog => {
                self.render_activity_log(frame, layout.sidecar_area);
            }
            ViewMode::FileDiff(_, file_path) => {
                let file_path = file_path.clone();
                self.render_file_diff(frame, layout.sidecar_area, &file_path);
            }
        }
    }
}
