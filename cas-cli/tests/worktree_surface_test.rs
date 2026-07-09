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
use cas::store::{init_cas_dir, open_task_store};
use cas::types::{Task, TaskType};
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
        clear: None,
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

// =============================================================================
// cas-1d11: worktree_merge must agree with spawn isolate=true on
// worktrees.enabled — a factory (System B) worktree must be mergeable
// even when System A is off, since spawn never checked that flag either.
// =============================================================================

/// `worktree_merge`'s handler resolves the repo root from the *process*
/// current directory (`std::env::current_dir()`), not from `cas_root` —
/// a pre-existing quirk shared by `worktree_create` too, unrelated to this
/// fix. Since cwd is process-global, tests that exercise `worktree_merge`
/// must serialize around changing it. All such tests live in this file and
/// take this lock for their full duration; no other test file is affected
/// (`cargo test` runs each integration-test file as its own process).
fn merge_cwd_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// RAII guard: switches the process cwd to `dir` on construction, restores
/// the original cwd on drop (including on panic/early return).
struct CwdGuard {
    original: PathBuf,
}

impl CwdGuard {
    fn enter(dir: &Path) -> Self {
        let original = std::env::current_dir().expect("current_dir");
        std::env::set_current_dir(dir).expect("set_current_dir");
        Self { original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

fn run_git(args: &[&str], dir: &Path) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// AC: spawn isolate=true creates a real factory worktree regardless of
/// `worktrees.enabled`; `worktree_merge` must actually merge it instead of
/// refusing with the "disabled by default" message.
#[tokio::test]
async fn test_worktree_merge_succeeds_for_factory_worktree_when_system_a_disabled() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");
    disable_system_a(&cas_root);

    let wt_path = cas_root.join("worktrees").join("alice");
    repo.add_worktree(&wt_path, "factory/alice");

    // Give the worker branch real content to merge, not just an empty commit.
    std::fs::write(wt_path.join("alice-work.txt"), "alice's work").unwrap();
    run_git(&["add", "."], &wt_path);
    run_git(&["commit", "-m", "alice work"], &wt_path);

    let _lock = merge_cwd_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _cwd = CwdGuard::enter(&repo.root);

    let svc = make_service(cas_root);
    let mut req = coord_req("worktree_merge");
    req.id = Some("factory/alice".to_string());
    let result = svc
        .coordination(Parameters(req))
        .await
        .expect("coordination call should succeed");

    let text = get_text(&result);

    assert!(
        !text.contains("experimental and disabled"),
        "worktree_merge must not refuse a real factory (System B) worktree just \
         because System A's flag is off — spawn never checked that flag either.\nGot:\n{text}"
    );
    assert!(
        text.contains("Merged worktree"),
        "worktree_merge should report a successful merge.\nGot:\n{text}"
    );

    // The merge actually landed: content reachable from the checked-out repo.
    assert!(
        repo.root.join("alice-work.txt").exists(),
        "merged content must land on the parent branch's working tree"
    );
    // cleanup_on_close defaults true: the worktree directory is reclaimed.
    assert!(
        !wt_path.exists(),
        "worktree directory should be cleaned up after a successful merge"
    );
}

/// Negative case: when neither System A nor System B has a matching
/// worktree, `worktree_merge` must report an accurate "not found" — never
/// silently succeed, and never fall back to the misleading "disabled"
/// message (that message implies the feature is off, not that the target
/// doesn't exist).
#[tokio::test]
async fn test_worktree_merge_reports_not_found_not_disabled_when_nothing_matches() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");
    disable_system_a(&cas_root);
    // No worktree created for "bob" in either system.

    let _lock = merge_cwd_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _cwd = CwdGuard::enter(&repo.root);

    let svc = make_service(cas_root);
    let mut req = coord_req("worktree_merge");
    req.id = Some("factory/bob".to_string());
    let result = svc.coordination(Parameters(req)).await;

    let (not_disabled, contains_not_found) = match &result {
        Ok(r) => {
            let text = get_text(r);
            (
                !text.contains("experimental and disabled"),
                text.to_lowercase().contains("not found"),
            )
        }
        Err(e) => {
            let msg = format!("{e:?}");
            (
                !msg.contains("experimental and disabled"),
                msg.to_lowercase().contains("not found"),
            )
        }
    };

    assert!(
        not_disabled,
        "a missing worktree must never be reported as 'disabled' — that implies \
         the feature is off, not that the target doesn't exist. Got: {result:?}"
    );
    assert!(
        contains_not_found,
        "a missing worktree should be reported as not found. Got: {result:?}"
    );
}

// =============================================================================
// cas-0938: worktree_merge's System-B fallback must target the worker's
// TASK'S EPIC branch, not the repo trunk — merging an epic worker's commits
// to trunk (then deleting the branch via cleanup_on_close) is a silent
// wrong-target class of bug: worse than cas-1d11's pre-fix refusal, because
// the close-gate still rejects AND unreviewed code now sits on trunk with
// the only copy of it gone.
// =============================================================================

fn create_epic_and_worker_task(cas_root: &Path, epic_branch: &str) -> (String, String) {
    let task_store = open_task_store(cas_root).expect("open_task_store");

    let mut epic = Task::new("epic-1".to_string(), "Test epic".to_string());
    epic.task_type = TaskType::Epic;
    epic.branch = Some(epic_branch.to_string());
    task_store.add(&epic).expect("add epic task");

    let worker_task = Task::new("worker-task-1".to_string(), "Worker task".to_string());
    task_store
        .create_atomic(&worker_task, &[], Some(&epic.id), None)
        .expect("create worker task under epic");

    (epic.id, worker_task.id)
}

#[tokio::test]
async fn test_worktree_merge_targets_epic_branch_when_task_id_given() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");
    disable_system_a(&cas_root);

