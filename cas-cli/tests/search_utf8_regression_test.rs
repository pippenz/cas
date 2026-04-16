//! Regression test for cas-f5e4 / EPIC cas-c351.
//!
//! On 2026-04-16 `cas serve` crashed on every `search` call that surfaced
//! the "CAS factory daemon boot order" entry. The root cause was six
//! open-coded `&s[..max_len-3]` preview-truncation paths on `Entry`,
//! `Rule`, `Skill`, `Spec`, `Prompt`, and `CommitLink`: the cut landed
//! inside the three bytes of `→` (U+2192, bytes 55..58) and Rust's string
//! slice panicked because index 57 is not a char boundary. The panic
//! unwound the Tokio worker, closed the stdio transport, and the MCP
//! client saw nothing but "Connection closed". Four auto-respawns later
//! Claude Code marked `cas serve` dead for the session.
//!
//! The fix (commit ad60df3) centralised the truncation into
//! `cas_types::preview::truncate_preview`, which walks back from the
//! target index with `is_char_boundary` until it finds a safe cut. The
//! function itself has unit tests in the `cas-types` crate, including
//! the exact 2026-04-16 fixture.
//!
//! This file goes one layer further and reproduces the original failure
//! end-to-end over the MCP stdio transport against a real `cas serve`
//! subprocess. If `ad60df3` is ever reverted, the preview panic unwinds
//! the server, the stdio pipe closes, the follow-up call errors, and
//! this test fails loudly. A unit test on `truncate_preview` alone would
//! not catch a re-introduction in, say, a new `*::preview` method that
//! bypassed the helper.
//!
//! # Test shape
//!
//! 1. Initialise a fresh `.cas` tempdir.
//! 2. Spawn `cas serve` and complete the MCP initialize handshake.
//! 3. Seed the crashing entry content via `memory.remember`.
//! 4. Call `search.search` with a query that matches — this triggers
//!    the `Entry::preview(60)` call that used to panic.
//! 5. Assert the search response is a clean non-error result.
//! 6. Call `memory.list` on the **same** stdio pipe as a liveness probe.
//!    If step 4 crashed the server, the pipe is closed and this call
//!    errors — which is the discriminating assertion.
//!
//! # Scope / acceptance
//!
//! - Lives under `cas-cli/tests/` (integration tree) per task spec.
//! - Runs in default `cargo test`.
//! - No network, no cloud auth (CAS_* env vars scrubbed on the child).
//! - Fails if `truncate_preview` reverts to byte-slice-based truncation.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::TempDir;

/// The exact entry content that crashed `cas serve` on 2026-04-16. Byte
/// index 57 lands inside `→` (bytes 55..58) when preview(60) runs.
/// Changing this string means you are no longer reproducing the real
/// regression — introduce a second fixture instead of mutating this one.
const CRASH_FIXTURE: &str =
    "CAS factory daemon boot order: `build_configs_for_mux` → `FactoryApp::new` (spawns PTYs)";

// ============================================================================
// JSON-RPC plumbing (copied from mcp_protocol_test.rs to keep this
// regression test self-contained; cross-file test-util extraction is
// tracked as a separate refactor under the EPIC).
// ============================================================================

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    #[allow(dead_code)]
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    data: Option<Value>,
}

