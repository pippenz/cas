//! Event detection for the Director
//!
//! Detects state changes in CAS data by comparing snapshots.
//! Used to trigger auto-prompting and activity logging.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::ui::factory::director::data::{DirectorData, TaskSummary};
use cas_types::TaskStatus;
use chrono::DateTime;

/// Debounce duration for events (don't emit same event within this window)
const DEBOUNCE_DURATION: Duration = Duration::from_secs(30);

/// Rate limit for WorkerIdle events — at most one per worker per 5 minutes.
/// Idle notifications are low-priority and flood the supervisor when multiple
/// workers idle simultaneously.
const IDLE_RATE_LIMIT: Duration = Duration::from_secs(300);

/// A worker whose `last_heartbeat` is within this many seconds of `now_utc` is
/// considered "recently alive" for the purposes of the idle gate (cas-4038).
/// CC agents heartbeat on every tool call, so a 60s window covers one full
/// turn without generating a false-idle notification.
const FRESH_HEARTBEAT_SECS: i64 = 60;

/// A worker whose `latest_activity` timestamp is within this many seconds of
/// `now_utc` is considered "recently active" (cas-4038). Combined with the
/// fresh-heartbeat gate: BOTH must be true to suppress a WorkerIdle tick.
/// 120s gives one comfortable "between tasks" turn window at the 2s refresh
/// rate without masking a genuinely stalled worker.
const RECENT_ACTIVITY_SECS: i64 = 120;

/// Number of consecutive refresh ticks an agent must appear idle before
/// WorkerIdle is emitted.
///
/// The daemon's `refresh_interval` is 2s (see
/// `cas-cli/src/ui/factory/daemon/runtime/lifecycle.rs`), so this gives a
/// sustained-idle window of roughly `2 * refresh_interval = 4s`. The window
/// is long enough to absorb normal close-X → start-Y transitions (where a
/// worker finishes one task and immediately claims the next) without
/// emitting a spurious "worker idle" prompt to the supervisor, and short
/// enough that genuinely idle workers are still surfaced quickly.
///
/// Before this threshold existed, a single refresh landing inside the
/// sub-second gap between a worker closing task X and starting task Y would
/// emit `WorkerIdle` immediately, producing apparent out-of-order delivery
/// ("idle notification arrived before the claim") even though the worker
/// was already working. See task cas-f9e8.
const IDLE_CONSECUTIVE_TICKS: u32 = 2;

/// Events detected from CAS state changes
#[derive(Debug, Clone)]
pub enum DirectorEvent {
    /// A task was assigned to a worker
    TaskAssigned {
        task_id: String,
        task_title: String,
        worker: String,
    },
    /// A task was completed
    TaskCompleted {
        task_id: String,
        task_title: String,
        worker: String,
    },
    /// A task was blocked
    TaskBlocked {
        task_id: String,
        task_title: String,
        worker: String,
    },
    /// A worker became idle (no in-progress tasks)
    WorkerIdle { worker: String },
    /// A worker has an in-progress task and a fresh heartbeat, but no
    /// observable activity (file edit, commit, subagent event, ...) for
    /// longer than the configured stall threshold (cas-9829). Heartbeat
    /// alone cannot distinguish "healthy" from "printed a plan and
    /// stopped" — this is the activity-based signal that fills that gap.
    ///
    /// `escalate = false` on first detection in a stall streak: the
    /// director auto-nudges the worker once (re-injects the task prompt)
    /// instead of paging the supervisor immediately, since a single
    /// re-poke often unsticks a stalled agent. `escalate = true` once the
    /// worker is still stalled after that nudge — the supervisor is
    /// notified at that point.
    WorkerStalled {
        worker: String,
        task_id: String,
        elapsed_secs: u64,
        escalate: bool,
    },
    /// A new agent registered
    AgentRegistered {
        agent_id: String,
        agent_name: String,
    },
    /// An epic was started (detected by new epic-type task)
    EpicStarted { epic_id: String, epic_title: String },
    /// All tasks in an epic are complete
    EpicCompleted { epic_id: String },
    /// All subtasks of an epic are closed but the epic itself is still open
    EpicAllSubtasksClosed {
        epic_id: String,
        epic_title: String,
    },
}

