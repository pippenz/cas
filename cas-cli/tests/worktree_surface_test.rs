//! Tests for A3: Truthful worktree status surface (cas-af86)
//!
//! Verifies that `worktree_list` and `worktree_status` accurately report factory
//! (System B) worktrees even when the CAS experimental worktree system (System A)
//! is disabled via config (`worktrees.enabled = false`).
//!
//! Prior to the fix, the gate in `mcp/tools/service/mod.rs` short-circuited
//! `worktree_list` with a misleading "experimental and disabled by default"
//! message whenever System A was off — even though factory workers were running
//! in real git worktrees under `.cas/worktrees/<name>`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cas::mcp::{CasCore, CasService};
use cas::store::init_cas_dir;
use cas_mcp::types::CoordinationRequest;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::RawContent;
use tempfile::TempDir;

// =============================================================================
// Test fixtures
// =============================================================================

struct GitRepo {
    _temp: TempDir,
    pub root: PathBuf,
}

impl GitRepo {
    fn new() -> Self {
        let temp = TempDir::new().expect("TempDir");
        let root = temp.path().to_path_buf();

        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .expect("git");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };

        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "test@test.com"]);
        run(&["config", "user.name", "Test"]);
        std::fs::write(root.join("README.md"), "test").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);

        Self { _temp: temp, root }
    }

    /// Create a git worktree at `path` on a new branch `branch`.
    /// The parent directory of `path` is created if needed; git creates `path` itself.
    fn add_worktree(&self, path: &Path, branch: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let out = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                branch,
                path.to_str().unwrap(),
            ])
            .current_dir(&self.root)
            .output()
            .expect("git worktree add");
        assert!(
            out.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn make_service(cas_root: PathBuf) -> CasService {
    let core = CasCore::with_daemon(cas_root, None, None);
    CasService::new(core, None)
}

/// Overwrite the config with `[worktrees] enabled = false` to simulate
/// a deployment where System A (experimental) is explicitly off.
fn disable_system_a(cas_root: &Path) {
    std::fs::write(
        cas_root.join("config.toml"),
        "[worktrees]\nenabled = false\n",
    )
    .unwrap();
}

/// Build a minimal CoordinationRequest with only `action` set.
fn coord_req(action: &str) -> CoordinationRequest {
    CoordinationRequest {
        action: action.to_string(),
        id: None,
        task_id: None,
        target: None,
        message: None,
        summary: None,
        urgent: None,
        force: None,
        limit: None,
        name: None,
        agent_type: None,
        parent_id: None,
        session_id: None,
        prompt: None,
        max_iterations: None,
        completion_promise: None,
        reason: None,
        stale_threshold_secs: None,
        supervisor_id: None,
        event_type: None,
        payload: None,
        priority: None,
        notification_id: None,
        count: None,
        worker_names: None,
        branch: None,
        older_than_secs: None,
        isolate: None,
        cli: None,
        model: None,
        effort: None,
        remind_message: None,
        remind_delay_secs: None,
        remind_event: None,
        remind_filter: None,
        remind_id: None,
        remind_ttl_secs: None,
        all: None,
        status: None,
        orphans: None,
        dry_run: None,
    }
}

fn get_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// Tests
// =============================================================================

/// AC1 + AC2: `worktree_list` returns the factory (System B) worktrees and labels
/// them `[factory]`, rather than returning the "experimental and disabled" gate
/// message, when System A is off but a real factory worktree is present.
#[tokio::test]
async fn test_worktree_list_shows_factory_worktrees_when_system_a_disabled() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");

    // System A explicitly off
    disable_system_a(&cas_root);

    // Create a factory (System B) worktree at the standard location
    let wt_path = cas_root.join("worktrees").join("alice");
    repo.add_worktree(&wt_path, "factory/alice");

    let svc = make_service(cas_root);
    let result = svc
        .coordination(Parameters(coord_req("worktree_list")))
        .await
        .expect("coordination call should succeed");

    let text = get_text(&result);

    // Must NOT show the misleading disabled-gate message
    assert!(
        !text.contains("experimental and disabled"),
        "worktree_list must not return the 'disabled' gate message when factory worktrees \
         exist (System A off, System B active).\nGot:\n{text}"
    );

    // Must include the factory worktree's branch name
    assert!(
        text.contains("factory/alice"),
        "worktree_list must list the factory/alice branch.\nGot:\n{text}"
    );

    // AC2: output must distinguish factory (System B) worktrees
    assert!(
        text.contains("[factory]") || text.to_lowercase().contains("factory"),
        "worktree_list output must label the worktree as factory (System B).\nGot:\n{text}"
    );
}

/// AC4 (regression): when NO worktrees exist and System A is off, `worktree_list`
/// returns an informational "No worktrees" message — not the misleading
/// "experimental and disabled" gate message.
#[tokio::test]
async fn test_worktree_list_no_disabled_message_when_no_factory_worktrees() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");

    // System A off, no factory worktrees at all
    disable_system_a(&cas_root);

    let svc = make_service(cas_root);
    let result = svc
        .coordination(Parameters(coord_req("worktree_list")))
        .await
        .expect("coordination call should succeed");

    let text = get_text(&result);

    // Gate must not block with the 'disabled' message
    assert!(
        !text.contains("experimental and disabled"),
        "worktree_list must not show the misleading 'disabled' gate message.\nGot:\n{text}"
    );

    // Should return the natural empty-list response
    assert!(
        text.contains("No worktrees"),
        "worktree_list should say 'No worktrees' when none exist.\nGot:\n{text}"
    );
}
