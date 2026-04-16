//! Panic catcher for MCP tool dispatch.
//!
//! Every `#[tool]` method on [`super::CasService`] delegates its real work
//! through [`dispatch_with_catch`], which runs the future on a dedicated
//! tokio task and converts any panic into a structured [`McpError`]. Without
//! this, a panic inside a handler kills the MCP stdio worker — the client
//! sees a generic "connection closed", the panic trace is lost, and the
//! factory supervisor has to respawn `cas serve` to keep working.
//!
//! We intentionally use `tokio::spawn(...).await` + `JoinError::is_panic()`
//! rather than `std::panic::catch_unwind`:
//!
//!   * `catch_unwind` does not cross `.await` points. A panic that
//!     originates inside an awaited future cannot be caught that way.
//!   * A spawned task that panics unwinds only its own stack. State held
//!     by the caller (the MCP dispatcher, store handles, mutex guards on
//!     `&self`) is not visible to the panicking task.
//!
//! `cas serve` startup installs a process-wide panic hook that writes the
//! panic location + backtrace to `{cas_root}/logs/cas-serve-{date}.log`
//! (see `crate::mcp::server::runtime::install_serve_panic_hook`). That
//! hook runs on the panicking worker thread **before** the `JoinError`
//! surfaces here, so by the time we synthesize the client-facing error
//! the forensic trail is already on disk.

use std::any::Any;
use std::borrow::Cow;
use std::future::Future;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, ErrorCode};

/// Run `fut` on a dedicated tokio task. On success, return the handler's
/// own result; on panic, return a structured `INTERNAL_ERROR` so the MCP
/// client sees a tool-error response instead of a dropped connection.
///
/// `tool_name` is the stable short label used in the client-facing error
/// message and in the server-side stderr log line. Downstream telemetry
/// treats it as a category, so do not interpolate dynamic data.
pub(super) async fn dispatch_with_catch<F>(
    tool_name: &'static str,
    fut: F,
) -> Result<CallToolResult, McpError>
where
    F: Future<Output = Result<CallToolResult, McpError>> + Send + 'static,
{
    // Stub: not yet panic-catching. Tests in this module are expected to
    // fail until dispatch_with_catch is implemented for real in the next
    // commit (cas-a436 test-first).
    fut.await
}

/// Best-effort extraction of a message from a panic payload. Rust stores
/// the argument to `panic!(...)` as a boxed `Any`:
///
///   * `panic!("literal")`      → `&'static str`
///   * `panic!("fmt {x}")`      → `String`
///   * `panic_any(custom_type)` → the custom type
///
/// We downcast the first two cases. Unknown payload types fall back to a
/// placeholder so the caller never sees an empty message.
#[allow(dead_code)]
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    "<non-string panic payload>".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Content;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn successful_handler_passes_through() {
        let result = dispatch_with_catch("ok_tool", async {
            Ok(CallToolResult::success(vec![Content::text("hi")]))
        })
        .await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn error_result_passes_through_unchanged() {
        let result = dispatch_with_catch("err_tool", async {
            Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::Borrowed("bad input"),
                data: None,
            })
        })
        .await;
        let err = result.expect_err("expected Err");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert_eq!(err.message, "bad input");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panic_with_str_literal_returns_internal_error() {
        let result = dispatch_with_catch("str_panic_tool", async {
            panic!("boom");
            #[allow(unreachable_code)]
            Ok(CallToolResult::success(vec![]))
        })
        .await;
        let err = result.expect_err("expected Err from panicking handler");
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
        assert!(
            err.message.contains("handler panicked in 'str_panic_tool'"),
            "missing tool label in message: {}",
            err.message
        );
        assert!(
            err.message.contains("boom"),
            "panic payload not propagated: {}",
            err.message
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panic_with_formatted_string_returns_internal_error() {
        let result = dispatch_with_catch("fmt_panic_tool", async {
            let n: u32 = 42;
            panic!("answer: {n}");
            #[allow(unreachable_code)]
            Ok(CallToolResult::success(vec![]))
        })
        .await;
        let err = result.expect_err("expected Err");
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
        assert!(
            err.message.contains("answer: 42"),
            "String-payload panic not propagated: {}",
            err.message
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ten_consecutive_panics_do_not_kill_runtime() {
        for i in 0..10u32 {
            let result = dispatch_with_catch("spam_tool", async move {
                panic!("boom #{i}");
                #[allow(unreachable_code)]
                Ok(CallToolResult::success(vec![]))
            })
            .await;
            assert!(
                result.is_err(),
                "iteration {i} should have surfaced the panic as Err"
            );
        }

        // Runtime must still be usable after 10 panics.
        let ok = dispatch_with_catch("ok_after_panics", async {
            Ok(CallToolResult::success(vec![Content::text("alive")]))
        })
        .await;
        assert!(
            ok.is_ok(),
            "runtime did not survive 10 consecutive panics: {ok:?}"
        );
    }
}