impl DirectorEvent {
    /// Get the worker/agent this event targets (for prompt injection)
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::TaskAssigned { worker, .. } => Some(worker),
            Self::TaskCompleted { worker, .. } => Some(worker),
            Self::TaskBlocked { worker, .. } => Some(worker),
            Self::WorkerIdle { worker } => Some(worker),
            Self::WorkerStalled { worker, .. } => Some(worker),
            Self::AgentRegistered { agent_name, .. } => Some(agent_name),
            Self::EpicStarted { .. } => None, // Broadcast or supervisor
            Self::EpicCompleted { .. } => None,
            Self::EpicAllSubtasksClosed { .. } => None, // Targets supervisor
        }
    }

    /// Get a description of the event for logging
    pub fn description(&self) -> String {
        match self {
            Self::TaskAssigned {
                task_id,
                worker,
                task_title,
            } => {
                format!("{worker} assigned task {task_id} ({task_title})")
            }
            Self::TaskCompleted {
                task_id,
                worker,
                task_title,
            } => {
                format!("{worker} completed task {task_id} ({task_title})")
            }
            Self::TaskBlocked {
                task_id,
                worker,
                task_title,
            } => {
                format!("{worker} blocked on task {task_id} ({task_title})")
            }
            Self::WorkerIdle { worker } => {
                format!("{worker} is idle")
            }
            Self::WorkerStalled {
                worker,
                task_id,
                elapsed_secs,
                escalate,
            } => {
                if *escalate {
                    format!(
                        "{worker} still stalled on {task_id} after {elapsed_secs}s (nudged, escalating to supervisor)"
                    )
                } else {
                    format!("{worker} stalled on {task_id}: no activity for {elapsed_secs}s")
                }
            }
            Self::AgentRegistered { agent_name, .. } => {
                format!("{agent_name} registered")
            }
            Self::EpicStarted {
                epic_id,
                epic_title,
            } => {
                format!("Epic {epic_id} started: {epic_title}")
            }
            Self::EpicCompleted { epic_id } => {
                format!("Epic {epic_id} completed")
            }
            Self::EpicAllSubtasksClosed {
                epic_id,
                epic_title,
            } => {
                format!("All subtasks of epic '{epic_title}' ({epic_id}) are closed — ready to close epic")
            }
        }
    }

    /// Get a unique key for debouncing this event
    ///
    /// Events with the same key are considered duplicates within the debounce window.
    pub fn debounce_key(&self) -> String {
        match self {
            Self::TaskAssigned {
                task_id, worker, ..
            } => {
                format!("assigned:{task_id}:{worker}")
            }
            Self::TaskCompleted {
                task_id, worker, ..
            } => {
                format!("completed:{task_id}:{worker}")
            }
            Self::TaskBlocked {
                task_id, worker, ..
            } => {
                format!("blocked:{task_id}:{worker}")
            }
            Self::WorkerIdle { worker } => {
                format!("idle:{worker}")
            }
            Self::WorkerStalled {
                worker, escalate, ..
            } => {
                format!("stalled:{worker}:{escalate}")
            }
            Self::AgentRegistered { agent_id, .. } => {
                format!("registered:{agent_id}")
            }
            Self::EpicStarted { epic_id, .. } => {
                format!("epic_started:{epic_id}")
            }
            Self::EpicCompleted { epic_id } => {
                format!("epic_completed:{epic_id}")
            }
            Self::EpicAllSubtasksClosed { epic_id, .. } => {
                format!("epic_all_subtasks_closed:{epic_id}")
            }
        }
    }

    /// Get the event type as a string (for recording export)
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::TaskAssigned { .. } => "task_assigned",
            Self::TaskCompleted { .. } => "task_completed",
            Self::TaskBlocked { .. } => "task_blocked",
            Self::WorkerIdle { .. } => "worker_idle",
            Self::WorkerStalled { .. } => "worker_stalled",
            Self::AgentRegistered { .. } => "agent_registered",
            Self::EpicStarted { .. } => "epic_started",
            Self::EpicCompleted { .. } => "epic_completed",
            Self::EpicAllSubtasksClosed { .. } => "epic_all_subtasks_closed",
        }
    }

    /// Convert event data to JSON (for recording export)
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::TaskAssigned {
                task_id,
                task_title,
                worker,
            } => serde_json::json!({
                "task_id": task_id,
                "task_title": task_title,
                "worker": worker,
            }),
            Self::TaskCompleted {
                task_id,
                task_title,
                worker,
            } => serde_json::json!({
                "task_id": task_id,
                "task_title": task_title,
                "worker": worker,
            }),
            Self::TaskBlocked {
                task_id,
                task_title,
                worker,
            } => serde_json::json!({
                "task_id": task_id,
                "task_title": task_title,
                "worker": worker,
            }),
            Self::WorkerIdle { worker } => serde_json::json!({
                "worker": worker,
            }),
            Self::WorkerStalled {
                worker,
                task_id,
                elapsed_secs,
                escalate,
            } => serde_json::json!({
                "worker": worker,
                "task_id": task_id,
                "elapsed_secs": elapsed_secs,
                "escalate": escalate,
            }),
            Self::AgentRegistered {
                agent_id,
                agent_name,
            } => serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
            }),
            Self::EpicStarted {
                epic_id,
                epic_title,
            } => serde_json::json!({
                "epic_id": epic_id,
                "epic_title": epic_title,
            }),
            Self::EpicCompleted { epic_id } => serde_json::json!({
                "epic_id": epic_id,
            }),
            Self::EpicAllSubtasksClosed {
                epic_id,
                epic_title,
            } => serde_json::json!({
                "epic_id": epic_id,
                "epic_title": epic_title,
            }),
        }
    }
}

/// State snapshot for comparison
#[derive(Debug, Clone, Default)]
struct DirectorState {
    /// Map of task_id -> (status, assignee)
    tasks: HashMap<String, (TaskStatus, Option<String>)>,
    /// Map of task_id -> title (for lookup when tasks disappear from active sets)
    task_titles: HashMap<String, String>,
    /// Set of active agent IDs
    active_agents: HashSet<String>,
    /// Map of epic_id -> (status, has_branch)
    epic_statuses: HashMap<String, (TaskStatus, bool)>,
    /// Map of epic_id -> count of active (non-closed) subtasks
    epic_active_subtask_counts: HashMap<String, usize>,
}