struct McpClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn spawn(cas_dir: &std::path::Path) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cas"));
        cmd.arg("serve")
            .env("CAS_DIR", cas_dir)
            .current_dir(cas_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        scrub_cas_env(&mut cmd);

        let mut child = cmd.spawn().expect("Failed to spawn cas serve");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));

        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    /// Send a JSON-RPC request and read the matching response. If the
    /// child has died (e.g., from a preview panic), the underlying
    /// read/write fails — the caller gets to observe that as the
    /// regression signal.
    fn send_request(&mut self, method: &str, params: Option<Value>) -> JsonRpcResponse {
        let id = self.next_id;
        self.next_id += 1;

        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&req).expect("serialize");
        writeln!(self.stdin, "{line}").expect("write stdin — server pipe closed?");
        self.stdin.flush().expect("flush stdin — server pipe closed?");

        // Skip notifications (no id) until we see our matching response.
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .expect("read stdout — server pipe closed? likely preview() panic");
            assert!(n > 0, "stdout EOF before response — server died");

            let resp: JsonRpcResponse = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("parse response '{line}': {e}"));
            if let Some(resp_id) = resp.id {
                assert_eq!(resp_id, id, "mismatched response id");
                return resp;
            }
            // notification — keep reading.
        }
    }

    fn send_notification(&mut self, method: &str, params: Option<Value>) {
        let n = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(json!({}))
        });
        let line = serde_json::to_string(&n).expect("serialize");
        writeln!(self.stdin, "{line}").expect("write notif");
        self.stdin.flush().expect("flush notif");
    }

    fn initialize(&mut self) {
        let resp = self.send_request(
            "initialize",
            Some(json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "cas-f5e4-test", "version": "1.0.0" }
            })),
        );
        assert!(
            resp.error.is_none(),
            "initialize failed: {:?}",
            resp.error
        );
        self.send_notification("notifications/initialized", None);
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> JsonRpcResponse {
        self.send_request(
            "tools/call",
            Some(json!({ "name": name, "arguments": arguments })),
        )
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Scrub CAS_* env vars so the child `cas serve` does not hijack the
/// host cas.db (e.g., when the test harness runs inside a factory
/// worker session that inherited CAS_ROOT).
fn scrub_cas_env(cmd: &mut Command) -> &mut Command {
    cmd.env_remove("CAS_ROOT")
        .env_remove("CAS_DIR")
        .env_remove("CAS_SESSION_ID")
        .env_remove("CAS_AGENT_NAME")
        .env_remove("CAS_AGENT_ROLE")
        .env_remove("CAS_FACTORY_MODE")
        .env_remove("CAS_CLONE_PATH")
}

fn init_cas_dir(dir: &TempDir) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cas"));
    cmd.args(["init", "--yes"]).current_dir(dir.path());
    scrub_cas_env(&mut cmd);
    let out = cmd.output().expect("spawn cas init");
    assert!(
        out.status.success(),
        "cas init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ============================================================================
// The regression
// ============================================================================

#[test]
fn search_with_multibyte_boundary_content_does_not_crash_server() {
    // Sanity: confirm the fixture actually lands a multi-byte char across
    // the byte-57 cut, so this test stays grounded in the real bug. If
    // the constant drifts and byte 57 becomes an ASCII boundary, the
    // panic path is not being exercised and the test is worthless.
    assert!(
        !CRASH_FIXTURE.is_char_boundary(57),
        "fixture lost its mid-char cut; test no longer reproduces the 2026-04-16 crash"
    );
    assert!(
        CRASH_FIXTURE.len() > 60,
        "fixture shorter than 60 bytes — preview(60) would return it unchanged"
    );

    let dir = TempDir::new().expect("tempdir");
    init_cas_dir(&dir);

    let mut client = McpClient::spawn(dir.path());
    client.initialize();

    // Step 1: seed the crashing entry. This call does not itself run
    // preview(60) — remember() only validates and stores.
    let remember = client.call_tool(
        "memory",
        json!({
            "action": "remember",
            "content": CRASH_FIXTURE,
            "entry_type": "learning",
            "tags": "cas-f5e4,regression,factory"
        }),
    );
    assert!(
        remember.error.is_none(),
        "memory.remember failed: {:?}",
        remember.error
    );

    // Step 2: trigger the panic path. Before ad60df3, this call would
    // crash the server inside entry.preview(60) and the MCP client would
    // see the stdio pipe close. After the fix, truncate_preview walks
    // back to byte 55 and returns cleanly.
    let search = client.call_tool(
        "search",
        json!({
            "action": "search",
            "query": "factory daemon boot"
        }),
    );
    assert!(
        search.error.is_none(),
        "search.search must not error (preview truncation regression?): {:?}",
        search.error
    );
    let result = search.result.expect("search.search must return a result");
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .expect("search result must carry a content array");
    assert!(
        !content.is_empty(),
        "search result content must be non-empty; preview truncation may have returned early"
    );

    // Step 3: server-liveness probe on the SAME pipe. If the preview
    // panic killed the worker, the pipe is already closed and
    // send_request will panic from the read side. A Result-based call
    // chain wrapped in catch_unwind would let us turn that into a
    // clean failure message; the simpler contract (test panics with a
    // pipe-closed message) is enough for regression signal.
    let list = client.call_tool("memory", json!({ "action": "list", "limit": 1 }));
    assert!(
        list.error.is_none(),
        "memory.list after search must succeed — server appears to have died"
    );
}