    // The epic branch is a real branch in this repo (created off main) so
    // merge_and_cleanup can actually check it out.
    Command::new("git")
        .args(["branch", "epic/foo"])
        .current_dir(&repo.root)
        .output()
        .unwrap();

    let (_epic_id, worker_task_id) = create_epic_and_worker_task(&cas_root, "epic/foo");

    let wt_path = cas_root.join("worktrees").join("alice");
    repo.add_worktree(&wt_path, "factory/alice");
    std::fs::write(wt_path.join("alice-work.txt"), "alice's work").unwrap();
    run_git(&["add", "."], &wt_path);
    run_git(&["commit", "-m", "alice work"], &wt_path);

    let _lock = merge_cwd_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _cwd = CwdGuard::enter(&repo.root);

    let svc = make_service(cas_root);
    let mut req = coord_req("worktree_merge");
    req.id = Some("factory/alice".to_string());
    req.task_id = Some(worker_task_id);
    let result = svc
        .coordination(Parameters(req))
        .await
        .expect("coordination call should succeed");
    let text = get_text(&result);

    assert!(
        text.contains("Merged worktree") && text.contains("epic/foo"),
        "must merge into the task's epic branch (epic/foo), not trunk.\nGot:\n{text}"
    );
    assert!(
        !text.contains("Merged worktree system-b-alice to main")
            && !text.contains("Merged worktree system-b-alice to master"),
        "must NOT merge to trunk when the task has a parent epic.\nGot:\n{text}"
    );
    assert!(
        text.contains("[resolved via:"),
        "the resolved target and why must be surfaced in the success message.\nGot:\n{text}"
    );

    // The epic branch itself must now contain the worker's content — proves
    // the merge landed on the right branch, not just that the message says so.
    let epic_tree = Command::new("git")
        .args(["ls-tree", "-r", "--name-only", "epic/foo"])
        .current_dir(&repo.root)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&epic_tree.stdout).contains("alice-work.txt"),
        "epic/foo must contain the merged worker content"
    );
}

#[tokio::test]
async fn test_worktree_merge_falls_back_to_trunk_when_task_has_no_parent_epic() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");
    disable_system_a(&cas_root);

    // A standalone (non-epic) task — legitimately "no epic in play".
    let task_store = open_task_store(&cas_root).expect("open_task_store");
    let standalone_task = Task::new("standalone-1".to_string(), "Standalone task".to_string());
    task_store.add(&standalone_task).expect("add standalone task");

    let wt_path = cas_root.join("worktrees").join("bob");
    repo.add_worktree(&wt_path, "factory/bob");

    let _lock = merge_cwd_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _cwd = CwdGuard::enter(&repo.root);

    let svc = make_service(cas_root);
    let mut req = coord_req("worktree_merge");
    req.id = Some("factory/bob".to_string());
    req.task_id = Some(standalone_task.id.clone());
    let result = svc
        .coordination(Parameters(req))
        .await
        .expect("coordination call should succeed");
    let text = get_text(&result);

    assert!(
        text.contains("Merged worktree"),
        "a task with no parent epic must still merge to trunk, not refuse.\nGot:\n{text}"
    );
    assert!(
        text.contains("no parent epic"),
        "the trunk fallback reason must explain why (no parent epic), not just say 'trunk'.\nGot:\n{text}"
    );
}