impl DirectorState {
    fn from_data(data: &DirectorData) -> Self {
        let mut tasks = HashMap::new();
        let mut task_titles = HashMap::new();

        // Add ready tasks
        for task in &data.ready_tasks {
            tasks.insert(task.id.clone(), (task.status, task.assignee.clone()));
            task_titles.insert(task.id.clone(), task.title.clone());
        }

        // Add in-progress tasks
        for task in &data.in_progress_tasks {
            tasks.insert(task.id.clone(), (task.status, task.assignee.clone()));
            task_titles.insert(task.id.clone(), task.title.clone());
        }

        let active_agents: HashSet<String> = data.agents.iter().map(|a| a.id.clone()).collect();

        // Track epic statuses and branch presence
        let epic_statuses: HashMap<String, (TaskStatus, bool)> = data
            .epic_tasks
            .iter()
            .map(|e| (e.id.clone(), (e.status, e.branch.is_some())))
            .collect();

        // Count active (non-closed) subtasks per epic.
        // Tasks in ready_tasks or in_progress_tasks are active by definition.
        let mut epic_active_subtask_counts: HashMap<String, usize> = HashMap::new();
        for task in data.ready_tasks.iter().chain(data.in_progress_tasks.iter()) {
            if let Some(ref epic_id) = task.epic {
                *epic_active_subtask_counts.entry(epic_id.clone()).or_insert(0) += 1;
            }
        }

        Self {
            tasks,
            task_titles,
            active_agents,
            epic_statuses,
            epic_active_subtask_counts,
        }
    }
}

/// Detects events by comparing CAS state snapshots
pub struct DirectorEventDetector {
    /// Previous state snapshot
    last_state: DirectorState,
    /// Factory worker names (for filtering)
    worker_names: Vec<String>,
    /// Supervisor name
    supervisor_name: String,
    /// Last prompt times for debouncing (event key -> instant)
    last_prompt_times: HashMap<String, Instant>,
    /// Workers that have been removed (shutdown/crashed) — suppress their events
    removed_workers: HashSet<String>,
    /// Consecutive refresh ticks each factory agent has appeared idle.
    /// Used with `IDLE_CONSECUTIVE_TICKS` to debounce `WorkerIdle` so that
    /// sub-second close-X → start-Y transitions do not generate spurious
    /// idle prompts. Keyed by agent id.
    consecutive_idle_ticks: HashMap<String, u32>,
    /// Agents for whom `WorkerIdle` has already been emitted in the current
    /// idle streak. Cleared once the agent picks up a task again, so a fresh
    /// idle streak can trigger another emission (subject to `IDLE_RATE_LIMIT`
    /// in `debounce_events`). Keyed by agent id.
    idle_already_emitted: HashSet<String>,
    /// Tasks for which `TaskCompleted` has already been announced this session.
    ///
    /// **Never cleared on active-set reappearance.** When a task oscillates
    /// (lease expires → temporarily disappears → lease re-acquired → reappears)
    /// the reappearance is NOT a new assignment; it is the same task continuing.
    /// Clearing the guard on reappearance would cause a re-emission on every
    /// subsequent oscillation cycle — exactly the ~30s re-fire bug (cas-55dc).
    ///
    /// Keyed by task_id.
    task_completed_announced: HashSet<String>,
    /// Assignment pairs for which `TaskAssigned` has already been announced.
    ///
    /// Same never-clear-on-reappearance policy as `task_completed_announced`:
    /// if a task oscillates out and back in with the same assignee the
    /// assignment was already dispatched and must not re-fire.
    ///
    /// Key is `"{task_id}:{assignee_id}"`. A genuine reassignment to a
    /// *different* worker produces a new key and is therefore not suppressed.
    task_assigned_announced: HashSet<String>,
    /// Workers that have already received the one-shot stall auto-nudge in
    /// the current stall streak (cas-9829). Cleared once the worker's
    /// activity resumes (elapsed drops back under the threshold) or the
    /// worker leaves the active set, so a fresh stall streak nudges again.
    stall_nudged: HashSet<String>,
    /// Workers for whom the stall has already been escalated to the
    /// supervisor in the current streak. Cleared alongside `stall_nudged`.
    stall_escalated: HashSet<String>,
    /// Seconds of no observable activity (with a fresh heartbeat and an
    /// in-progress task) before a worker is flagged `WorkerStalled`.
    /// Defaults to `cas_factory::DEFAULT_STALL_THRESHOLD_SECS`; overridden
    /// via [`Self::set_stall_threshold_secs`] from `.cas/config.toml`
    /// `[factory] stall_threshold_secs`.
    stall_threshold_secs: u64,
}

impl DirectorEventDetector {
    /// Create a new event detector
    pub fn new(worker_names: Vec<String>, supervisor_name: String) -> Self {
        Self {
            last_state: DirectorState::default(),
            worker_names,
            supervisor_name,
            last_prompt_times: HashMap::new(),
            removed_workers: HashSet::new(),
            consecutive_idle_ticks: HashMap::new(),
            idle_already_emitted: HashSet::new(),
            task_completed_announced: HashSet::new(),
            task_assigned_announced: HashSet::new(),
            stall_nudged: HashSet::new(),
            stall_escalated: HashSet::new(),
            stall_threshold_secs: cas_factory::DEFAULT_STALL_THRESHOLD_SECS,
        }
    }

    /// Initialize with current state (call after first data load)
    pub fn initialize(&mut self, data: &DirectorData) {
        self.last_state = DirectorState::from_data(data);
    }

