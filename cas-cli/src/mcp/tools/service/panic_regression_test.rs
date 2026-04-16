//! End-to-end regression test for cas-3b51 (EPIC cas-c351).
//!
//! Verifies that the A2 panic catcher applied to every `#[tool]` method on
//! [`super::CasService`] is wired through the real dispatch path, not just
//! the helper module's isolated unit tests. Constructs a real `CasService`
//! backed by a real `CasCore` over a temporary DB, hits
//! `CasService::system` with a `#[cfg(test)]`-guarded action that forces
//! the handler body to `panic!(...)`, and asserts:
//!
//!   * the call surfaces as `McpError{INTERNAL_ERROR, "handler panicked
//!     in 'system': ..."}` rather than killing the process;
//!   * a follow-up call on the **same** `CasService` instance succeeds,
//!     proving the service continues serving after a handler panic;
//!   * the pattern holds for 10 consecutive panics.
//!
//! ## Why this test fails if A2 is removed
//!
//! "A2 removed" has two meaningful shapes:
//!
//!   1. `panic_catch::dispatch_with_catch` deleted / not called by
//!      `system()`. The `#[cfg(test)]` panic injection then runs on the
//!      caller's task (the test's own tokio runtime worker). The panic
//!      unwinds through the `.await` and into the test function, where
//!      the test harness records a panicked test.
//!   2. The `#[cfg(test)]` injection branch removed from `system()`. The
//!      forced action falls through to the handler's `_ =>` arm and
//!      returns `Err(INVALID_PARAMS)`, which does not contain the
//!      required `"handler panicked in 'system'"` substring and the
//!      error-code assertion fails.
//!
//! Either collapse produces a concrete test failure, which is what the
//! A3 acceptance criteria asks for.
//!
//! ## Why it lives in the unit-test tree rather than `cas-cli/tests/`
//!
//! Integration tests under `cas-cli/tests/` build the lib crate with
//! `#[cfg(test)]` stripped, so the panic-injection branch inside
//! `system()` would not be compiled and the test could not reach it.
//! A unit test inside the crate sees `#[cfg(test)]` turned on, so the
//! injection compiles and the test can exercise it. The acceptance
//! criterion "lives under `cas-cli/src/mcp/tools/service/` test tree"
//! matches this constraint.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ErrorCode;
use serde_json::json;
use tempfile::TempDir;

use super::{CasService, SystemRequest};
use crate::mcp::server::CasCore;

/// Construct a fresh `CasService` over a temporary DB. The `TempDir` is
/// returned so the caller keeps it alive for the duration of the test;
/// dropping it would delete the backing SQLite file mid-test.
fn make_service() -> (TempDir, CasService) {
    let dir = TempDir::new().expect("tempdir");
    let core = CasCore::with_daemon(dir.path().to_path_buf(), None, None);
    #[cfg(feature = "mcp-proxy")]
    let svc = CasService::new(core, None);
    #[cfg(not(feature = "mcp-proxy"))]
    let svc = CasService::new(core);
    (dir, svc)
}

fn panic_req() -> SystemRequest {
    serde_json::from_value(json!({ "action": "__panic_for_test__" }))
        .expect("static JSON must deserialize")
}

fn version_req() -> SystemRequest {
    serde_json::from_value(json!({ "action": "version" }))
        .expect("static JSON must deserialize")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panic_in_system_dispatch_returns_internal_error_and_keeps_server_alive() {
    let (_dir, svc) = make_service();

    // Forced panic inside system()'s handler body must be converted by
    // dispatch_with_catch into a structured McpError, not propagated to
    // the caller's task.
    let err = svc
        .system(Parameters(panic_req()))
        .await
        .expect_err("forced panic must surface as Err, not unwind to caller");
    assert_eq!(
        err.code,
        ErrorCode::INTERNAL_ERROR,
        "panic must be reported as INTERNAL_ERROR, got {:?}: {}",
        err.code,
        err.message
    );
    assert!(
        err.message.contains("handler panicked in 'system'"),
        "error message must carry the tool label so the client can diagnose: {}",
        err.message
    );

    // Same CasService instance must still be usable after a panicked
    // handler — that is the whole point of A2.
    svc.system(Parameters(version_req()))
        .await
        .expect("system('version') must succeed after a panicked dispatch");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ten_consecutive_dispatch_panics_do_not_kill_service() {
    let (_dir, svc) = make_service();

    for i in 0..10 {
        let err = svc
            .system(Parameters(panic_req()))
            .await
            .unwrap_err_or_else(|| panic!("iteration {i} unexpectedly succeeded"));
        assert_eq!(
            err.code,
            ErrorCode::INTERNAL_ERROR,
            "iteration {i} returned wrong code: {:?}",
            err.code
        );
        assert!(
            err.message.contains("handler panicked in 'system'"),
            "iteration {i} lost the tool label: {}",
            err.message
        );
    }

    // Service survives the sustained panic storm and still serves a
    // real action.
    svc.system(Parameters(version_req()))
        .await
        .expect("service must still serve version after 10 consecutive panics");
}

/// Tiny extension so the loop body above reads cleanly. `unwrap_err`
/// panics with the `Ok` Debug-printed, which for `CallToolResult` is
/// an opaque payload — a short helper lets the caller inject the loop
/// index into the failure message.
trait ExpectErrExt<T, E> {
    fn unwrap_err_or_else<F: FnOnce() -> E>(self, f: F) -> E;
}

impl<T, E> ExpectErrExt<T, E> for Result<T, E> {
    fn unwrap_err_or_else<F: FnOnce() -> E>(self, f: F) -> E {
        match self {
            Ok(_) => f(),
            Err(e) => e,
        }
    }
}
