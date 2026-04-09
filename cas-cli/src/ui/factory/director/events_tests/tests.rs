use crate::ui::factory::director::data::AgentSummary;
use crate::ui::factory::director::events::*;
use cas_types::{AgentStatus, TaskType};

fn make_task(id: &str, title: &str, status: TaskStatus, assignee: Option<&str>) -> TaskSummary {
    TaskSummary {
        id: id.to_string(),
        title: title.to_string(),
        status,
        priority: cas_types::Priority::MEDIUM,
        assignee: assignee.map(String::from),
        task_type: TaskType::Task,
        epic: None,
        branch: None,
    }
}

fn make_epic(id: &str, title: &str, status: TaskStatus) -> TaskSummary {
    TaskSummary {
        id: id.to_string(),
        title: title.to_string(),
        status,
        priority: cas_types::Priority::HIGH,
        assignee: None,
        task_type: TaskType::Epic,
        epic: None,
        branch: None,
    }
}

fn make_epic_with_branch(id: &str, title: &str, status: TaskStatus, branch: &str) -> TaskSummary {
    TaskSummary {
        id: id.to_string(),
        title: title.to_string(),
        status,
        priority: cas_types::Priority::HIGH,
        assignee: None,
        task_type: TaskType::Epic,
        epic: None,
        branch: Some(branch.to_string()),
    }
}

fn make_agent(id: &str, name: &str, current_task: Option<&str>) -> AgentSummary {
    AgentSummary {
        id: id.to_string(),
        name: name.to_string(),
        status: AgentStatus::Active,
        current_task: current_task.map(String::from),
        latest_activity: None,
        last_heartbeat: Some(chrono::Utc::now()),
    }
}

#[test]
fn test_detect_task_assigned() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: task unassigned
    let data1 = DirectorData {
        ready_tasks: vec![make_task("task-1", "Test Task", TaskStatus::Open, None)],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", None)],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // New state: task assigned to swift-fox
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![make_task(
            "task-1",
            "Test Task",
            TaskStatus::InProgress,
            Some("agent-1"),
        )],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", Some("task-1"))],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    assert!(events.iter().any(|e| matches!(
        e,
        DirectorEvent::TaskAssigned { task_id, worker, .. }
            if task_id == "task-1" && worker == "swift-fox"
    )));
}

#[test]
fn test_detect_task_completed() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: task in progress
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![make_task(
            "task-1",
            "Test Task",
            TaskStatus::InProgress,
            Some("agent-1"),
        )],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", Some("task-1"))],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // New state: task completed (no longer in in_progress_tasks)
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", None)],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    assert!(events.iter().any(|e| matches!(
        e,
        DirectorEvent::TaskCompleted { task_id, worker, .. }
            if task_id == "task-1" && worker == "swift-fox"
    )));
}

