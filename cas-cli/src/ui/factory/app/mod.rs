//! Factory application state and orchestration

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use cas_mux::{Mux, PaneKind};
use ratatui::layout::Rect;

use super::director::{
    DiffLine, DirectorData, DirectorEvent, DirectorEventDetector, PanelAreas, Prompt, SidecarFocus,
    ViewMode, generate_prompt,
};
use crate::store::open_prompt_queue_store;
use crate::types::Worktree;
use crate::ui::factory::input::{InputMode, LayoutSizes};
use crate::ui::factory::layout::{FactoryLayout, PaneGrid};
use crate::ui::factory::notification::Notifier;
use crate::ui::theme::ActiveTheme;
use crate::ui::widgets::TreeItemType;
use crate::worktree::WorktreeManager;

mod imports;
mod init;
mod panels_and_modes;
mod render_and_ops;
mod sidecar_and_selection;

// Re-export from cas-factory for backward compatibility
pub use cas_factory::{AutoPromptConfig, EpicState, FactoryConfig};

/// Booting state for a worker that is being spawned (after prepare, before finish)
#[derive(Debug, Clone)]
pub struct PendingWorkerState {
    /// Worker name
    pub name: String,
    /// When this worker entered the pending state
    pub started_at: Instant,
    /// Whether this spawn is using worktree isolation
    pub isolate: bool,
}

/// Worktree preparation data (can be sent to background thread)
pub struct WorktreePrep {
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub parent_branch: String,
    pub repo_root: PathBuf,
    pub cas_dir: PathBuf,
}

/// Data needed to spawn a worker (phase 1 output, can be sent to background thread)
pub struct WorkerSpawnPrep {
    pub worker_name: String,
    pub worktree_info: Option<WorktreePrep>,
}

/// Result of background worktree preparation (phase 2 output)
pub struct WorkerSpawnResult {
    pub worker_name: String,
    pub cwd: PathBuf,
    pub cas_root: Option<PathBuf>,
    pub worktree: Option<Worktree>,
}

impl WorkerSpawnPrep {
    /// Phase 2: Run the slow git operations (designed for spawn_blocking).
    pub fn run(self) -> anyhow::Result<WorkerSpawnResult> {
        if let Some(wt) = self.worktree_info {
            use crate::worktree::GitOperations;

            let git = GitOperations::new(wt.repo_root.clone());

            // Check if worktree already exists on disk (reuse from previous session)
            if wt.worktree_path.exists() {
                let _ = git.init_submodules(&wt.worktree_path);
                let worktree = Worktree::new(
                    Worktree::generate_id(),
                    wt.branch_name,
                    wt.parent_branch,
                    wt.worktree_path.clone(),
                );
                return Ok(WorkerSpawnResult {
                    worker_name: self.worker_name,
                    cwd: wt.worktree_path,
                    cas_root: Some(wt.cas_dir),
                    worktree: Some(worktree),
                });
            }

            // Create parent directory
            if let Some(parent) = wt.worktree_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Create git worktree (THE SLOW PART)
            git.create_worktree(&wt.worktree_path, &wt.branch_name, Some(&wt.parent_branch))?;

            let worktree = Worktree::new(
                Worktree::generate_id(),
                wt.branch_name,
                wt.parent_branch,
                wt.worktree_path.clone(),
            );

            Ok(WorkerSpawnResult {
                worker_name: self.worker_name,
                cwd: wt.worktree_path,
                cas_root: Some(wt.cas_dir),
                worktree: Some(worktree),
            })
        } else {
            let cwd = std::env::current_dir()?;
            Ok(WorkerSpawnResult {
                worker_name: self.worker_name,
                cwd,
                cas_root: None,
                worktree: None,
            })
        }
    }
}