#[tokio::test]
async fn test_worktree_merge_refuses_when_task_id_does_not_exist() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");
    disable_system_a(&cas_root);

    let wt_path = cas_root.join("worktrees").join("carol");
    repo.add_worktree(&wt_path, "factory/carol");

    let _lock = merge_cwd_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _cwd = CwdGuard::enter(&repo.root);

    let svc = make_service(cas_root);
    let mut req = coord_req("worktree_merge");
    req.id = Some("factory/carol".to_string());
    req.task_id = Some("cas-does-not-exist".to_string());
    let result = svc.coordination(Parameters(req)).await;

    // A caller-asserted task_id we can't verify must refuse — never guess a
    // merge target (that's exactly how the original wrong-target-to-trunk
    // defect happened) and never silently merge to trunk instead.
    assert!(
        result.is_err(),
        "an unresolvable task_id must be refused, not silently fall back to trunk"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("not found") || msg.to_lowercase().contains("not found"),
        "the refusal should explain the task_id couldn't be resolved. Got: {msg}"
    );

    // The worktree must survive the refused merge — not silently deleted.
    assert!(
        wt_path.exists(),
        "a refused merge must not clean up / delete the worktree"
    );
}

#[tokio::test]
async fn test_worktree_merge_falls_back_to_trunk_when_no_task_id_given() {
    // Regression guard: the original cas-1d11 caller pattern (no task_id at
    // all) must keep working exactly as before — trunk fallback, not a
    // refusal, since "no task_id" is a legitimate "no epic context" case.
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");
    disable_system_a(&cas_root);

    let wt_path = cas_root.join("worktrees").join("dave");
    repo.add_worktree(&wt_path, "factory/dave");

    let _lock = merge_cwd_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _cwd = CwdGuard::enter(&repo.root);

    let svc = make_service(cas_root);
    let mut req = coord_req("worktree_merge");
    req.id = Some("factory/dave".to_string());
    let result = svc
        .coordination(Parameters(req))
        .await
        .expect("coordination call should succeed");
    let text = get_text(&result);

    assert!(text.contains("Merged worktree"));
    assert!(
        text.contains("no task_id given"),
        "the trunk-fallback reason must be explicit about why. Got:\n{text}"
    );
}

/// cas-0938 P3: System-B path resolution must honor a customized
/// `worktrees.base_path`, not the hardcoded `<cas_root>/worktrees/<assignee>`
/// convention — `spawn_workers isolate=true` itself resolves paths via
/// `WorktreeManager::worktree_path_for_worker`, which respects this config,
/// so a hardcoded path in `worktree_merge` would false-not-found any worker
/// spawned under a non-default layout.
#[tokio::test]
async fn test_worktree_merge_honors_configured_base_path_not_hardcoded_convention() {
    let repo = GitRepo::new();
    let cas_root = init_cas_dir(&repo.root).expect("init_cas_dir");
    std::fs::write(
        cas_root.join("config.toml"),
        "[worktrees]\nenabled = false\nbase_path = \"custom-worktree-loc\"\n",
    )
    .unwrap();

    // Mirrors WorktreeManager::worktree_root()'s resolution for a relative,
    // non-{project} base_path: repo_root.parent().join(base_path).
    let expected_root = repo
        .root
        .parent()
        .unwrap()
        .join("custom-worktree-loc")
        .join("erin");
    repo.add_worktree(&expected_root, "factory/erin");

    // Sanity: this is NOT where the old hardcoded convention would look.
    assert_ne!(expected_root, cas_root.join("worktrees").join("erin"));

    let _lock = merge_cwd_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _cwd = CwdGuard::enter(&repo.root);

    let svc = make_service(cas_root);
    let mut req = coord_req("worktree_merge");
    req.id = Some("factory/erin".to_string());
    let result = svc
        .coordination(Parameters(req))
        .await
        .expect("coordination call should succeed");
    let text = get_text(&result);

    assert!(
        text.contains("Merged worktree"),
        "must find and merge the worker worktree at its CONFIGURED location, \
         not the hardcoded default.\nGot:\n{text}"
    );
    assert!(
        !text.contains("Worktree not found"),
        "must not false-not-found a worker under a customized base_path.\nGot:\n{text}"
    );
}