/// Helper: build an idle-agent snapshot with a single factory worker.
fn idle_data_for(agent_id: &str, agent_name: &str) -> DirectorData {
    DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![make_agent(agent_id, agent_name, None)],
        activity: vec![],
        agent_id_to_name: [(agent_id.to_string(), agent_name.to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    }
}

/// Helper: build a working-agent snapshot with one task assigned to a single factory worker.
fn working_data_for(
    agent_id: &str,
    agent_name: &str,
    task_id: &str,
    task_title: &str,
) -> DirectorData {
    DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![make_task(
            task_id,
            task_title,
            TaskStatus::InProgress,
            Some(agent_id),
        )],
        epic_tasks: vec![],
        agents: vec![make_agent(agent_id, agent_name, Some(task_id))],
        activity: vec![],
        agent_id_to_name: [(agent_id.to_string(), agent_name.to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    }
}

#[test]
fn test_detect_worker_idle() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: worker has task.
    detector.initialize(&working_data_for("agent-1", "swift-fox", "task-1", "Test Task"));

    // Tick 1 of sustained idle — must NOT emit (debounce threshold is 2 ticks).
    let idle = idle_data_for("agent-1", "swift-fox");
    let events_tick1 = detector.detect_changes(&idle);
    assert!(
        !events_tick1
            .iter()
            .any(|e| matches!(e, DirectorEvent::WorkerIdle { .. })),
        "WorkerIdle must not fire after a single idle tick — debouncing prevents spurious \
         idle prompts during close-X → start-Y transitions"
    );

    // Tick 2 of sustained idle — now emit.
    let events_tick2 = detector.detect_changes(&idle);
    assert!(
        events_tick2.iter().any(|e| matches!(
            e,
            DirectorEvent::WorkerIdle { worker } if worker == "swift-fox"
        )),
        "WorkerIdle must fire once the agent has been idle for the consecutive-tick threshold"
    );

    // Tick 3 of sustained idle — already emitted, do not re-emit every tick.
    let events_tick3 = detector.detect_changes(&idle);
    assert!(
        !events_tick3
            .iter()
            .any(|e| matches!(e, DirectorEvent::WorkerIdle { .. })),
        "WorkerIdle must not re-fire on every tick of a sustained idle streak"
    );
}

/// Regression test for cas-f9e8.
///
/// Reproduces the close-X → start-Y race that caused spurious "Worker X is
/// idle" prompts: a worker finishes one task and immediately claims another,
/// but a single director refresh lands inside the sub-second window where
/// `agent_tasks[worker] = None`. Before the fix, the old transition-based
/// detector emitted `WorkerIdle` as soon as it observed that None, delivering
/// a stale "idle" prompt to the supervisor after the worker had already
/// resumed work. The consecutive-tick debounce must suppress this.
#[test]
fn test_no_worker_idle_on_transient_close_then_start() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initialize: worker is working on task-1.
    detector.initialize(&working_data_for(
        "agent-1",
        "swift-fox",
        "task-1",
        "First task",
    ));

    // Transient idle: one refresh tick catches the close-X → start-Y gap.
    let events_transient = detector.detect_changes(&idle_data_for("agent-1", "swift-fox"));
    assert!(
        !events_transient
            .iter()
            .any(|e| matches!(e, DirectorEvent::WorkerIdle { .. })),
        "WorkerIdle must not fire from a single transient idle tick — this is exactly the \
         close-X → start-Y race described in cas-f9e8"
    );

    // Next refresh: worker has claimed task-2. Idle streak should be reset.
    let events_claimed = detector.detect_changes(&working_data_for(
        "agent-1",
        "swift-fox",
        "task-2",
        "Second task",
    ));
    assert!(
        !events_claimed
            .iter()
            .any(|e| matches!(e, DirectorEvent::WorkerIdle { .. })),
        "WorkerIdle must not fire once the worker has resumed work on a new task"
    );

    // Sanity: a subsequent sustained idle still emits after the threshold,
    // so the reset didn't break normal idle detection.
    let idle = idle_data_for("agent-1", "swift-fox");
    let _ = detector.detect_changes(&idle); // tick 1
    let events_tick2 = detector.detect_changes(&idle); // tick 2
    assert!(
        events_tick2.iter().any(|e| matches!(
            e,
            DirectorEvent::WorkerIdle { worker } if worker == "swift-fox"
        )),
        "Sustained idle after a reset must still emit WorkerIdle once the threshold is met"
    );
}

#[test]
fn test_ignore_non_factory_agents() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state
    let data1 = DirectorData {
        ready_tasks: vec![make_task("task-1", "Test Task", TaskStatus::Open, None)],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // New state: task assigned to agent not in factory
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![make_task(
            "task-1",
            "Test Task",
            TaskStatus::InProgress,
            Some("other-agent"),
        )],
        epic_tasks: vec![],
        agents: vec![make_agent("other-agent", "other-agent", Some("task-1"))],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    // Should not detect assignment since "other-agent" is not in factory
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, DirectorEvent::TaskAssigned { .. }))
    );
}

#[test]
fn test_debouncing() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: task unassigned
    let data1 = DirectorData {
        ready_tasks: vec![make_task("task-1", "Test Task", TaskStatus::Open, None)],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", None)],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // First assignment - should emit event
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![make_task(
            "task-1",
            "Test Task",
            TaskStatus::InProgress,
            Some("agent-1"),
        )],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", Some("task-1"))],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events1 = detector.detect_changes(&data2);
    assert_eq!(events1.len(), 1);
    assert!(matches!(
        &events1[0],
        DirectorEvent::TaskAssigned { task_id, .. } if task_id == "task-1"
    ));

    // Re-initialize and try again immediately - should be debounced
    detector.last_state = DirectorState::from_data(&data1);
    let events2 = detector.detect_changes(&data2);
    assert!(
        events2.is_empty(),
        "Expected debounced event, but got {events2:?}"
    );
}