/// The main factory application
pub struct FactoryApp {
    /// The terminal multiplexer
    pub mux: Mux,
    /// CAS directory for data loading
    cas_dir: PathBuf,
    /// Director panel data
    director_data: DirectorData,
    /// Current input mode
    pub input_mode: InputMode,
    /// Buffer for inject mode text input
    pub inject_buffer: String,
    /// Target pane for injection
    pub inject_target: Option<String>,
    /// Show help overlay
    pub show_help: bool,
    /// Show file changes dialog
    pub show_changes_dialog: bool,
    /// Selected file for changes dialog (source_path, file_path, source_name, agent_name)
    pub changes_dialog_file: Option<(PathBuf, String, String, Option<String>)>,
    /// Scroll offset for changes dialog diff
    pub changes_dialog_scroll: u16,
    /// Cached diff lines for changes dialog
    pub changes_dialog_diff: Vec<DiffLine>,
    /// Show task detail dialog
    pub show_task_dialog: bool,
    /// Selected task ID for task dialog
    pub task_dialog_id: Option<String>,
    /// Scroll offset for task dialog
    pub task_dialog_scroll: u16,
    /// Max scroll offset for task dialog (computed during render)
    pub task_dialog_max_scroll: u16,
    /// Show reminder detail dialog
    pub show_reminder_dialog: bool,
    /// Selected reminder index for reminder dialog
    pub reminder_dialog_idx: Option<usize>,
    /// Scroll offset for reminder dialog
    pub reminder_dialog_scroll: u16,
    /// Show terminal dialog (interactive shell)
    pub show_terminal_dialog: bool,
    /// Name of the shell pane in the mux
    pub terminal_pane_name: Option<String>,
    /// Show feedback dialog
    pub show_feedback_dialog: bool,
    /// Current feedback category
    pub feedback_category: super::input::FeedbackCategory,
    /// Feedback text buffer
    pub feedback_buffer: String,
    /// Last CAS data refresh time
    last_refresh: Instant,
    /// Refresh interval for CAS data
    refresh_interval: Duration,
    /// Last observed CAS DB file fingerprint used for cheap change detection
    last_db_fingerprint: Option<CasDbFingerprint>,
    /// Last git refresh time
    last_git_refresh: Instant,
    /// Interval for expensive git refresh operations
    git_refresh_interval: Duration,
    /// Theme for rendering
    theme: ActiveTheme,
    /// Worker names (for reference)
    worker_names: Vec<String>,
    /// Supervisor name (for reference)
    supervisor_name: String,
    /// Factory session name (for prompt queue isolation)
    factory_session: Option<String>,
    /// Supervisor CLI mode (claude/codex)
    supervisor_cli: cas_mux::SupervisorCli,
    /// Worker CLI mode (claude/codex)
    worker_cli: cas_mux::SupervisorCli,
    /// Error message to display (cleared on next key or after timeout)
    pub error_message: Option<String>,
    /// Number of workers currently being spawned (for loading indicator)
    pub spawning_count: usize,
    /// Workers currently in the spawning pipeline (after prepare, before finish).
    /// These appear as booting placeholder panes in the layout.
    pub pending_workers: Vec<PendingWorkerState>,
    /// When the error message was set (for auto-dismiss)
    error_set_at: Option<Instant>,
    /// Sidebar collapsed state
    pub sidecar_collapsed: bool,
    /// Worktree manager for worker isolation (None if worktrees disabled)
    worktree_manager: Option<WorktreeManager>,
    /// Index of the currently selected worker tab (0-based, used in tabbed mode)
    pub selected_worker_tab: usize,
    /// Use tabbed worker view instead of side-by-side (config preference)
    tabbed_workers: bool,
    /// Actual tabbed mode active (accounts for auto-switch due to space constraints)
    is_tabbed: bool,
    /// Custom layout percentages (None = use defaults)
    pub layout_sizes: Option<LayoutSizes>,
    /// Spatial grid for pane navigation (rebuilt on layout change)
    pane_grid: PaneGrid,
    /// Currently selected pane in pane select mode
    selected_pane: Option<String>,
    /// Event detector for CAS state changes
    event_detector: DirectorEventDetector,
    /// Notification manager
    notifier: Notifier,
    /// Current epic state
    epic_state: EpicState,
    /// Explicit current epic ID — set when supervisor creates/starts an epic.
    /// Takes priority over passive scanning in detect_epic_state().
    current_epic_id: Option<String>,
    /// Sidecar panel focus
    pub sidecar_focus: SidecarFocus,
    /// Sidecar panel scroll/collapse state
    pub panels: super::director::PanelRegistry,
    /// Panel areas for click detection (updated during render)
    panel_areas: PanelAreas,
    /// Current view mode for the sidecar
    pub view_mode: ViewMode,
    /// Scroll offset for detail views
    detail_scroll: u16,
    /// Agent filter (None = show all)
    pub agent_filter: Option<String>,
    /// Cached diff content for FileDiff view (used by search)
    diff_cache: Vec<DiffLine>,
    /// Scroll offset for diff view (legacy, used by search jump)
    diff_scroll: u16,
    /// Parsed diff metadata for DiffWidget rendering
    diff_metadata: Option<cas_diffs::FileDiffMetadata>,
    /// Syntax highlighter for diff rendering
    syntax_highlighter: cas_diffs::highlight::SyntaxHighlighter,
    /// Scroll/hunk navigation state for DiffWidget
    diff_view_state: cas_diffs::widget::DiffViewState,
    /// Diff display style (unified vs split)
    diff_display_style: cas_diffs::iter::DiffStyle,
    /// Inline diff highlighting mode
    diff_inline_mode: cas_diffs::LineDiffType,
    /// Whether to show line numbers in diff view
    diff_show_line_numbers: bool,
    /// Expanded hunk regions (for expanding collapsed context)
    diff_expanded_hunks: std::collections::HashMap<usize, cas_diffs::iter::HunkExpansionRegion>,
    /// Whether all collapsed regions are expanded
    diff_expand_all: bool,
    /// Whether diff search input mode is active
    diff_search_mode: bool,
    /// Current diff search query
    diff_search_query: String,
    /// Line indices that match the search query
    diff_search_matches: Vec<usize>,
    /// Current match index (for n/N navigation)
    diff_search_current: usize,
    /// Collapsed epic IDs (epics whose subtasks are hidden)
    collapsed_epics: HashSet<String>,
    /// Collapsed directory paths in changes panel
    collapsed_dirs: HashSet<String>,
    /// Tree item types for changes panel (for scroll bounds)
    changes_item_types: Vec<TreeItemType>,
    /// Layout areas for click detection (updated during render)
    worker_tab_bar_area: Option<Rect>,
    worker_content_area: Option<Rect>,
    worker_areas: Vec<Rect>,
    supervisor_area: Option<Rect>,
    sidecar_area: Option<Rect>,
    /// Stored terminal dimensions (for daemon mode where crossterm::terminal::size() doesn't work)
    terminal_cols: u16,
    terminal_rows: u16,
    /// Current text selection (for copy support)
    selection: super::selection::Selection,
    /// Auto-prompting configuration
    auto_prompt: AutoPromptConfig,
    /// Epic branch name (e.g., "epic/add-user-auth") - workers branch from this
    epic_branch: Option<String>,
    /// Whether recording is enabled for this session
    record_enabled: bool,
    /// Session ID for recordings (only set if record_enabled)
    recording_session_id: Option<String>,
    /// When recording started (for computing event timestamps)
    recording_start: Option<Instant>,
    /// Collected events during this session (for export)
    recorded_events: Vec<(Instant, DirectorEvent)>,
    /// UUID for the team lead's Claude Code session (for Teams config.json)
    lead_session_id: Option<String>,
    /// Project directory (for git operations)
    project_dir: PathBuf,
    /// Session ID to pane name mapping for interaction routing
    session_to_pane: HashMap<String, String>,
    /// Last time Ctrl+C was sent to a pane (debounce rapid repeated presses)
    pub last_interrupt_time: Option<Instant>,
    /// Top-level view mode (Panes vs Mission Control)
    pub factory_view_mode: crate::ui::factory::renderer::FactoryViewMode,
    /// Which panel has focus in Mission Control view
    pub mc_focus: crate::ui::factory::renderer::MissionControlFocus,
    /// Mission Control panel areas for click detection (updated during render)
    mc_workers_area: Rect,
    mc_tasks_area: Rect,
    mc_changes_area: Rect,
    mc_activity_area: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CasDbFingerprint {
    db_mtime: Option<SystemTime>,
    wal_mtime: Option<SystemTime>,
}

impl CasDbFingerprint {
    fn from_cas_dir(cas_dir: &std::path::Path) -> Self {
        let db_path = cas_dir.join("cas.db");
        let wal_path = cas_dir.join("cas.db-wal");

        Self {
            db_mtime: file_mtime(&db_path),
            wal_mtime: file_mtime(&wal_path),
        }
    }
}

fn file_mtime(path: &std::path::Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

/// Convert a title to a branch-safe slug
fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(50)
        .collect()
}

/// Create the epic branch name from a title
pub(crate) fn epic_branch_name(title: &str) -> String {
    format!("epic/{}", slugify(title))
}

impl FactoryApp {
    fn filter_director_agents_to_current_session(&mut self) {
        let mut allowed = std::collections::HashSet::with_capacity(self.worker_names.len() + 1);
        for name in &self.worker_names {
            allowed.insert(name.clone());
        }
        allowed.insert(self.supervisor_name.clone());

        self.director_data
            .agents
            .retain(|agent| allowed.contains(&agent.name));
        self.director_data
            .agent_id_to_name
            .retain(|_, name| allowed.contains(name));

        // Filter tasks to active epic's subtasks only (prevents cross-project task leakage)
        if let Some(epic_id) = self.epic_state.epic_id() {
            let epic_id = epic_id.to_string();
            self.director_data
                .ready_tasks
                .retain(|t| t.epic.as_deref() == Some(&epic_id));
            self.director_data
                .in_progress_tasks
                .retain(|t| t.epic.as_deref() == Some(&epic_id));
            self.director_data
                .epic_tasks
                .retain(|t| t.id == epic_id);
        }
    }