    /// Override the stall-detection threshold (default
    /// `cas_factory::DEFAULT_STALL_THRESHOLD_SECS`). Call once after
    /// construction, before the first `detect_changes`/`detect_changes_at`,
    /// with the value resolved from `.cas/config.toml`
    /// `[factory] stall_threshold_secs` (cas-9829).
    pub fn set_stall_threshold_secs(&mut self, secs: u64) {
        self.stall_threshold_secs = secs;
    }

    /// Add a worker to the tracked list (call when spawning workers dynamically)
    pub fn add_worker(&mut self, name: String) {
        // cas-c790: guard against the supervisor's name being silently added to
        // the worker list on resume/reconnect paths, which would cause
        // is_worker_agent_name to return true for the lead — leaking WorkerIdle
        // events for the supervisor (recurrence of cas-b67d).
        if name == self.supervisor_name {
            return;
        }
        if !self.worker_names.contains(&name) {
            self.worker_names.push(name);
        }
    }

    /// Remove a worker from the tracked list (call when shutting down workers)
    pub fn remove_worker(&mut self, name: &str) {
        self.worker_names.retain(|n| n != name);
        self.removed_workers.insert(name.to_string());
    }

    /// Detect changes between the last state and new data.
    ///
    /// Thin shim: captures `Instant::now()` and `Utc::now()` and delegates to
    /// [`detect_changes_at`]. Production callers use this; tests that need to
    /// isolate the state-guard from the 30s debounce window or the heartbeat
    /// freshness gate call `detect_changes_at` directly with synthetic clocks.
    pub fn detect_changes(
        &mut self,
        data: &DirectorData,
        current_epic_id: Option<&str>,
    ) -> Vec<DirectorEvent> {
        self.detect_changes_at(data, current_epic_id, Instant::now(), chrono::Utc::now())
    }

