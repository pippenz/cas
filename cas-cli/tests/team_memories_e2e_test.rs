//! End-to-end tests for the team-memories workflow (EPIC cas-cf44).
//!
//! Exercises the full pipeline that lets a teammate land on a project
//! and see shared memories with zero flags and zero UUID lookups:
//!
//! 1. `cas memory share --all` retroactively promotes pre-existing
//!    personal entries to the team queue (T5 cas-07d7).
//! 2. `cas cloud sync` drains the team queue into the team push
//!    endpoint (T4 cas-1f44).
//! 3. `cas cloud team-memories` pulls those memories into a fresh
//!    teammate's local store (the zero-flag journey).
//!
//! Each test exercises the real SQLite store, real `SyncQueue`, and
//! the real `CloudSyncer::push_team` / `pull_team` code paths. Only
//! the HTTP boundary is mocked via `wiremock`. This matches the
//! pattern established by `team_sync_test.rs` (cas-1f44) and extends
//! it to the retroactive backfill + pull-side cases.
//!
//! Carry-in (from T5 verification note): `share --since <duration>`
//! has parse_duration unit-test coverage but no integration coverage
//! of the time-window filter path. `share_since_filter_...` below
//! closes that gap.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cas::cli::Cli;
use cas::cli::cloud::execute_team_push;
use cas::cli::memory::{ShareArgs, execute_share};
use cas::cloud::{CloudConfig, CloudSyncer, CloudSyncerConfig, SyncQueue};
use cas::store::open_store;
use cas::types::{Entry, EntryType, Scope, ShareScope};
use tempfile::TempDir;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Fixture UUID shared with other team-sync tests.
const TEST_TEAM: &str = "550e8400-e29b-41d4-a716-446655440000";

fn make_cli_json() -> Cli {
    Cli {
        json: true,
        full: false,
        verbose: false,
        command: None,
    }
}

fn team_cloud_config(endpoint: String) -> CloudConfig {
    let mut cfg = CloudConfig::default();
    cfg.endpoint = endpoint;
    cfg.token = Some("test-token".to_string());
    cfg.set_team(TEST_TEAM, "test-team");
    cfg
}

fn seed_team_cloud_config_on_disk(cas_dir: &Path, endpoint: String) {
    team_cloud_config(endpoint).save_to_cas_dir(cas_dir).unwrap();
}

fn mock_push_endpoint() -> Mock {
    Mock::given(method("POST"))
        .and(path_matcher(format!("/api/teams/{TEST_TEAM}/sync/push")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "synced": {
                    "entries": 0,
                    "tasks": 0, "rules": 0, "skills": 0,
                    "sessions": 0, "verifications": 0, "events": 0,
                    "prompts": 0, "file_changes": 0, "commit_links": 0,
                    "agents": 0, "worktrees": 0,
                }
            })),
        )
        .expect(1..)
}