    /// Check if we should refresh CAS data
    pub fn should_refresh(&self) -> bool {
        self.last_refresh.elapsed() >= self.refresh_interval
    }

    /// Check if automatic prompting is globally enabled
    pub fn auto_prompt_enabled(&self) -> bool {
        self.auto_prompt.enabled
    }

    /// Get the auto-prompt configuration
    pub fn auto_prompt_config(&self) -> &AutoPromptConfig {
        &self.auto_prompt
    }

    /// Refresh CAS data from stores and detect state changes
    ///
    /// Returns a tuple of (prompts, events) for further processing.
    pub fn refresh_data(&mut self) -> anyhow::Result<(Vec<Prompt>, Vec<DirectorEvent>)> {
        let next_fingerprint = CasDbFingerprint::from_cas_dir(&self.cas_dir);
        let db_changed = match self.last_db_fingerprint {
            Some(prev) => prev != next_fingerprint,
            None => true,
        };
        let git_due = !self.director_data.git_loaded
            || self.last_git_refresh.elapsed() >= self.git_refresh_interval;

        let worktree_root = self.worktree_manager.as_ref().map(|m| m.worktree_root());
        if db_changed {
            let loaded =
                DirectorData::load_with_git(&self.cas_dir, worktree_root.as_deref(), git_due)?;
            self.director_data =
                merge_director_data_preserving_git(&self.director_data, loaded, git_due);
            if git_due {
                self.last_git_refresh = Instant::now();
            }
        } else if git_due {
            self.director_data
                .refresh_git_changes(&self.cas_dir, worktree_root.as_deref())?;
            self.last_git_refresh = Instant::now();
        } else {
            self.last_refresh = Instant::now();
            return Ok((Vec::new(), Vec::new()));
        }

        self.last_db_fingerprint = Some(next_fingerprint);
        self.last_refresh = Instant::now();

        // Sync session_id → pane_name mappings from agent store
        self.sync_session_mappings();

        // Detect state changes BEFORE filtering so new epics are visible to the
        // event detector. This allows EpicStarted to fire and update epic_state,
        // which the filter depends on for subsequent refresh cycles.
        let events = self.event_detector.detect_changes(&self.director_data);

        // Update epic_state immediately from detected events so the filter below
        // uses the correct epic_id (otherwise a new epic's tasks get filtered out)
        for event in &events {
            if let DirectorEvent::EpicStarted {
                epic_id,
                epic_title,
            } = event
            {
                self.current_epic_id = Some(epic_id.clone());
                self.epic_state = EpicState::Active {
                    epic_id: epic_id.clone(),
                    epic_title: epic_title.clone(),
                };
            }
        }

        // Now filter to current session (agents + tasks scoped to active epic)
        if db_changed {
            self.filter_director_agents_to_current_session();
        }

        // Generate prompts from events (respecting auto-prompt config)
        let prompts: Vec<Prompt> = events
            .iter()
            .filter_map(|event| {
                generate_prompt(
                    event,
                    &self.director_data,
                    &self.supervisor_name,
                    &self.auto_prompt,
                    self.supervisor_cli,
                    self.worker_cli,
                )
            })
            .collect();

        Ok((prompts, events))
    }