    /// Core implementation of change detection with injectable clocks.
    ///
    /// Returns a list of detected events. Call after each refresh.
    ///
    /// `now` — `Instant` used for debounce bookkeeping (`last_prompt_times`).
    /// Pass `Instant::now()` in production; inject a synthetic value in tests
    /// to isolate state-guards from the 30s `DEBOUNCE_DURATION` window.
    ///
    /// `now_utc` — `DateTime<Utc>` used for heartbeat / activity freshness
    /// comparisons. Pass `Utc::now()` in production; inject a synthetic value
    /// in tests to exercise the `FRESH_HEARTBEAT_SECS` / `RECENT_ACTIVITY_SECS`
    /// gates without actually sleeping.
    ///
    /// `current_epic_id` is the factory app's currently-tracked epic (pass
    /// `None` at init time before any epic has been resolved). When `Some`,
    /// `EpicStarted` for an Open-with-branch epic is only emitted if the
    /// candidate is **strictly better** than the active epic under the shared
    /// subtask-count heuristic (see [`pick_best_open_branch_epic`]). This
    /// prevents a fresh zero-subtask Open-with-branch epic from overwriting
    /// the active `epic_state` mid-session (see task cas-4181).
    /// `InProgress` epic transitions still emit unconditionally.
    pub fn detect_changes_at(
        &mut self,
        data: &DirectorData,
        current_epic_id: Option<&str>,
        now: Instant,
        now_utc: DateTime<chrono::Utc>,
    ) -> Vec<DirectorEvent> {
        let new_state = DirectorState::from_data(data);
        let mut events = Vec::new();

        // Build lookup maps for task info
        let task_info: HashMap<&str, &TaskSummary> = data
            .ready_tasks
            .iter()
            .chain(data.in_progress_tasks.iter())
            .map(|t| (t.id.as_str(), t))
            .collect();

        // Detect task assignments (task now has assignee that it didn't before).
        //
        // Terminal-status guard (cas-177f): only emit `TaskAssigned` when the
        // new status is actionable. Closed and Blocked tasks must never
        // generate dispatch prompts, even if they somehow leak into
        // `new_state.tasks` via a data-loading bug or future refactor. This
        // also supersedes the older
        // `bugfix_director_dispatches_blocked_tasks` memory — the `ready_tasks`
        // bucket in `crates/cas-factory/src/director.rs` still conflates
        // `Open | Blocked`, so without this guard blocked assignments would
        // still be dispatched.
        for (task_id, (new_status, new_assignee)) in &new_state.tasks {
            if let Some(assignee) = new_assignee {
                let dispatchable =
                    matches!(new_status, TaskStatus::Open | TaskStatus::InProgress);

                // Check if this is a new assignment
                let was_assigned = self
                    .last_state
                    .tasks
                    .get(task_id)
                    .map(|(_, old_assignee)| old_assignee.as_ref() == Some(assignee))
                    .unwrap_or(false);

                if dispatchable && !was_assigned && self.is_factory_agent(assignee, data) {
                    // State-guard (cas-55dc): suppress re-emission if this
                    // (task, assignee) pair was already announced. Oscillation
                    // (lease churn causes the task to temporarily leave and
                    // re-enter active sets with the same assignee) must not
                    // re-fire TaskAssigned. A genuine reassignment to a
                    // *different* worker produces a different key and is not
                    // suppressed.
                    let announced_key = format!("{task_id}:{assignee}");
                    if !self.task_assigned_announced.contains(&announced_key) {
                        self.task_assigned_announced.insert(announced_key);
                        let task_title = task_info
                            .get(task_id.as_str())
                            .map(|t| t.title.clone())
                            .unwrap_or_default();

                        events.push(DirectorEvent::TaskAssigned {
                            task_id: task_id.clone(),
                            task_title,
                            worker: self.resolve_agent_name(assignee, data),
                        });
                    }
                }
            }

            // Detect task blocked
            if *new_status == TaskStatus::Blocked {
                let was_blocked = self
                    .last_state
                    .tasks
                    .get(task_id)
                    .map(|(old_status, _)| *old_status == TaskStatus::Blocked)
                    .unwrap_or(false);

                if !was_blocked {
                    if let Some(assignee) = new_assignee {
                        if self.is_factory_agent(assignee, data) {
                            let task_title = task_info
                                .get(task_id.as_str())
                                .map(|t| t.title.clone())
                                .unwrap_or_default();

                            events.push(DirectorEvent::TaskBlocked {
                                task_id: task_id.clone(),
                                task_title,
                                worker: self.resolve_agent_name(assignee, data),
                            });
                        }
                    }
                }
            }
        }

        // Detect task completions (task disappeared from active sets).
        //
        // State-guard (cas-55dc): `task_completed_announced` is a per-session
        // HashSet that records every task_id for which TaskCompleted has been
        // emitted. The guard is NEVER cleared on active-set reappearance because
        // reappearance is the oscillation we are defending against: a task whose
        // lease expires and then is re-acquired temporarily disappears from and
        // reappears in the active sets, and without this guard every subsequent
        // disappearance would re-fire TaskCompleted (observed at ~30-second
        // intervals, the DEBOUNCE_DURATION). By recording the announcement at the
        // HashSet level (not the debounce map), the guard remains in force across
        // the debounce window and indefinitely thereafter.
        //
        // Genuine completions are not suppressed: the first time a task_id
        // disappears while InProgress the announcement fires; subsequent
        // disappearances for the same ID are no-ops.
        let completed_task_ids: Vec<(String, String, String)> = self
            .last_state
            .tasks
            .iter()
            .filter_map(|(task_id, (old_status, old_assignee))| {
                let removed_from_active_sets = !new_state.tasks.contains_key(task_id);
                if removed_from_active_sets
                    && *old_status == TaskStatus::InProgress
                    && !self.task_completed_announced.contains(task_id)
                {
                    if let Some(assignee) = old_assignee {
                        if self.is_factory_agent(assignee, data) {
                            let title = self
                                .last_state
                                .task_titles
                                .get(task_id)
                                .cloned()
                                .unwrap_or_default();
                            let worker = self.resolve_agent_name(assignee, data);
                            return Some((task_id.clone(), title, worker));
                        }
                    }
                }
                None
            })
            .collect();
        for (task_id, task_title, worker) in completed_task_ids {
            // Mark before pushing so the borrow on self.last_state is released.
            self.task_completed_announced.insert(task_id.clone());
            events.push(DirectorEvent::TaskCompleted {
                task_id,
                task_title,
                worker,
            });
        }

        // Detect idle workers using consecutive-tick debouncing.
        //
        // Previous logic emitted `WorkerIdle` the moment a worker transitioned
        // from having a task to having none. In practice that window is often
        // sub-second (worker closes task X, immediately calls `task start Y`),
        // and if the 2s director refresh landed inside the gap it emitted a
        // spurious idle prompt that the supervisor saw as "idle arrived before
        // the claim." See cas-f9e8.
        //
        // We now track how many consecutive refresh ticks each factory agent
        // has appeared idle and only emit once the count reaches
        // `IDLE_CONSECUTIVE_TICKS`. A single "has task" observation resets the
        // streak, so transient None states never accumulate. `idle_already_emitted`
        // prevents re-emission on every tick of a sustained idle streak; the
        // existing `IDLE_RATE_LIMIT` debounce at `debounce_events` handles the
        // cross-streak cooldown.
        let mut seen_factory_agents: HashSet<String> = HashSet::new();
        for agent in &data.agents {
            if !self.is_factory_agent(&agent.id, data) {
                continue;
            }

            // WorkerIdle must never fire for the supervisor / team-lead / primary
            // agent (cas-b67d). `is_factory_agent` deliberately includes the
            // supervisor so that task-assignment and completion events can
            // reference work done by the lead; but for idle tracking we only want
            // to surface genuine workers. A supervisor with current_task=None is
            // just waiting between decisions — not idle in the worker sense.
            let resolved_name = self.resolve_agent_name(&agent.id, data);
            if !self.is_worker_agent_name(&resolved_name) {
                continue;
            }

            seen_factory_agents.insert(agent.id.clone());

            if let Some(task_id) = &agent.current_task {
                // Agent is working — reset the idle streak. The next time this
                // agent's `current_task` goes to `None`, the counter starts
                // again from zero, which is exactly what we want: sustained idle
                // from THIS point on, not a stale count from an earlier streak.
                self.consecutive_idle_ticks.remove(&agent.id);
                self.idle_already_emitted.remove(&agent.id);

                // Stall detection (cas-9829): heartbeat alone cannot tell a
                // healthy in-progress worker from one that printed a plan and
                // stopped — a worker can heartbeat every tick while producing
                // zero tool calls/file edits/commits for the task it holds.
                // Require BOTH signals to diverge: a fresh heartbeat (the
                // worker process is alive) AND a `latest_activity` timestamp
                // older than `stall_threshold_secs` (it has genuinely gone
                // quiet, not just mid-turn). When either signal is absent
                // (no heartbeat data, no activity ever recorded) the gate
                // stays inactive rather than guessing.
                let has_fresh_heartbeat = agent
                    .last_heartbeat
                    .map(|hb| {
                        let age_secs = (now_utc - hb).num_seconds();
                        age_secs >= 0 && age_secs < FRESH_HEARTBEAT_SECS
                    })
                    .unwrap_or(false);
                let stalled_elapsed_secs = agent.latest_activity.as_ref().and_then(|(_, ts)| {
                    let age_secs = (now_utc - *ts).num_seconds();
                    (age_secs >= self.stall_threshold_secs as i64).then_some(age_secs)
                });

                if has_fresh_heartbeat {
                    if let Some(elapsed) = stalled_elapsed_secs {
                        if !self.stall_nudged.contains(&agent.id) {
                            // First detection in this streak: auto-nudge the
                            // worker (re-inject the task prompt) before
                            // paging the supervisor — a single re-poke often
                            // unsticks these (see bug report cas-9829).
                            events.push(DirectorEvent::WorkerStalled {
                                worker: resolved_name.clone(),
                                task_id: task_id.clone(),
                                elapsed_secs: elapsed as u64,
                                escalate: false,
                            });
                            self.stall_nudged.insert(agent.id.clone());
                        } else if !self.stall_escalated.contains(&agent.id) {
                            // Still stalled after the nudge — escalate to
                            // the supervisor.
                            events.push(DirectorEvent::WorkerStalled {
                                worker: resolved_name.clone(),
                                task_id: task_id.clone(),
                                elapsed_secs: elapsed as u64,
                                escalate: true,
                            });
                            self.stall_escalated.insert(agent.id.clone());
                        }
                    } else {
                        // Activity is fresh (or was never observed) — clear
                        // any prior streak so a future stall re-nudges from
                        // scratch instead of silently staying suppressed.
                        self.stall_nudged.remove(&agent.id);
                        self.stall_escalated.remove(&agent.id);
                    }
                }

                continue;
            }

            if agent.pending_messages > 0 {
                // Worker has unread messages in the prompt queue — don't count
                // this tick as idle. A freshly spawned worker appears task-less
                // before it has polled its first assignment; firing `WorkerIdle`
                // here would cause the supervisor to re-assign on top of the
                // queued message (spawn race, cas-afb7). Reset the streak so the
                // counter only starts accumulating after the queue is drained.
                self.consecutive_idle_ticks.remove(&agent.id);
                self.idle_already_emitted.remove(&agent.id);
                continue;
            }

            // Fresh-heartbeat + recent-activity gate (cas-4038).
            //
            // A CC agent sends heartbeats on every tool call. Between turns the
            // agent has `current_task = None` (no active lease) but is still
            // alive and may have uncommitted work. If the worker's heartbeat is
            // fresh AND it had recent activity, the current task-less state is
            // almost certainly a between-turns gap, not a genuine idle. Reset the
            // idle streak so WorkerIdle only fires after a truly sustained window
            // where BOTH signals are cold.
            //
            // The gate requires BOTH conditions (AND logic):
            //  - fresh heartbeat: a live worker always heartbeats; stale = dead/stalled
            //  - recent activity: guards against a worker that heartbeats as a
            //    daemon alive-check but hasn't actually done any work lately
            //
            // When either signal is absent (no heartbeat data, no activity) the
            // gate is inactive and normal consecutive-tick debounce governs.
            let has_fresh_heartbeat = agent
                .last_heartbeat
                .map(|hb| {
                    let age_secs = (now_utc - hb).num_seconds();
                    age_secs >= 0 && age_secs < FRESH_HEARTBEAT_SECS
                })
                .unwrap_or(false);
            let has_recent_activity = agent
                .latest_activity
                .as_ref()
                .map(|(_, ts)| {
                    let age_secs = (now_utc - *ts).num_seconds();
                    age_secs >= 0 && age_secs < RECENT_ACTIVITY_SECS
                })
                .unwrap_or(false);
            if has_fresh_heartbeat && has_recent_activity {
                // Worker is alive and recently active between turns — do not count
                // this tick and reset any partial idle streak so a genuine idle
                // that follows has to accumulate from zero.
                self.consecutive_idle_ticks.remove(&agent.id);
                continue;
            }

            let count = self
                .consecutive_idle_ticks
                .entry(agent.id.clone())
                .or_insert(0);
            *count += 1;

            if *count >= IDLE_CONSECUTIVE_TICKS
                && !self.idle_already_emitted.contains(&agent.id)
            {
                // `resolved_name` is guaranteed to be a worker (supervisor
                // was excluded above). Re-use it directly — no re-resolve.
                events.push(DirectorEvent::WorkerIdle {
                    worker: resolved_name.clone(),
                });
                self.idle_already_emitted.insert(agent.id.clone());
            }
        }

        // Stop tracking idle state for agents that have left the active set
        // (shutdown, crash, reassigned out of this factory). Without this the
        // maps would grow unbounded across long sessions.
        self.consecutive_idle_ticks
            .retain(|id, _| seen_factory_agents.contains(id));
        self.idle_already_emitted
            .retain(|id| seen_factory_agents.contains(id));
        self.stall_nudged
            .retain(|id| seen_factory_agents.contains(id));
        self.stall_escalated
            .retain(|id| seen_factory_agents.contains(id));

        // Detect new agent registrations
        for agent_id in &new_state.active_agents {
            if !self.last_state.active_agents.contains(agent_id) {
                let agent_name = self.resolve_agent_name(agent_id, data);
                if self.is_factory_agent_name(&agent_name) {
                    events.push(DirectorEvent::AgentRegistered {
                        agent_id: agent_id.clone(),
                        agent_name,
                    });
                }
            }
        }

        // Detect epic state changes
        // EpicStarted fires when:
        // 1. An epic transitions to InProgress (highest priority)
        // 2. A newly-appearing Open-with-branch epic is strictly better than
        //    the currently-active epic under the shared subtask-count
        //    heuristic. The picker and the init-time `detect_epic_state`
        //    share `pick_best_open_branch_epic` so they cannot diverge.
        {
            let mut in_progress_started: Option<(&str, &str)> = None;
            let mut saw_new_open_branch = false;

            for epic in &data.epic_tasks {
                if epic.status == TaskStatus::InProgress {
                    let was_in_progress = self
                        .last_state
                        .epic_statuses
                        .get(&epic.id)
                        .map(|(s, _)| *s == TaskStatus::InProgress)
                        .unwrap_or(false);

                    if !was_in_progress {
                        in_progress_started = Some((&epic.id, &epic.title));
                    }
                } else if epic.status == TaskStatus::Open && epic.branch.is_some() {
                    let was_open_with_branch = self
                        .last_state
                        .epic_statuses
                        .get(&epic.id)
                        .map(|(s, had_branch)| *s == TaskStatus::Open && *had_branch)
                        .unwrap_or(false);

                    if !was_open_with_branch {
                        saw_new_open_branch = true;
                    }
                }
            }

            // InProgress transitions always fire.
            if let Some((id, title)) = in_progress_started {
                events.push(DirectorEvent::EpicStarted {
                    epic_id: id.to_string(),
                    epic_title: title.to_string(),
                });
            } else if saw_new_open_branch {
                // Pick the best Open-with-branch epic using the shared
                // heuristic (subtasks, then lex ID). Applies the
                // strict-improvement gate when a current epic is known.
                if let Some(candidate) = pick_best_open_branch_epic(
                    &data.epic_tasks,
                    &data.in_progress_tasks,
                    &data.ready_tasks,
                ) {
                    // A tracked epic that has since been closed/deleted is
                        // treated as vacant so a legitimate new Open-with-branch
                        // epic can take over instead of the UI freezing on a
                        // ghost id (cas-4181 adversarial finding).
                        let cur_still_exists = current_epic_id
                            .map(|cur| data.epic_tasks.iter().any(|e| e.id == cur))
                            .unwrap_or(false);
                    let effective_current = if cur_still_exists {
                        current_epic_id
                    } else {
                        None
                    };
                    let should_fire = match effective_current {
                        // No active epic yet — any valid candidate wins.
                        None => true,
                        // Same epic already active — no change to announce.
                        Some(cur) if cur == candidate.id => false,
                        // Different epic — only announce if it is strictly
                        // better than the currently-active epic under the
                        // shared heuristic. A zero-subtask fresh epic cannot
                        // hijack an active one that has subtasks.
                        Some(cur) => {
                            let cand_score = open_branch_epic_score(
                                &candidate.id,
                                &data.in_progress_tasks,
                                &data.ready_tasks,
                            );
                            let cur_score = open_branch_epic_score(
                                cur,
                                &data.in_progress_tasks,
                                &data.ready_tasks,
                            );
                            cand_score > cur_score
                        }
                    };

                    if should_fire {
                        events.push(DirectorEvent::EpicStarted {
                            epic_id: candidate.id.clone(),
                            epic_title: candidate.title.clone(),
                        });
                    }
                }
            }
        }

        // EpicCompleted: Epic status changed to Closed
        for epic in &data.epic_tasks {
            if epic.status == TaskStatus::Closed {
                let was_closed = self
                    .last_state
                    .epic_statuses
                    .get(&epic.id)
                    .map(|(s, _)| *s == TaskStatus::Closed)
                    .unwrap_or(false);

                if !was_closed {
                    events.push(DirectorEvent::EpicCompleted {
                        epic_id: epic.id.clone(),
                    });
                }
            }
        }

        // EpicAllSubtasksClosed: All subtasks of a non-closed epic just became closed.
        // Detected when active subtask count drops to 0 from a previous count > 0.
        for epic in &data.epic_tasks {
            if epic.status != TaskStatus::Closed {
                let current_count = new_state
                    .epic_active_subtask_counts
                    .get(&epic.id)
                    .copied()
                    .unwrap_or(0);
                let previous_count = self
                    .last_state
                    .epic_active_subtask_counts
                    .get(&epic.id)
                    .copied()
                    .unwrap_or(0);

                if current_count == 0 && previous_count > 0 {
                    events.push(DirectorEvent::EpicAllSubtasksClosed {
                        epic_id: epic.id.clone(),
                        epic_title: epic.title.clone(),
                    });
                }
            }
        }

        // Update state for next comparison
        self.last_state = new_state;

        // Apply debouncing - filter out events emitted recently
        self.debounce_events(events, now)
    }