#[test]
fn test_debounce_key_uniqueness() {
    // Different event types should have different keys
    let assigned = DirectorEvent::TaskAssigned {
        task_id: "task-1".to_string(),
        task_title: "Title".to_string(),
        worker: "worker-1".to_string(),
    };
    let completed = DirectorEvent::TaskCompleted {
        task_id: "task-1".to_string(),
        task_title: "Title".to_string(),
        worker: "worker-1".to_string(),
    };

    assert_ne!(assigned.debounce_key(), completed.debounce_key());

    // Same event type with different tasks should have different keys
    let assigned2 = DirectorEvent::TaskAssigned {
        task_id: "task-2".to_string(),
        task_title: "Title".to_string(),
        worker: "worker-1".to_string(),
    };

    assert_ne!(assigned.debounce_key(), assigned2.debounce_key());

    // Same event type with same task but different worker should have different keys
    let assigned3 = DirectorEvent::TaskAssigned {
        task_id: "task-1".to_string(),
        task_title: "Title".to_string(),
        worker: "worker-2".to_string(),
    };

    assert_ne!(assigned.debounce_key(), assigned3.debounce_key());
}

#[test]
fn test_detect_epic_started() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: epic is open
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic("epic-1", "Test Epic", TaskStatus::Open)],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // New state: epic started (in progress)
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic("epic-1", "Test Epic", TaskStatus::InProgress)],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    assert!(events.iter().any(|e| matches!(
        e,
        DirectorEvent::EpicStarted { epic_id, epic_title }
            if epic_id == "epic-1" && epic_title == "Test Epic"
    )));
}

#[test]
fn test_detect_epic_completed() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: epic is in progress
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic("epic-1", "Test Epic", TaskStatus::InProgress)],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // New state: epic completed
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic("epic-1", "Test Epic", TaskStatus::Closed)],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    assert!(events.iter().any(|e| matches!(
        e,
        DirectorEvent::EpicCompleted { epic_id } if epic_id == "epic-1"
    )));
}

#[test]
fn test_no_epic_event_when_unchanged() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: epic is in progress
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic("epic-1", "Test Epic", TaskStatus::InProgress)],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // Same state: epic still in progress
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic("epic-1", "Test Epic", TaskStatus::InProgress)],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    // No epic events should be emitted
    assert!(!events.iter().any(|e| matches!(
        e,
        DirectorEvent::EpicStarted { .. } | DirectorEvent::EpicCompleted { .. }
    )));
}

#[test]
fn test_idle_events_suppressed_for_removed_workers() {
    let mut detector = DirectorEventDetector::new(
        vec!["swift-fox".to_string(), "calm-owl".to_string()],
        "supervisor".to_string(),
    );

    // Initial state: both workers have tasks
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![
            make_task("task-1", "Task 1", TaskStatus::InProgress, Some("agent-1")),
            make_task("task-2", "Task 2", TaskStatus::InProgress, Some("agent-2")),
        ],
        epic_tasks: vec![],
        agents: vec![
            make_agent("agent-1", "swift-fox", Some("task-1")),
            make_agent("agent-2", "calm-owl", Some("task-2")),
        ],
        activity: vec![],
        agent_id_to_name: [
            ("agent-1".to_string(), "swift-fox".to_string()),
            ("agent-2".to_string(), "calm-owl".to_string()),
        ]
        .into_iter()
        .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // Shut down swift-fox
    detector.remove_worker("swift-fox");

    // New state: both workers idle (swift-fox's agent might still linger in data)
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![
            make_agent("agent-1", "swift-fox", None),
            make_agent("agent-2", "calm-owl", None),
        ],
        activity: vec![],
        agent_id_to_name: [
            ("agent-1".to_string(), "swift-fox".to_string()),
            ("agent-2".to_string(), "calm-owl".to_string()),
        ]
        .into_iter()
        .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    // Two idle ticks are required to cross the consecutive-tick debounce.
    let _ = detector.detect_changes(&data2);
    let events = detector.detect_changes(&data2);

    // calm-owl idle event should be emitted
    assert!(
        events.iter().any(|e| matches!(
            e,
            DirectorEvent::WorkerIdle { worker } if worker == "calm-owl"
        )),
        "Expected idle event for calm-owl"
    );

    // swift-fox idle event should be suppressed (removed worker)
    assert!(
        !events.iter().any(|e| matches!(
            e,
            DirectorEvent::WorkerIdle { worker } if worker == "swift-fox"
        )),
        "Expected no idle event for removed worker swift-fox"
    );
}