    /// Get the focused pane kind
    pub fn focused_kind(&self) -> Option<&PaneKind> {
        self.mux.focused().map(|p| p.kind())
    }

    /// Check if the supervisor pane is focused
    pub fn focused_is_supervisor(&self) -> bool {
        matches!(self.focused_kind(), Some(PaneKind::Supervisor))
    }

    /// Check if a worker pane is focused
    pub fn focused_is_worker(&self) -> bool {
        matches!(self.focused_kind(), Some(PaneKind::Worker))
    }

    /// Check if the focused pane accepts keyboard input
    ///
    /// Supervisor and worker panes accept keyboard input.
    pub fn focused_accepts_input(&self) -> bool {
        matches!(
            self.focused_kind(),
            Some(PaneKind::Supervisor | PaneKind::Worker | PaneKind::Shell)
        )
    }

    /// Get all worker names for layout (real + pending booting workers)
    pub fn layout_worker_names(&self) -> Vec<String> {
        let mut names = self.worker_names.clone();
        for pw in &self.pending_workers {
            if !names.contains(&pw.name) {
                names.push(pw.name.clone());
            }
        }
        names
    }

    /// Check if a worker name is pending (still booting)
    pub fn is_pending_worker(&self, name: &str) -> bool {
        self.pending_workers.iter().any(|pw| pw.name == name)
    }

    /// Add a worker to the pending set (called after prepare_worker_spawn succeeds)
    pub fn add_pending_worker(&mut self, name: String, isolate: bool) {
        self.pending_workers.push(PendingWorkerState {
            name,
            started_at: Instant::now(),
            isolate,
        });
        // Rebuild pane grid is NOT needed — pending workers are not navigable
        // But we do need to sync layout sizes so the boot pane gets space
        let _ = self.sync_pane_sizes();
    }

    /// Remove a worker from the pending set (called on spawn success or failure)
    pub fn remove_pending_worker(&mut self, name: &str) {
        self.pending_workers.retain(|pw| pw.name != name);
    }

    /// Get worker names
    pub fn worker_names(&self) -> &[String] {
        &self.worker_names
    }

    /// Get the number of active workers
    pub fn worker_count(&self) -> usize {
        self.worker_names.len()
    }

    /// Select a worker tab by index (0-based)
    ///
    /// Returns true if the selection changed.
    pub fn select_worker_tab(&mut self, index: usize) -> bool {
        let total = self.layout_worker_names().len();
        if index < total && index != self.selected_worker_tab {
            self.selected_worker_tab = index;
            true
        } else {
            false
        }
    }

    /// Select a worker tab by 1-based number (for keyboard shortcuts)
    ///
    /// Returns true if the selection changed.
    pub fn select_worker_by_number(&mut self, number: usize) -> bool {
        let total = self.layout_worker_names().len();
        if number > 0 && number <= total {
            self.select_worker_tab(number - 1)
        } else {
            false
        }
    }