    /// Filter out events that were emitted recently (within debounce window)
    ///
    /// WorkerIdle events use a longer rate limit (5 minutes) to prevent flooding
    /// the supervisor when multiple workers idle simultaneously.
    /// Events from removed (shutdown/crashed) workers are suppressed entirely.
    fn debounce_events(&mut self, events: Vec<DirectorEvent>, now: Instant) -> Vec<DirectorEvent> {
        // Clean up old entries (use the longer idle rate limit as max TTL)
        self.last_prompt_times
            .retain(|_, time| now.duration_since(*time) < IDLE_RATE_LIMIT);

        // Filter events and update timestamps
        events
            .into_iter()
            .filter(|event| {
                // Suppress all events from removed (shutdown/crashed) workers
                if let Some(target) = event.target() {
                    if self.removed_workers.contains(target) {
                        return false;
                    }
                }

                let key = event.debounce_key();
                let window = if matches!(event, DirectorEvent::WorkerIdle { .. }) {
                    IDLE_RATE_LIMIT
                } else {
                    DEBOUNCE_DURATION
                };
                let should_emit = self
                    .last_prompt_times
                    .get(&key)
                    .map(|last_time| now.duration_since(*last_time) >= window)
                    .unwrap_or(true);

                if should_emit {
                    self.last_prompt_times.insert(key, now);
                }
                should_emit
            })
            .collect()
    }

