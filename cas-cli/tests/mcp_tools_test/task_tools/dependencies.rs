//! cas-ac2e: `dep_add` confirmation + `dep_list`/`task show` rendering must
//! state dependency direction in plain, unambiguous words — no bare
//! `from -> to` arrow. See BUG-dep-add-direction-ambiguous-output-2026-07-08.md.
//!
//! Edge direction/semantics are unchanged by this task (verified by the
//! existing dependency-store tests continuing to pass); these tests cover
//! only the OUTPUT.

use crate::support::*;
use cas::mcp::tools::*;
use rmcp::handler::server::wrapper::Parameters;

async fn create_task(service: &cas::mcp::CasCore, title: &str) -> String {
    let req = TaskCreateRequest {
        depth: None,
        title: title.to_string(),
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
    };
    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");
    extract_task_id(&extract_text(result))
        .expect("should have task ID")
        .to_string()
}

/// AC: "Test asserting the new dep_add confirmation states 'blocked_by' in
/// plain words (no bare `A -> B` arrow)."
///
/// Reproduces the exact bug-doc scenario: `dep_add id=A to_id=B
/// dep_type=blocks` must NOT read as "A blocks B" — it must plainly say A
/// is blocked_by B (waits on B).
#[tokio::test]
async fn test_dep_add_blocks_confirmation_states_blocked_by_in_plain_words() {
    let (_temp, service) = setup_cas();

    let id_a = create_task(&service, "Task A").await;
    let id_b = create_task(&service, "Task B").await;

    let dep_req = DependencyRequest {
        from_id: id_a.clone(),
        to_id: id_b.clone(),
        dep_type: "blocks".to_string(),
    };
    let text = extract_text(
        service
            .cas_task_dep_add(Parameters(dep_req))
            .await
            .expect("dep_add should succeed"),
    );

    assert!(
        !text.contains("->"),
        "confirmation must not contain a bare arrow: {text}"
    );
    assert!(
        text.contains("blocked_by"),
        "confirmation must state blocked_by in plain words: {text}"
    );
    assert!(
        text.contains(&format!("{id_a} will not start until {id_b} is done")),
        "confirmation must spell out which task waits on which: {text}"
    );
    // Precondition sanity: the edge's actual effect really is A blocked_by B
    // (not the reverse) — confirms the output now matches the real semantics.
    let blocked = extract_text(
        service
            .cas_task_blocked(Parameters(TaskReadyBlockedRequest {
                limit: None,
                scope: "all".to_string(),
                sort: None,
                sort_order: None,
                epic: None,
            }))
            .await
            .expect("blocked should succeed"),
    );
    assert!(
        blocked.contains(&id_a),
        "task A must actually be blocked (by B), confirming the output matches reality: {blocked}"
    );
}

/// AC: "dep_list ... should render dependencies as 'blocked by: […]' and
/// 'blocks: […]' sections rather than raw `A -> B` arrows."
#[tokio::test]
async fn test_dep_list_renders_blocked_by_and_blocks_sections_no_arrows() {
    let (_temp, service) = setup_cas();

    let id_a = create_task(&service, "Task A").await;
    let id_b = create_task(&service, "Task B").await;
    let id_c = create_task(&service, "Task C").await;

    // A is blocked_by B (A -- blocks --> created as from=A to=B).
    service
        .cas_task_dep_add(Parameters(DependencyRequest {
            from_id: id_a.clone(),
            to_id: id_b.clone(),
            dep_type: "blocks".to_string(),
        }))
        .await
        .expect("dep_add A blocked_by B");

    // C is blocked_by A (A blocks C).
    service
        .cas_task_dep_add(Parameters(DependencyRequest {
            from_id: id_c.clone(),
            to_id: id_a.clone(),
            dep_type: "blocks".to_string(),
        }))
        .await
        .expect("dep_add C blocked_by A");

    let text = extract_text(
        service
            .cas_task_dep_list(Parameters(IdRequest { id: id_a.clone() }))
            .await
            .expect("dep_list should succeed"),
    );

    assert!(
        !text.contains("->"),
        "dep_list must not render bare arrows: {text}"
    );
    assert!(
        text.to_lowercase().contains("blocked by"),
        "dep_list must have a plain-worded 'blocked by' section: {text}"
    );
    assert!(
        text.to_lowercase().contains("blocks"),
        "dep_list must have a plain-worded 'blocks' section: {text}"
    );
    // A is blocked BY b (b must go first) and A blocks C (c waits on a).
    assert!(
        text.contains(&id_b),
        "blocked-by section must name the blocker (B): {text}"
    );
    assert!(
        text.contains(&id_c),
        "blocks section must name the task waiting on A (C): {text}"
    );
}

/// AC: "task show" must also render plain-worded sections, not arrows.
/// `cas_task_show` already avoided bare arrows before this task (it used
/// `BlockedBy:`/`Blocks:` labels); this task aligns the wording with
/// dep_list's new "Blocked by:" phrasing for consistency.
#[tokio::test]
async fn test_task_show_with_deps_has_no_arrows_and_names_both_directions() {
    let (_temp, service) = setup_cas();

    let id_a = create_task(&service, "Task A").await;
    let id_b = create_task(&service, "Task B").await;
    let id_c = create_task(&service, "Task C").await;

    service
        .cas_task_dep_add(Parameters(DependencyRequest {
            from_id: id_a.clone(),
            to_id: id_b.clone(),
            dep_type: "blocks".to_string(),
        }))
        .await
        .expect("dep_add A blocked_by B");
    service
        .cas_task_dep_add(Parameters(DependencyRequest {
            from_id: id_c.clone(),
            to_id: id_a.clone(),
            dep_type: "blocks".to_string(),
        }))
        .await
        .expect("dep_add C blocked_by A");

    let text = extract_text(
        service
            .cas_task_show(Parameters(TaskShowRequest {
                id: id_a.clone(),
                with_deps: true,
            }))
            .await
            .expect("task_show should succeed"),
    );

    assert!(
        !text.contains("->"),
        "task show must not render bare arrows: {text}"
    );
    assert!(
        text.to_lowercase().contains("blocked by"),
        "task show must name the blocked-by relationship in plain words: {text}"
    );
    assert!(
        text.contains(&id_b),
        "task show must name the blocker (B): {text}"
    );
    assert!(
        text.contains(&id_c),
        "task show must name the task blocked by A (C): {text}"
    );
}