    /// Get the currently selected worker name
    pub fn selected_worker(&self) -> Option<&str> {
        self.worker_names
            .get(self.selected_worker_tab)
            .map(|s| s.as_str())
    }

    /// Ensure selected_worker_tab is valid after workers change
    fn clamp_selected_worker_tab(&mut self) {
        let total = self.layout_worker_names().len();
        if total > 0 && self.selected_worker_tab >= total {
            self.selected_worker_tab = total - 1;
        }
    }

    /// Get supervisor name
    pub fn supervisor_name(&self) -> &str {
        &self.supervisor_name
    }

    /// Get factory session name (for prompt queue isolation)
    pub fn factory_session(&self) -> Option<&str> {
        self.factory_session.as_deref()
    }

    /// Set factory session name
    pub fn set_factory_session(&mut self, name: String) {
        self.factory_session = Some(name);
    }

    /// Get the worktree manager (if worktrees are enabled)
    pub fn worktree_manager(&self) -> Option<&WorktreeManager> {
        self.worktree_manager.as_ref()
    }

    /// Get the worktree manager mutably (if worktrees are enabled)
    pub fn worktree_manager_mut(&mut self) -> Option<&mut WorktreeManager> {
        self.worktree_manager.as_mut()
    }

    /// Check if worktree-based isolation is enabled
    pub fn worktrees_enabled(&self) -> bool {
        self.worktree_manager.is_some()
    }

    /// Get the lead session ID (UUID for Teams config.json)
    pub fn lead_session_id(&self) -> Option<&str> {
        self.lead_session_id.as_deref()
    }

    /// Get the director data
    pub fn director_data(&self) -> &DirectorData {
        &self.director_data
    }

    /// Get the CAS directory path
    pub fn cas_dir(&self) -> &std::path::Path {
        &self.cas_dir
    }

    /// Get the theme
    pub fn theme(&self) -> &ActiveTheme {
        &self.theme
    }

    /// Get the notifier
    pub fn notifier(&self) -> &Notifier {
        &self.notifier
    }

    /// Send notifications for detected events
    pub fn notify_events(&self, events: &[DirectorEvent]) {
        for event in events {
            self.notifier.notify_event(event);
        }
    }

    /// Set an error message (auto-dismisses after 5 seconds)
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error_message = Some(msg.into());
        self.error_set_at = Some(Instant::now());
    }

    /// Clear the error message
    pub fn clear_error(&mut self) {
        self.error_message = None;
        self.error_set_at = None;
    }

    /// Check if error should be auto-dismissed (after 5 seconds)
    pub fn check_error_timeout(&mut self) {
        if let Some(set_at) = self.error_set_at {
            if set_at.elapsed() >= Duration::from_secs(5) {
                self.clear_error();
            }
        }
    }

    /// Toggle between Panes and MissionControl factory view modes.
    pub fn toggle_factory_view_mode(&mut self) {
        use crate::ui::factory::renderer::FactoryViewMode;
        self.factory_view_mode = match self.factory_view_mode {
            FactoryViewMode::Panes => FactoryViewMode::MissionControl,
            FactoryViewMode::MissionControl => FactoryViewMode::Panes,
        };
    }

    /// Toggle sidebar collapsed state
    pub fn toggle_sidecar_collapsed(&mut self) {
        self.sidecar_collapsed = !self.sidecar_collapsed;
        // Recalculate PTY dimensions to match new layout
        let _ = self.handle_resize(self.terminal_cols, self.terminal_rows);
    }

    /// Handle resize event
    pub fn handle_resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        // Store terminal dimensions
        self.terminal_cols = cols;
        self.terminal_rows = rows;

        // Include pending workers in layout so boot panes get space
        let all_names = self.layout_worker_names();

        // Calculate actual layout areas and resize panes to match
        let area = Rect::new(0, 0, cols, rows);
        let layout = FactoryLayout::calculate_from_names_with_header_rows(
            area,
            &all_names,
            self.tabbed_workers,
            self.sidecar_collapsed,
            self.layout_sizes,
            0,
        );

        // Resize only REAL worker panes (pending workers have no PTY)
        if layout.is_tabbed {
            // Tabbed mode: all workers share the same viewport size
            if let Some(content_area) = layout.worker_content {
                let inner_height = content_area.height.saturating_sub(2);
                let inner_width = content_area.width.saturating_sub(2);

                for name in &self.worker_names {
                    if let Some(pane) = self.mux.get_mut(name) {
                        let _ = pane.resize(inner_height, inner_width);
                    }
                }
            }
        } else {
            // Side-by-side mode: find each real worker's index in the combined list
            for name in &self.worker_names {
                if let Some(idx) = all_names.iter().position(|n| n == name) {
                    if let Some(worker_area) = layout.worker_areas.get(idx) {
                        let inner_height = worker_area.height.saturating_sub(2);
                        let inner_width = worker_area.width.saturating_sub(2);
                        if let Some(pane) = self.mux.get_mut(name) {
                            let _ = pane.resize(inner_height, inner_width);
                        }
                    }
                }
            }
        }