    /// Check if an agent ID belongs to this factory session
    fn is_factory_agent(&self, agent_id: &str, data: &DirectorData) -> bool {
        // Resolve agent ID to name first
        let name = data
            .agent_id_to_name
            .get(agent_id)
            .map(|s| s.as_str())
            .unwrap_or(agent_id);

        // Check if name matches any worker or supervisor
        self.worker_names.contains(&name.to_string()) || name == self.supervisor_name
    }

    /// Check if an agent name belongs to this factory session
    fn is_factory_agent_name(&self, name: &str) -> bool {
        self.worker_names.contains(&name.to_string()) || name == self.supervisor_name
    }

    /// Check if an agent name is a **worker** in this factory session.
    ///
    /// Unlike `is_factory_agent_name`, this explicitly excludes the supervisor /
    /// primary agent. Use this wherever the intent is "this is one of MY workers"
    /// and the supervisor receiving the event would be wrong — e.g. the WorkerIdle
    /// path (cas-b67d / cas-c790).
    ///
    /// The explicit `!= supervisor_name` guard is defense-in-depth: even if the
    /// supervisor's name ends up in `worker_names` via a resume/reconnect path that
    /// doesn't go through `add_worker`, this check prevents a spurious WorkerIdle
    /// from propagating to the prompt layer.
    fn is_worker_agent_name(&self, name: &str) -> bool {
        self.worker_names.contains(&name.to_string()) && name != self.supervisor_name
    }