/// Test 1 (primary E2E): retroactive backfill + team push.
///
/// Simulates Daniel's 392-entry scenario: entries that were written
/// BEFORE a team was configured (so they only hit the personal queue)
/// can be retroactively promoted with `cas memory share --all` and
/// pushed to the team via `execute_team_push`. This is the key
/// regression-catching path — if any of T3 (dual-enqueue), T4 (team
/// push), or T5 (share CLI) breaks, the team queue stays empty and
/// the assertion below fails.
#[tokio::test]
async fn retroactive_share_all_then_team_push_surfaces_preexisting_entries() {
    let server = MockServer::start().await;
    mock_push_endpoint().mount(&server).await;

    let tmp = TempDir::new().unwrap();
    let cas_dir = tmp.path();

    // Stage 1: cold start — NO team configured, no cloud.json.
    // Seed two project-scoped learnings (T1 predicate says eligible
    // for team push once a team is configured).
    {
        let store = open_store(cas_dir).unwrap();
        store
            .add(&Entry {
                id: "2026-03-01-1".to_string(),
                scope: Scope::Project,
                entry_type: EntryType::Learning,
                content: "pre-existing personal entry A".to_string(),
                ..Default::default()
            })
            .unwrap();
        store
            .add(&Entry {
                id: "2026-03-01-2".to_string(),
                scope: Scope::Project,
                entry_type: EntryType::Learning,
                content: "pre-existing personal entry B".to_string(),
                ..Default::default()
            })
            .unwrap();
    }

    // With no team configured, the team queue must be empty — both
    // entries only went to the personal queue.
    {
        let q = SyncQueue::open(cas_dir).unwrap();
        q.init().unwrap();
        assert_eq!(
            q.pending_for_team(TEST_TEAM, 1000, 10).unwrap().len(),
            0,
            "team queue must start empty before team is configured"
        );
    }

    // Stage 2: configure the team on disk (simulates `cas cloud team set`).
    seed_team_cloud_config_on_disk(cas_dir, server.uri());

    // Stage 3: retroactive backfill.
    let share_args = ShareArgs {
        id: None,
        since: None,
        all: true,
        dry_run: false,
    };
    let cas_dir_owned = cas_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        execute_share(&share_args, &cas_dir_owned).expect("share --all")
    })
    .await
    .unwrap();

    // Both entries must now carry share=Team on disk AND be in the
    // team queue. Check both — the store assertion proves T5's
    // mutation, the queue assertion proves T3's dual-enqueue fired
    // on the update.
    {
        let store = open_store(cas_dir).unwrap();
        assert_eq!(
            store.get("2026-03-01-1").unwrap().share,
            Some(ShareScope::Team),
            "share --all must mark eligible entries share=Team"
        );
        assert_eq!(
            store.get("2026-03-01-2").unwrap().share,
            Some(ShareScope::Team),
        );

        let q = SyncQueue::open(cas_dir).unwrap();
        q.init().unwrap();
        let team_rows = q.pending_for_team(TEST_TEAM, 1000, 10).unwrap();
        assert_eq!(
            team_rows.len(),
            2,
            "both entries must land in the team queue after share --all"
        );
    }

    // Stage 4: team push drains the queue.
    let cfg = team_cloud_config(server.uri());
    let cas_dir_owned = cas_dir.to_path_buf();
    let cli = make_cli_json();
    tokio::task::spawn_blocking(move || {
        execute_team_push(&cfg, &cas_dir_owned, &cli).expect("execute_team_push")
    })
    .await
    .unwrap();

    {
        let q = SyncQueue::open(cas_dir).unwrap();
        q.init().unwrap();
        assert_eq!(
            q.pending_for_team(TEST_TEAM, 1000, 10).unwrap().len(),
            0,
            "team queue must be drained after execute_team_push",
        );
    }
    // wiremock's `.expect(1..)` ensures at least one POST to the
    // team push endpoint fired; the MockServer Drop verifies it.
}

/// Test 2 (carry-in from T5 verification): `--since <duration>`
/// selects only entries within the cutoff window.
///
/// `parse_duration` is unit-tested, but no integration test
/// exercises the store.list() + created-timestamp filter. This
/// seeds entries with distinct created timestamps and verifies
/// only the recent one gets promoted.
#[tokio::test]
async fn share_since_filter_selects_only_recent_entries() {
    let tmp = TempDir::new().unwrap();
    let cas_dir = tmp.path();

    // Configure team so execute_share produces team-queue side-effects
    // — we're verifying selection, not just mutation.
    seed_team_cloud_config_on_disk(cas_dir, "http://127.0.0.1:0".to_string());

    // Seed three entries: one recent, one ~3 days old, one ~30 days
    // old. All Project/Learning so the T1 predicate is satisfied;
    // the cutoff is the only filter in play.
    let now = chrono::Utc::now();
    let mut recent = Entry {
        id: "recent".to_string(),
        scope: Scope::Project,
        entry_type: EntryType::Learning,
        content: "recent".to_string(),
        ..Default::default()
    };
    recent.created = now - chrono::Duration::hours(12);
    let mut medium = Entry {
        id: "medium".to_string(),
        scope: Scope::Project,
        entry_type: EntryType::Learning,
        content: "medium".to_string(),
        ..Default::default()
    };
    medium.created = now - chrono::Duration::days(3);
    let mut old = Entry {
        id: "old".to_string(),
        scope: Scope::Project,
        entry_type: EntryType::Learning,
        content: "old".to_string(),
        ..Default::default()
    };
    old.created = now - chrono::Duration::days(30);

    {
        let store = open_store(cas_dir).unwrap();
        store.add(&recent).unwrap();
        store.add(&medium).unwrap();
        store.add(&old).unwrap();
    }

    // `--since 48h` should select only the 12-hours-ago entry.
    let args = ShareArgs {
        id: None,
        since: Some("48h".to_string()),
        all: false,
        dry_run: false,
    };
    let cas_dir_owned = cas_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        execute_share(&args, &cas_dir_owned).expect("share --since 48h")
    })
    .await
    .unwrap();

    let store = open_store(cas_dir).unwrap();
    assert_eq!(
        store.get("recent").unwrap().share,
        Some(ShareScope::Team),
        "entry inside --since 48h window must be promoted"
    );
    assert_eq!(
        store.get("medium").unwrap().share,
        None,
        "entry 3d old must be outside a 48h window"
    );
    assert_eq!(
        store.get("old").unwrap().share,
        None,
        "entry 30d old must be outside a 48h window"
    );
}