        // Resize supervisor pane
        if let Some(pane) = self.mux.get_mut(&self.supervisor_name) {
            let inner_height = layout.supervisor_area.height.saturating_sub(2);
            let inner_width = layout.supervisor_area.width.saturating_sub(2);
            let _ = pane.resize(inner_height, inner_width);
        }

        Ok(())
    }

    /// Sync pane sizes with current terminal dimensions
    ///
    /// In daemon mode, crossterm::terminal::size() returns a default (80x24) instead
    /// of failing, so we prefer stored dimensions if they're set to something reasonable.
    pub fn sync_pane_sizes(&mut self) -> anyhow::Result<()> {
        // Use stored dimensions if they're set (indicates daemon mode with client-provided size)
        // Only fall back to crossterm if stored dimensions are at default (120x40)
        let (cols, rows) = if self.terminal_cols > 120 || self.terminal_rows > 40 {
            // We have real dimensions from a client resize event
            (self.terminal_cols, self.terminal_rows)
        } else {
            // Try crossterm, but validate the result
            match crossterm::terminal::size() {
                Ok((c, r)) if c > 80 || r > 24 => (c, r),
                _ => (self.terminal_cols, self.terminal_rows),
            }
        };
        tracing::info!(
            "sync_pane_sizes: using {}x{} (stored: {}x{})",
            cols,
            rows,
            self.terminal_cols,
            self.terminal_rows
        );
        self.handle_resize(cols, rows)
    }
}

fn merge_director_data_preserving_git(
    previous: &DirectorData,
    mut loaded: DirectorData,
    git_due: bool,
) -> DirectorData {
    if !git_due && previous.git_loaded {
        loaded.changes = previous.changes.clone();
        loaded.git_loaded = true;
    }
    loaded
}

pub(crate) fn queue_supervisor_intro_prompt(
    cas_dir: &std::path::Path,
    supervisor_name: &str,
    supervisor_cli: cas_mux::SupervisorCli,
    worker_names: &[String],
    factory_session: Option<&str>,
) {
    let worker_list = if worker_names.is_empty() {
        "(none)".to_string()
    } else {
        worker_names.join(", ")
    };
    let prompt = match supervisor_cli {
        cas_mux::SupervisorCli::Codex => format!(
            "Codex supervisor startup:\n\
- Use skills: cas-supervisor, cas-codex-supervisor-checklist\n\
- No hooks: call MCP tools explicitly (tasks/memory/rules/search)\n\
- Do NOT use /cas-start, /cas-context, or /cas-end\n\
- Canonical current workers for this session: {worker_list}\n\
- First steps: mcp__cs__coordination action=whoami; mcp__cs__task action=list task_type=epic; mcp__cs__task action=ready"
        ),
        cas_mux::SupervisorCli::Claude => return,
    };

    if let Ok(queue) = open_prompt_queue_store(cas_dir) {
        if let Some(session) = factory_session {
            let _ = queue.enqueue_with_session("cas", supervisor_name, &prompt, session);
        } else {
            let _ = queue.enqueue("cas", supervisor_name, &prompt);
        }
    }
}

pub(crate) fn queue_codex_worker_intro_prompt(
    cas_dir: &std::path::Path,
    worker_name: &str,
    worker_cli: cas_mux::SupervisorCli,
) {
    match worker_cli {
        cas_mux::SupervisorCli::Codex => {
            // Codex workers now receive startup workflow as the initial codex prompt arg at spawn time.
            // Avoid queue injection here to prevent duplicate or draft-only startup prompts.
        }
        cas_mux::SupervisorCli::Claude => {
            // Workers in worktrees usually can't access MCP tools due to SQLite contention.
            // Tell them to try once, then immediately fall back to built-in tools.
            let prompt = format!(
                "You are a CAS factory worker ({worker_name}).\n\
                 Check your assigned tasks: `mcp__cas__task action=mine`\n\n\
                 IMPORTANT — if mcp__cas__* tools are unavailable:\n\
                 1. Do NOT retry or debug MCP — use built-in tools (Read, Edit, Write, Bash, Glob, Grep)\n\
                 2. Your task details are in the supervisor's message — scroll up in your conversation\n\
                 3. Notify supervisor immediately via CLI fallback:\n\
                    `cas factory message --project-dir {cas_dir} --target supervisor --message \"Worker {worker_name}: MCP tools unavailable. Standing by for task details via message.\"`\n\
                 Do not remain silently idle — always notify the supervisor if you cannot access MCP tools.",
                cas_dir = cas_dir.parent().unwrap_or(cas_dir).display()
            );
            if let Ok(queue) = open_prompt_queue_store(cas_dir) {
                let _ = queue.enqueue("cas", worker_name, &prompt);
            }
        }
    }
}

/// A change in epic state
#[derive(Debug, Clone)]
pub enum EpicStateChange {
    /// An epic was started
    Started {
        epic_id: String,
        epic_title: String,
        previous_state: EpicState,
    },
    /// An epic was completed
    Completed { epic_id: String, epic_title: String },
}