#[test]
fn test_idle_rate_limit_longer_than_general_debounce() {
    use std::time::Duration;

    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: worker has task
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![make_task(
            "task-1",
            "Test Task",
            TaskStatus::InProgress,
            Some("agent-1"),
        )],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", Some("task-1"))],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // Worker goes idle - first event should emit
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![make_agent("agent-1", "swift-fox", None)],
        activity: vec![],
        agent_id_to_name: [("agent-1".to_string(), "swift-fox".to_string())]
            .into_iter()
            .collect(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    // Two idle ticks are required to cross the consecutive-tick debounce.
    let _ = detector.detect_changes(&data2);
    let events = detector.detect_changes(&data2);
    assert!(
        events.iter().any(|e| matches!(
            e,
            DirectorEvent::WorkerIdle { worker } if worker == "swift-fox"
        )),
        "First idle event should emit"
    );

    // Simulate: worker gets task and goes idle again after 60 seconds
    // (past the 30s general debounce but within the 5-minute idle rate limit).
    // We drive this through detect_changes rather than poking last_state directly
    // so the consecutive-idle counters reset the way they would in production.
    let _ = detector.detect_changes(&data1);

    // Manually advance the idle debounce time to 60s ago (past 30s general debounce)
    let key = "idle:swift-fox".to_string();
    if let Some(time) = detector.last_prompt_times.get_mut(&key) {
        *time = std::time::Instant::now() - Duration::from_secs(60);
    }

    let events2 = detector.detect_changes(&data2);
    assert!(
        !events2.iter().any(|e| matches!(
            e,
            DirectorEvent::WorkerIdle { worker } if worker == "swift-fox"
        )),
        "Idle event should be rate-limited (within 5-minute window)"
    );
}

#[test]
fn test_detect_epic_started_open_with_branch() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: no epics
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // New state: an Open epic with a branch appears (auto-created by supervisor)
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic_with_branch(
            "epic-1",
            "New Epic",
            TaskStatus::Open,
            "epic/new-epic",
        )],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    assert!(
        events.iter().any(|e| matches!(
            e,
            DirectorEvent::EpicStarted { epic_id, epic_title }
                if epic_id == "epic-1" && epic_title == "New Epic"
        )),
        "Open-with-branch epic should fire EpicStarted"
    );
}

#[test]
fn test_no_duplicate_epic_started_for_existing_open_with_branch() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: already has an Open epic with branch
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic_with_branch(
            "epic-1",
            "Existing Epic",
            TaskStatus::Open,
            "epic/existing",
        )],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // Same state: epic still Open with branch
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![make_epic_with_branch(
            "epic-1",
            "Existing Epic",
            TaskStatus::Open,
            "epic/existing",
        )],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    assert!(
        !events.iter().any(|e| matches!(
            e,
            DirectorEvent::EpicStarted { .. }
        )),
        "Should not fire EpicStarted for already-tracked Open-with-branch epic"
    );
}

#[test]
fn test_in_progress_epic_takes_priority_over_open_with_branch() {
    let mut detector =
        DirectorEventDetector::new(vec!["swift-fox".to_string()], "supervisor".to_string());

    // Initial state: no epics
    let data1 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };
    detector.initialize(&data1);

    // Both an Open-with-branch and an InProgress epic appear
    let data2 = DirectorData {
        ready_tasks: vec![],
        in_progress_tasks: vec![],
        epic_tasks: vec![
            make_epic_with_branch("epic-open", "Open Epic", TaskStatus::Open, "epic/open"),
            make_epic("epic-active", "Active Epic", TaskStatus::InProgress),
        ],
        agents: vec![],
        activity: vec![],
        agent_id_to_name: HashMap::new(),
        changes: vec![],
        git_loaded: true,
        reminders: vec![],
        epic_closed_counts: HashMap::new(),
    };

    let events = detector.detect_changes(&data2);

    // InProgress should win
    assert!(
        events.iter().any(|e| matches!(
            e,
            DirectorEvent::EpicStarted { epic_id, .. } if epic_id == "epic-active"
        )),
        "InProgress epic should take priority over Open-with-branch"
    );
}
