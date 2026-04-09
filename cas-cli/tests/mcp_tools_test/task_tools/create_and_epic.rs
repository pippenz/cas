use crate::support::*;
use cas::mcp::tools::*;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;

#[tokio::test]
async fn test_task_create_basic() {
    let (_temp, service) = setup_cas();

    let req = TaskCreateRequest {
        title: "Test task".to_string(),
        description: Some("Task description".to_string()),
        priority: 2,
        task_type: "task".to_string(),
        labels: Some("test,task".to_string()),
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        execution_note: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let text = extract_text(result);
    assert!(text.contains("Created task"));
    assert!(text.contains("Test task"));
}

#[tokio::test]
async fn test_task_create_and_start() {
    let (_temp, service) = setup_cas();

    // Create task
    let req = TaskCreateRequest {
        title: "Auto-start task".to_string(),
        description: None,
        priority: 1,
        task_type: "feature".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        execution_note: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let text = extract_text(result);
    assert!(text.contains("Created"));
    let task_id = extract_task_id(&text).expect("should have task ID");

    // Start the task separately using the start action
    let start_req = IdRequest {
        id: task_id.to_string(),
    };
    let start_result = service
        .cas_task_start(Parameters(start_req))
        .await
        .expect("task_start should succeed");

    let start_text = extract_text(start_result);
    // After starting, the output includes claim info (e.g., "claimed until HH:MM")
    assert!(
        start_text.contains("claimed"),
        "Task start should show claimed: {start_text}"
    );
    // Workflow guidance should be included when starting a task
    assert!(
        start_text.contains("Workflow Guidance"),
        "Task start should include workflow guidance: {start_text}"
    );
    assert!(
        start_text.contains("mcp__cas__search"),
        "Workflow guidance should mention CAS search: {start_text}"
    );
}

/// Test that epic creation creates a branch, not a worktree
///
/// This is a regression test for the bug where supervisors were getting
/// worktrees when creating epics. Epics should only get branches.
#[tokio::test]
async fn test_epic_creates_branch_not_worktree() {
    use std::process::Command;

    let (temp, service) = setup_cas();

    // Initialize git repo (required for branch creation)
    Command::new("git")
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to init git");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Create initial commit (required for branch creation)
    std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(temp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Create epic task
    let req = TaskCreateRequest {
        title: "Add User Authentication".to_string(),
        description: Some("Epic for auth feature".to_string()),
        priority: 1,
        task_type: "epic".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        execution_note: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let create_text = extract_text(result);
    let epic_id = extract_task_id(&create_text).expect("should have epic ID");
    let expected_branch = format!("epic/add-user-authentication-{epic_id}");

    // Should contain branch info, not worktree info
    assert!(
        create_text.contains("Epic branch created") || create_text.contains("epic/"),
        "Epic should create branch on create: {create_text}"
    );
    assert!(
        !create_text.contains("Worktree created"),
        "Epic should NOT create worktree: {create_text}"
    );

    // Start the epic (which triggers branch creation)
    let start_req = IdRequest {
        id: epic_id.to_string(),
    };
    let start_result = service
        .cas_task_start(Parameters(start_req))
        .await
        .expect("task_start should succeed");

    let text = extract_text(start_result);
    println!("Epic start output: {text}");

    // Start should not create a worktree for epics
    assert!(
        !text.contains("Worktree created"),
        "Epic should NOT create worktree: {text}"
    );

    // Verify git branch was created
    let branch_list = Command::new("git")
        .args(["branch", "--list", "epic/*"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to list branches");

    let branches = String::from_utf8_lossy(&branch_list.stdout);
    println!("Git branches: {branches}");
    assert!(
        branches.contains(&expected_branch),
        "Expected {expected_branch} branch, got: {branches}"
    );

    // Verify no worktree directory was created
    let worktree_dir = temp.path().parent().unwrap().join(format!(
        "{}-worktrees",
        temp.path().file_name().unwrap().to_str().unwrap()
    ));
    assert!(
        !worktree_dir.exists(),
        "Worktree directory should not exist"
    );
}

#[tokio::test]
async fn test_task_create_invalid_epic_does_not_persist_task() {
    let (_temp, service) = setup_cas();

    let req = TaskCreateRequest {
        title: "Should fail atomic create".to_string(),
        description: Some("invalid epic should not leave orphan task".to_string()),
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        execution_note: None,
        epic: Some("cas-does-not-exist".to_string()),
    };

    let result = service.cas_task_create(Parameters(req)).await;
    assert!(result.is_err(), "Create should fail for invalid epic");

    let list_req = TaskListRequest {
        scope: "all".to_string(),
        limit: Some(20),
        status: None,
        task_type: None,
        label: None,
        assignee: None,
        epic: None,
        sort: None,
        sort_order: None,
    };
    let list_result = service
        .cas_task_list(Parameters(list_req))
        .await
        .expect("task_list should succeed");
    let text = extract_text(list_result);
    assert!(
        text.contains("No tasks found matching filters"),
        "Task create should be atomic; unexpected task list output: {text}"
    );
}

#[tokio::test]
async fn test_task_create_surfaces_dependency_write_failure() {
    let (temp, service) = setup_cas();

    let blocker = service
        .cas_task_create(Parameters(TaskCreateRequest {
            title: "Blocking task".to_string(),
            description: None,
            priority: 2,
            task_type: "task".to_string(),
            labels: None,
            notes: None,
            blocked_by: None,
            design: None,
            acceptance_criteria: None,
            external_ref: None,
            assignee: None,
            demo_statement: None,
            execution_note: None,
            epic: None,
        }))
        .await
        .expect("blocker create should succeed");
    let blocker_id = extract_task_id(&extract_text(blocker))
        .expect("blocker id")
        .to_string();

    let db_path = temp.path().join(".cas").join("cas.db");
    let conn = Connection::open(&db_path).expect("open sqlite db");
    conn.execute(
        "CREATE TRIGGER fail_dependency_insert
         BEFORE INSERT ON dependencies
         BEGIN
             SELECT RAISE(FAIL, 'forced dependency insert failure');
         END;",
        [],
    )
    .expect("create insert failure trigger");

    let create_result = service
        .cas_task_create(Parameters(TaskCreateRequest {
            title: "Should fail dependency write".to_string(),
            description: None,
            priority: 2,
            task_type: "task".to_string(),
            labels: None,
            notes: None,
            blocked_by: Some(blocker_id),
            design: None,
            acceptance_criteria: None,
            external_ref: None,
            assignee: None,
            demo_statement: None,
            execution_note: None,
            epic: None,
        }))
        .await;
    assert!(
        create_result.is_err(),
        "Dependency write failure should be returned to caller"
    );

    let list_text = extract_text(
        service
            .cas_task_list(Parameters(TaskListRequest {
                scope: "all".to_string(),
                limit: Some(20),
                status: None,
                task_type: None,
                label: None,
                assignee: None,
                epic: None,
                sort: None,
                sort_order: None,
            }))
            .await
            .expect("task_list should succeed"),
    );
    assert!(
        !list_text.contains("Should fail dependency write"),
        "create_atomic should roll back task on dependency insert error: {list_text}"
    );
}
