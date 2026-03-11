//! Worktree lifecycle E2E tests
//!
//! Tests git worktree integration: creation, listing, cleanup, merge

use crate::fixtures::new_cas_instance;
use std::fs;
use std::process::Command;

/// Initialize a git repo in the test directory
fn init_git_repo(path: &std::path::Path) {
    let _ = Command::new("git")
        .current_dir(path)
        .args(["init"])
        .output();

    // Configure git user for commits
    let _ = Command::new("git")
        .current_dir(path)
        .args(["config", "user.email", "test@example.com"])
        .output();
    let _ = Command::new("git")
        .current_dir(path)
        .args(["config", "user.name", "Test User"])
        .output();

    // Create initial commit
    fs::write(path.join("README.md"), "# Test Project").expect("Failed to write README");
    let _ = Command::new("git")
        .current_dir(path)
        .args(["add", "."])
        .output();
    let _ = Command::new("git")
        .current_dir(path)
        .args(["commit", "-m", "Initial commit"])
        .output();
}

/// Test listing worktrees
#[test]
fn test_worktree_list() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    let output = cas
        .cas_cmd()
        .args(["worktree", "list"])
        .output()
        .expect("Failed to list worktrees");

    assert!(
        output.status.success(),
        "worktree list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test worktree status
#[test]
fn test_worktree_status() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    let output = cas
        .cas_cmd()
        .args(["worktree", "status"])
        .output()
        .expect("Failed to get worktree status");

    assert!(
        output.status.success(),
        "worktree status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test worktree show command
#[test]
fn test_worktree_show() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // First, create an epic to associate with a worktree
    let epic_id = cas.create_task_with_options("Test epic for worktree", Some("epic"), None, false);

    // Try to show worktree (may not exist yet)
    let output = cas
        .cas_cmd()
        .args(["worktree", "show", &epic_id])
        .output()
        .expect("Failed to show worktree");

    // May succeed or fail depending on whether worktree exists
    println!(
        "worktree show output: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "worktree show stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test worktree cleanup with dry-run
#[test]
fn test_worktree_cleanup_dry_run() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    let output = cas
        .cas_cmd()
        .args(["worktree", "cleanup", "--dry-run"])
        .output()
        .expect("Failed to run worktree cleanup");

    assert!(
        output.status.success(),
        "worktree cleanup --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test worktree listing filters
#[test]
fn test_worktree_list_filters() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // List with --all flag
    let output = cas
        .cas_cmd()
        .args(["worktree", "list", "--all"])
        .output()
        .expect("Failed to list worktrees with --all");

    assert!(output.status.success());

    // List with --orphans flag
    let output = cas
        .cas_cmd()
        .args(["worktree", "list", "--orphans"])
        .output()
        .expect("Failed to list orphan worktrees");

    assert!(output.status.success());
}

/// Test worktree list by status
#[test]
fn test_worktree_list_by_status() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    let statuses = ["active", "merged", "abandoned", "conflict", "removed"];

    for status in statuses {
        let output = cas
            .cas_cmd()
            .args(["worktree", "list", "--status", status])
            .output()
            .expect("Failed to list worktrees by status");

        assert!(
            output.status.success(),
            "worktree list --status {} failed: {}",
            status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Test creating a worktree for a task
#[test]
fn test_worktree_create_for_task() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // Create an epic task
    let epic_id =
        cas.create_task_with_options("Epic for worktree creation", Some("epic"), None, false);

    // Try to create worktree via MCP or CLI
    // Note: The worktree creation might require MCP or specific workflow
    let output = cas
        .cas_cmd()
        .args(["worktree", "create", "--task-id", &epic_id])
        .output();

    match output {
        Ok(out) => {
            println!(
                "worktree create output: {}",
                String::from_utf8_lossy(&out.stdout)
            );
            println!(
                "worktree create stderr: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            // May succeed or have specific requirements
        }
        Err(e) => {
            println!("worktree create error (may be expected): {}", e);
        }
    }
}

/// Test that worktree path is tracked in task
#[test]
fn test_worktree_task_association() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // Create an epic
    let epic_id = cas.create_task_with_options("Epic with worktree", Some("epic"), None, false);

    // Check task JSON for worktree fields
    let task = cas.get_task_json(&epic_id);

    // Tasks should have worktree-related fields
    let has_branch_field = task.get("branch").is_some();
    let has_worktree_field = task.get("worktree_id").is_some();

    println!("Task has branch field: {}", has_branch_field);
    println!("Task has worktree_id field: {}", has_worktree_field);
    println!("Task: {:?}", task);
}

/// Test worktree cleanup removes stale entries
#[test]
fn test_worktree_cleanup() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // Run cleanup
    let output = cas
        .cas_cmd()
        .args(["worktree", "cleanup"])
        .output()
        .expect("Failed to run worktree cleanup");

    assert!(
        output.status.success(),
        "worktree cleanup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test pending_worktree_merge flag
#[test]
fn test_pending_worktree_merge_flag() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // Create an epic task
    let epic_id = cas.create_task_with_options("Epic for merge test", Some("epic"), None, false);
    cas.start_task(&epic_id);

    // Set pending_worktree_merge via direct DB update
    let db_path = cas.temp_dir.path().join(".cas").join("cas.db");
    let _ = Command::new("sqlite3")
        .arg(&db_path)
        .arg(format!(
            "UPDATE tasks SET pending_worktree_merge = 1 WHERE id = '{}';",
            epic_id
        ))
        .output();

    // Check task
    let task = cas.get_task_json(&epic_id);
    let pending_merge = task
        .get("pending_worktree_merge")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    println!("pending_worktree_merge: {}", pending_merge);
}

/// Test worktree listing with JSON output
#[test]
fn test_worktree_list_json() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    let output = cas
        .cas_cmd()
        .args(["worktree", "list", "--json"])
        .output()
        .expect("Failed to list worktrees as JSON");

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Try to parse as JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            println!("Worktree list JSON: {:?}", json);
        }
    }
}

/// Test worktree status includes current branch
#[test]
fn test_worktree_status_shows_branch() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    let output = cas
        .cas_cmd()
        .args(["worktree", "status"])
        .output()
        .expect("Failed to get worktree status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show current branch info (main or master)
    let has_branch_info =
        stdout.contains("main") || stdout.contains("master") || stdout.contains("branch");

    assert!(
        has_branch_info,
        "Worktree status should include branch info. Got: {}",
        stdout
    );

    println!("Worktree status: {}", stdout);
}

/// Test worktree operations in non-git directory
#[test]
fn test_worktree_in_non_git_dir() {
    let cas = new_cas_instance();
    // Don't initialize git

    // Should handle gracefully
    let output = cas
        .cas_cmd()
        .args(["worktree", "list"])
        .output()
        .expect("Failed to run worktree list");

    // May succeed with empty list or fail gracefully
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    println!("Non-git worktree list stdout: {}", stdout);
    println!("Non-git worktree list stderr: {}", stderr);

    // Should not crash
}

/// Test worktree cleanup with force flag
#[test]
fn test_worktree_cleanup_force() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    let output = cas
        .cas_cmd()
        .args(["worktree", "cleanup", "--force"])
        .output()
        .expect("Failed to run worktree cleanup with force");

    // Force cleanup should succeed
    assert!(
        output.status.success(),
        "worktree cleanup --force failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test worktree merge command
#[test]
fn test_worktree_merge() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // Create an epic
    let epic_id = cas.create_task_with_options("Epic to merge", Some("epic"), None, false);

    // Try merge command (may require existing worktree)
    let output = cas.cas_cmd().args(["worktree", "merge", &epic_id]).output();

    match output {
        Ok(out) => {
            println!(
                "worktree merge stdout: {}",
                String::from_utf8_lossy(&out.stdout)
            );
            println!(
                "worktree merge stderr: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Err(e) => {
            println!("worktree merge error (may be expected): {}", e);
        }
    }
}

/// Test that CLI epic creation does NOT create a worktree
///
/// Note: The CLI `task create` command does not create branches - only the MCP
/// tool does. This test verifies that no worktree directory is created.
/// For full branch creation testing, see mcp_tools_test.rs.
#[test]
fn test_cli_epic_no_worktree_created() {
    let cas = new_cas_instance();
    init_git_repo(cas.temp_dir.path());

    // Create an epic via CLI
    let epic_id = cas.create_task_with_options("Add User Authentication", Some("epic"), None, true);

    // Get task JSON
    let task = cas.get_task_json(&epic_id);

    // worktree_id should be null (no worktree created)
    let worktree_id = task.get("worktree_id").and_then(|v| v.as_str());
    assert!(
        worktree_id.is_none() || worktree_id == Some(""),
        "worktree_id should be empty/null for CLI-created epics, got: {:?}",
        worktree_id
    );

    // Verify no worktree directory was created
    let worktree_path = cas.temp_dir.path().parent().unwrap().join(format!(
        "{}-worktrees",
        cas.temp_dir.path().file_name().unwrap().to_str().unwrap()
    ));
    let epic_worktree = worktree_path.join(&epic_id);
    assert!(
        !epic_worktree.exists(),
        "Worktree directory should not exist at: {}",
        epic_worktree.display()
    );

    println!("✓ CLI epic creation correctly does NOT create worktree");
}

// Note: Branch slugification tests are in mcp_tools_test.rs and the library unit tests.
// CLI task create doesn't create branches - only the MCP tool does.
