//! Event detection for the Director
//!
//! Detects state changes in CAS data by comparing snapshots.
//! Used to trigger auto-prompting and activity logging.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::ui::factory::director::data::{DirectorData, TaskSummary};
use cas_types::TaskStatus;

/// Debounce duration for events (don't emit same event within this window)
const DEBOUNCE_DURATION: Duration = Duration::from_secs(30);

/// Rate limit for WorkerIdle events — at most one per worker per 5 minutes.
/// Idle notifications are low-priority and flood the supervisor when multiple
/// workers idle simultaneously.
const IDLE_RATE_LIMIT: Duration = Duration::from_secs(300);

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
    /// A new agent registered
    AgentRegistered {
        agent_id: String,
        agent_name: String,
    },
    /// An epic was started (detected by new epic-type task)
    EpicStarted { epic_id: String, epic_title: String },
    /// All tasks in an epic are complete
    EpicCompleted { epic_id: String },
}

impl DirectorEvent {
    /// Get the worker/agent this event targets (for prompt injection)
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::TaskAssigned { worker, .. } => Some(worker),
            Self::TaskCompleted { worker, .. } => Some(worker),
            Self::TaskBlocked { worker, .. } => Some(worker),
            Self::WorkerIdle { worker } => Some(worker),
            Self::AgentRegistered { agent_name, .. } => Some(agent_name),
            Self::EpicStarted { .. } => None, // Broadcast or supervisor
            Self::EpicCompleted { .. } => None,
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
            Self::AgentRegistered { agent_id, .. } => {
                format!("registered:{agent_id}")
            }
            Self::EpicStarted { epic_id, .. } => {
                format!("epic_started:{epic_id}")
            }
            Self::EpicCompleted { epic_id } => {
                format!("epic_completed:{epic_id}")
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
            Self::AgentRegistered { .. } => "agent_registered",
            Self::EpicStarted { .. } => "epic_started",
            Self::EpicCompleted { .. } => "epic_completed",
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
    /// Map of agent_id -> current_task_id
    agent_tasks: HashMap<String, Option<String>>,
    /// Map of epic_id -> (status, has_branch)
    epic_statuses: HashMap<String, (TaskStatus, bool)>,
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

        let agent_tasks: HashMap<String, Option<String>> = data
            .agents
            .iter()
            .map(|a| (a.id.clone(), a.current_task.clone()))
            .collect();

        // Track epic statuses and branch presence
        let epic_statuses: HashMap<String, (TaskStatus, bool)> = data
            .epic_tasks
            .iter()
            .map(|e| (e.id.clone(), (e.status, e.branch.is_some())))
            .collect();

        Self {
            tasks,
            task_titles,
            active_agents,
            agent_tasks,
            epic_statuses,
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
        }
    }

    /// Initialize with current state (call after first data load)
    pub fn initialize(&mut self, data: &DirectorData) {
        self.last_state = DirectorState::from_data(data);
    }

    /// Add a worker to the tracked list (call when spawning workers dynamically)
    pub fn add_worker(&mut self, name: String) {
        if !self.worker_names.contains(&name) {
            self.worker_names.push(name);
        }
    }

    /// Remove a worker from the tracked list (call when shutting down workers)
    pub fn remove_worker(&mut self, name: &str) {
        self.worker_names.retain(|n| n != name);
        self.removed_workers.insert(name.to_string());
    }

    /// Detect changes between the last state and new data
    ///
    /// Returns a list of detected events. Call this after each refresh.
    pub fn detect_changes(&mut self, data: &DirectorData) -> Vec<DirectorEvent> {
        let now = Instant::now();
        let new_state = DirectorState::from_data(data);
        let mut events = Vec::new();

        // Build lookup maps for task info
        let task_info: HashMap<&str, &TaskSummary> = data
            .ready_tasks
            .iter()
            .chain(data.in_progress_tasks.iter())
            .map(|t| (t.id.as_str(), t))
            .collect();

        // Detect task assignments (task now has assignee that it didn't before)
        for (task_id, (new_status, new_assignee)) in &new_state.tasks {
            if let Some(assignee) = new_assignee {
                // Check if this is a new assignment
                let was_assigned = self
                    .last_state
                    .tasks
                    .get(task_id)
                    .map(|(_, old_assignee)| old_assignee.as_ref() == Some(assignee))
                    .unwrap_or(false);

                if !was_assigned && self.is_factory_agent(assignee, data) {
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

        // Detect task completions (task disappeared from active sets)
        for (task_id, (old_status, old_assignee)) in &self.last_state.tasks {
            let removed_from_active_sets = !new_state.tasks.contains_key(task_id);
            if removed_from_active_sets {
                // Only emit TaskCompleted for tasks that were actively being worked on
                if *old_status == TaskStatus::InProgress {
                    if let Some(assignee) = old_assignee {
                        if self.is_factory_agent(assignee, data) {
                            events.push(DirectorEvent::TaskCompleted {
                                task_id: task_id.clone(),
                                task_title: self
                                    .last_state
                                    .task_titles
                                    .get(task_id)
                                    .cloned()
                                    .unwrap_or_default(),
                                worker: self.resolve_agent_name(assignee, data),
                            });
                        }
                    }
                }
            }
        }

        // Detect idle workers (agent had a task, now doesn't)
        for (agent_id, old_task) in &self.last_state.agent_tasks {
            if old_task.is_some() {
                let new_task = new_state.agent_tasks.get(agent_id);
                if new_task.map(|t| t.is_none()).unwrap_or(true) {
                    // Agent had task, now doesn't
                    let agent_name = self.resolve_agent_name(agent_id, data);
                    if self.is_factory_agent_name(&agent_name) {
                        events.push(DirectorEvent::WorkerIdle { worker: agent_name });
                    }
                }
            }
        }

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
        // 2. A new Open-with-branch epic appears (mirrors detect_epic_state init logic)
        //
        // When multiple epics qualify, later entries in the list win (typically newer).
        {
            let mut epic_started: Option<(&str, &str)> = None;

            for epic in &data.epic_tasks {
                if epic.status == TaskStatus::InProgress {
                    let was_in_progress = self
                        .last_state
                        .epic_statuses
                        .get(&epic.id)
                        .map(|(s, _)| *s == TaskStatus::InProgress)
                        .unwrap_or(false);

                    if !was_in_progress {
                        epic_started = Some((&epic.id, &epic.title));
                    }
                } else if epic.status == TaskStatus::Open && epic.branch.is_some() {
                    // New Open-with-branch epic that wasn't previously tracked with a branch
                    let was_open_with_branch = self
                        .last_state
                        .epic_statuses
                        .get(&epic.id)
                        .map(|(s, had_branch)| *s == TaskStatus::Open && *had_branch)
                        .unwrap_or(false);

                    if !was_open_with_branch {
                        // Only pick Open-with-branch if no InProgress epic already selected
                        if epic_started.is_none() {
                            epic_started = Some((&epic.id, &epic.title));
                        }
                    }
                }
            }

            if let Some((id, title)) = epic_started {
                events.push(DirectorEvent::EpicStarted {
                    epic_id: id.to_string(),
                    epic_title: title.to_string(),
                });
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

    /// Resolve agent ID to display name
    fn resolve_agent_name(&self, agent_id: &str, data: &DirectorData) -> String {
        data.agent_id_to_name
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| agent_id.to_string())
    }
}

#[cfg(test)]
#[path = "events_tests/tests.rs"]
mod tests;