    /// Resolve agent ID to display name
    fn resolve_agent_name(&self, agent_id: &str, data: &DirectorData) -> String {
        data.agent_id_to_name
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| agent_id.to_string())
    }
}

/// Score an Open-with-branch epic by active-subtask counts.
///
/// Returns `(in_progress_count, ready_count)` for subtasks whose `epic`
/// field matches `epic_id`. The tuple compares lexicographically: an
/// epic with more in-progress subtasks always outranks one with fewer,
/// regardless of ready-count. Used by both the init-time picker and the
/// runtime EpicStarted strict-improvement gate.
pub(crate) fn open_branch_epic_score(
    epic_id: &str,
    in_progress_tasks: &[TaskSummary],
    ready_tasks: &[TaskSummary],
) -> (usize, usize) {
    let ip = in_progress_tasks
        .iter()
        .filter(|t| t.epic.as_deref() == Some(epic_id))
        .count();
    let ready = ready_tasks
        .iter()
        .filter(|t| t.epic.as_deref() == Some(epic_id))
        .count();
    (ip, ready)
}

/// Pick the best Open-with-branch epic from `epic_tasks` using the shared
/// heuristic: highest in-progress subtask count wins; then highest ready
/// subtask count; then lexicographically greatest ID as a deterministic
/// final tiebreak.
///
/// Used by both `ui::factory::app::detect_epic_state` (init-time epic
/// resolution) and `DirectorEventDetector::detect_changes` (runtime
/// `EpicStarted` detection) so the two paths cannot disagree on which
/// Open-with-branch epic should own the factory panel.
///
/// Returns `None` if no epic in `epic_tasks` is `Open` with a branch set.
pub(crate) fn pick_best_open_branch_epic<'a>(
    epic_tasks: &'a [TaskSummary],
    in_progress_tasks: &[TaskSummary],
    ready_tasks: &[TaskSummary],
) -> Option<&'a TaskSummary> {
    epic_tasks
        .iter()
        .filter(|e| e.status == TaskStatus::Open && e.branch.is_some())
        .max_by(|a, b| {
            let a_score = open_branch_epic_score(&a.id, in_progress_tasks, ready_tasks);
            let b_score = open_branch_epic_score(&b.id, in_progress_tasks, ready_tasks);
            a_score
                .cmp(&b_score)
                // Deterministic final tiebreak: greatest lex ID wins.
                .then_with(|| a.id.cmp(&b.id))
        })
}

#[cfg(test)]
#[path = "events_tests/tests.rs"]
mod tests;