/// Extract selected text from a terminal pane.
///
/// This function extracts text from the pane's terminal buffer based on
/// the selection coordinates. It handles single-line and multi-line
/// selections, respecting line boundaries.
fn extract_selected_text_from_pane(
    pane: &cas_mux::Pane,
    selection: &super::selection::Selection,
) -> Option<String> {
    if selection.is_empty() {
        return None;
    }

    let (sr, sc, er, ec) = selection.normalized();

    // Adjust selection rows for any scrolling that happened after the selection was made.
    let scroll_delta = pane.scroll_offset() as i32 - selection.scroll_offset as i32;

    let mut text = String::new();

    for row in sr..=er {
        let adjusted_row = row as i32 + scroll_delta;
        if adjusted_row < 0 {
            continue;
        }
        let row_text = match pane.dump_row(adjusted_row as u16) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let chars: Vec<char> = row_text.chars().collect();

        let (start_col, end_col) = if sr == er {
            // Single line selection
            (sc as usize, ec as usize)
        } else if row == sr {
            // First line: from start_col to end
            (sc as usize, chars.len().saturating_sub(1))
        } else if row == er {
            // Last line: from start to end_col
            (0, ec as usize)
        } else {
            // Middle lines: entire line
            (0, chars.len().saturating_sub(1))
        };

        // Extract the relevant portion
        let start = start_col.min(chars.len());
        let end = (end_col + 1).min(chars.len());
        if start < end {
            let selected: String = chars[start..end].iter().collect();
            text.push_str(selected.trim_end());
        }

        // Add newline between lines (but not after last line)
        if row < er {
            text.push('\n');
        }
    }

    if text.is_empty() { None } else { Some(text) }
}