/// Test 3 (fresh-teammate pull): the wire contract that makes the
/// zero-flag journey work. A `GET /api/teams/{uuid}/sync/pull`
/// response containing a team-scoped entry must land in the local
/// SQLite store via `CloudSyncer::pull_team`. This is the receive
/// side of the E2E chain — if it regresses, teammate B sees
/// nothing regardless of how correctly A pushed.
#[tokio::test]
async fn fresh_teammate_pull_applies_team_memories_to_local_store() {
    let server = MockServer::start().await;

    // Teammate B's shared memory — arrives via the team pull endpoint.
    // No `project_canonical_id` / `project_id` field so the filter
    // `entity_matches_project` accepts it unconditionally (legacy
    // path). Covered separately by entity_matches_project unit tests.
    let shared_entry = serde_json::json!({
        "id": "alice-shared-001",
        "entry_type": "learning",
        "scope": "project",
        "created": chrono::Utc::now().to_rfc3339(),
        "content": "alice's shared learning",
        "helpful_count": 0,
        "harmful_count": 0,
        "archived": false,
        "pending_extraction": false,
        "pending_embedding": false,
        "stability": 0.5,
        "access_count": 0,
        "memory_tier": "working",
        "importance": 0.5,
        "belief_type": "fact",
        "confidence": 1.0,
        "compressed": false,
    });

    Mock::given(method("GET"))
        .and(path_matcher(format!("/api/teams/{TEST_TEAM}/sync/pull")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "entries": [shared_entry],
            "tasks": [],
            "rules": [],
            "skills": [],
            "pulled_at": chrono::Utc::now().to_rfc3339(),
            "team_id": TEST_TEAM,
            "status": "ok",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let cas_dir = tmp.path();

    // Fresh teammate — empty stores, team configured.
    let queue = SyncQueue::open(cas_dir).unwrap();
    queue.init().unwrap();
    let entry_store = open_store(cas_dir).unwrap();
    let task_store = cas::store::open_task_store(cas_dir).unwrap();
    let rule_store = cas::store::open_rule_store(cas_dir).unwrap();
    let skill_store = cas::store::open_skill_store(cas_dir).unwrap();

    // No existing entries.
    assert!(entry_store.get("alice-shared-001").is_err());

    let cfg = team_cloud_config(server.uri());
    let syncer_config = CloudSyncerConfig {
        timeout: Duration::from_secs(5),
        ..Default::default()
    };
    let syncer = CloudSyncer::new(Arc::new(queue), cfg, syncer_config);

    // `pull_team` is sync + blocking ureq; run on the blocking pool
    // so the wiremock tokio runtime can serve the GET.
    let result = tokio::task::spawn_blocking(move || {
        syncer.pull_team(
            TEST_TEAM,
            &*entry_store,
            &*task_store,
            &*rule_store,
            &*skill_store,
        )
    })
    .await
    .unwrap();

    let sync_result = result.expect("pull_team returned Err");
    assert_eq!(
        sync_result.pulled_entries, 1,
        "exactly one entry must be applied to the fresh teammate's store"
    );

    // Fresh teammate's local store now has alice's shared memory,
    // with zero flags, zero UUID lookups — AC demo met.
    let fresh_store = open_store(cas_dir).unwrap();
    let pulled = fresh_store
        .get("alice-shared-001")
        .expect("fresh teammate must see alice's shared memory");
    assert_eq!(pulled.content, "alice's shared learning");
    assert_eq!(pulled.scope, Scope::Project);
    assert_eq!(pulled.entry_type, EntryType::Learning);
    // `wiremock`'s `.expect(1)` asserts the GET actually fired.
}