/// Detect the initial epic state from loaded data.
///
/// If `preferred_epic_id` is set (from session metadata or explicit tracking),
/// look it up directly instead of scanning all epics. Falls back to scanning
/// if the preferred epic is not found or is closed.
pub(crate) fn detect_epic_state(
    data: &DirectorData,
    preferred_epic_id: Option<&str>,
) -> EpicState {
    use cas_types::TaskStatus;

    // If we have an explicit epic ID, try to use it directly (skip scanning)
    if let Some(epic_id) = preferred_epic_id {
        if let Some(epic) = data.epic_tasks.iter().find(|e| e.id == epic_id) {
            if epic.status != TaskStatus::Closed {
                return EpicState::Active {
                    epic_id: epic.id.clone(),
                    epic_title: epic.title.clone(),
                };
            }
        }
    }

    // Find an in-progress epic first (highest priority)
    for epic in &data.epic_tasks {
        if epic.status == TaskStatus::InProgress {
            return EpicState::Active {
                epic_id: epic.id.clone(),
                epic_title: epic.title.clone(),
            };
        }
    }

    // Fall back to open epics that have a branch set (auto-created branch)
    // This allows workers to branch from the epic branch before the epic is started
    // When multiple qualify, pick the one with the lexicographically greatest ID
    // for deterministic selection (avoids flip-flopping when list order is unstable)
    if let Some(epic) = data
        .epic_tasks
        .iter()
        .filter(|e| e.status == TaskStatus::Open && e.branch.is_some())
        .max_by(|a, b| a.id.cmp(&b.id))
    {
        return EpicState::Active {
            epic_id: epic.id.clone(),
            epic_title: epic.title.clone(),
        };
    }

    // Completing state is transitioned to via handle_epic_events() when EpicCompleted fires
    // Initial state detection only identifies Active epics; Completing is a transient state
    EpicState::Idle
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cas_factory::{FileChangeInfo, GitFileStatus, SourceChangesInfo};

    use super::{DirectorData, merge_director_data_preserving_git};

    fn data_with_changes(git_loaded: bool, changes: Vec<SourceChangesInfo>) -> DirectorData {
        DirectorData {
            ready_tasks: Vec::new(),
            in_progress_tasks: Vec::new(),
            epic_tasks: Vec::new(),
            agents: Vec::new(),
            activity: Vec::new(),
            agent_id_to_name: HashMap::new(),
            changes,
            git_loaded,
            reminders: Vec::new(),
            epic_closed_counts: HashMap::new(),
        }
    }

    #[test]
    fn epic_state_update_before_filter_retains_new_epic_tasks() {
        use cas_factory::{EpicState, TaskSummary};
        use cas_types::{Priority, TaskStatus, TaskType};

        use super::{DirectorEvent, DirectorEventDetector};

        let old_epic_id = "epic-old";
        let new_epic_id = "epic-new";
        let new_epic_title = "New Feature Epic";

        // Simulate director_data with a new Open-with-branch epic and its subtasks
        let mut data = DirectorData {
            ready_tasks: vec![TaskSummary {
                id: "task-1".to_string(),
                title: "Subtask of new epic".to_string(),
                status: TaskStatus::Open,
                priority: Priority::MEDIUM,
                assignee: None,
                task_type: TaskType::Task,
                epic: Some(new_epic_id.to_string()),
                branch: None,
            }],
            in_progress_tasks: vec![TaskSummary {
                id: "task-2".to_string(),
                title: "In-progress subtask".to_string(),
                status: TaskStatus::InProgress,
                priority: Priority::MEDIUM,
                assignee: Some("worker-1".to_string()),
                task_type: TaskType::Task,
                epic: Some(new_epic_id.to_string()),
                branch: None,
            }],
            epic_tasks: vec![TaskSummary {
                id: new_epic_id.to_string(),
                title: new_epic_title.to_string(),
                status: TaskStatus::Open,
                priority: Priority::MEDIUM,
                assignee: None,
                task_type: TaskType::Epic,
                epic: None,
                branch: Some("epic/new-feature".to_string()),
            }],
            agents: Vec::new(),
            activity: Vec::new(),
            agent_id_to_name: HashMap::new(),
            changes: Vec::new(),
            git_loaded: false,
            reminders: Vec::new(),
            epic_closed_counts: HashMap::new(),
        };

        // Start with stale epic_state pointing to old epic
        let mut epic_state = EpicState::Active {
            epic_id: old_epic_id.to_string(),
            epic_title: "Old Epic".to_string(),
        };

        // Event detector sees the new epic and fires EpicStarted
        let mut detector = DirectorEventDetector::new(
            vec!["worker-1".to_string()],
            "supervisor".to_string(),
        );
        // Initialize with empty state so detector sees the new epic as new
        detector.initialize(&DirectorData {
            ready_tasks: Vec::new(),
            in_progress_tasks: Vec::new(),
            epic_tasks: Vec::new(),
            agents: Vec::new(),
            activity: Vec::new(),
            agent_id_to_name: HashMap::new(),
            changes: Vec::new(),
            git_loaded: false,
            reminders: Vec::new(),
            epic_closed_counts: HashMap::new(),
        });

        let events = detector.detect_changes(&data);

        // Verify EpicStarted was fired
        let epic_started = events.iter().any(|e| {
            matches!(e, DirectorEvent::EpicStarted { epic_id, .. } if epic_id == new_epic_id)
        });
        assert!(epic_started, "EpicStarted event should fire for new epic");

        // THE FIX: Update epic_state from events BEFORE filtering
        for event in &events {
            if let DirectorEvent::EpicStarted {
                epic_id,
                epic_title,
            } = event
            {
                epic_state = EpicState::Active {
                    epic_id: epic_id.clone(),
                    epic_title: epic_title.clone(),
                };
            }
        }

        // Filter tasks to active epic (simulating filter_director_agents_to_current_session)
        if let Some(eid) = epic_state.epic_id() {
            let eid = eid.to_string();
            data.ready_tasks
                .retain(|t| t.epic.as_deref() == Some(&eid));
            data.in_progress_tasks
                .retain(|t| t.epic.as_deref() == Some(&eid));
            data.epic_tasks.retain(|t| t.id == eid);
        }

        // Tasks should be retained because epic_state now points to new epic
        assert_eq!(data.ready_tasks.len(), 1, "ready_tasks should not be empty after filter");
        assert_eq!(data.in_progress_tasks.len(), 1, "in_progress_tasks should not be empty after filter");
        assert_eq!(data.epic_tasks.len(), 1, "epic_tasks should have the new epic");
    }

    #[test]
    fn preserves_previous_changes_when_git_refresh_not_due() {
        let previous = data_with_changes(
            true,
            vec![SourceChangesInfo {
                source_name: "main".to_string(),
                source_path: std::path::PathBuf::from("."),
                agent_name: None,
                changes: vec![FileChangeInfo {
                    file_path: "src/main.rs".to_string(),
                    lines_added: 3,
                    lines_removed: 1,
                    status: GitFileStatus::Modified,
                    staged: false,
                }],
                total_added: 3,
                total_removed: 1,
            }],
        );
        let loaded_without_git = data_with_changes(false, Vec::new());

        let merged = merge_director_data_preserving_git(&previous, loaded_without_git, false);

        assert!(merged.git_loaded);
        assert_eq!(merged.changes.len(), 1);
        assert_eq!(merged.changes[0].source_name, "main");
    }

    #[test]
    fn keeps_loaded_changes_when_git_refresh_is_due() {
        let previous = data_with_changes(true, Vec::new());
        let loaded_with_git = data_with_changes(
            true,
            vec![SourceChangesInfo {
                source_name: "worker-1".to_string(),
                source_path: std::path::PathBuf::from("."),
                agent_name: Some("worker-1".to_string()),
                changes: vec![FileChangeInfo {
                    file_path: "README.md".to_string(),
                    lines_added: 10,
                    lines_removed: 0,
                    status: GitFileStatus::Added,
                    staged: false,
                }],
                total_added: 10,
                total_removed: 0,
            }],
        );

        let merged = merge_director_data_preserving_git(&previous, loaded_with_git, true);

        assert!(merged.git_loaded);
        assert_eq!(merged.changes.len(), 1);
        assert_eq!(merged.changes[0].source_name, "worker-1");
    }
}
