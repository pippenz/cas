//! Regression test for cas-f5e4 / EPIC cas-c351.
//!
//! On 2026-04-16 `cas serve` crashed on every `search` call that surfaced
//! the "CAS factory daemon boot order" entry. The root cause was six
//! open-coded `&s[..max_len-3]` preview-truncation paths on `Entry`,
//! `Rule`, `Skill`, `Spec`, `Prompt`, and `CommitLink`: the cut landed
//! inside the three bytes of `→` and Rust's string slice panicked
//! because the byte index was not a char boundary. The panic unwound
//! the Tokio worker, closed the stdio transport, and the MCP client saw
//! nothing but "Connection closed". Four auto-respawns later Claude
//! Code marked `cas serve` dead for the session.
//!
//! The fix (commit ad60df3) centralised the truncation into
//! `cas_types::preview::truncate_preview`, which walks back from the
//! target index with `is_char_boundary` until it finds a safe cut. The
//! function itself has unit tests in the `cas-types` crate, including
//! the exact 2026-04-16 fixture.
//!
//! This file reproduces the original failure end-to-end over the MCP
//! stdio transport against a real `cas serve` subprocess. If `ad60df3`
//! is ever reverted, the preview panic either unwinds the server (pre
//! cas-a436) or surfaces as an INTERNAL_ERROR from the A2 panic catcher
//! (post cas-a436). Either way this test fails — the `search.error`
//! assertion catches the A2-wrapped case; the liveness probe catches
//! the server-death case.
//!
//! # Test shape
//!
//! 1. Initialise a fresh `.cas` tempdir.
//! 2. Spawn `cas serve` and complete the MCP initialize handshake.
//! 3. Seed the crashing entry via `memory.remember` with
//!    `bypass_overlap=true` so overlap detection cannot short-circuit
//!    the store.
//! 4. Call `search.search`. Assert: no JSON-RPC error, response is not
//!    an `is_error: true` tool-error envelope, and the content text
//!    carries a recognisable substring of the seeded fixture — proving
//!    the BM25 index actually surfaced the entry and `Entry::preview`
//!    ran.
//! 5. Call `memory.list` on the **same** stdio pipe as a liveness
//!    probe for the pre-A2 failure mode (server process death).
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
/// index 57 lands inside the three bytes of `→`. Changing this string
/// means you are no longer reproducing the real regression — introduce
/// a second fixture instead of mutating this one.
const CRASH_FIXTURE: &str =
    "CAS factory daemon boot order: `build_configs_for_mux` → `FactoryApp::new` (spawns PTYs)";

/// ASCII substring of [`CRASH_FIXTURE`] that the BM25 preview must
/// include when the entry is surfaced. Sits before the multi-byte cut
/// so it survives any `truncate_preview(..., 60)` output, and is
/// distinctive enough that a stray "no results found" response cannot
/// contain it by accident.
const FIXTURE_PREVIEW_MARKER: &str = "build_configs_for_mux";

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
#[allow(dead_code)] // fields are consumed by {:?} in assertion messages
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // fields are consumed by {:?} in assertion messages
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

struct McpClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpClient {
    /// Spawn `cas serve` in `cas_dir`. Isolation from the host `.cas`
    /// relies on `current_dir` + `scrub_cas_env` — the server does a
    /// cwd walk via `find_cas_root` after the inherited CAS_* env vars
    /// are cleared, so only the child's tempdir is reachable.
    fn spawn(cas_dir: &std::path::Path) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cas"));
        cmd.arg("serve")
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

        // Skip notifications (no id) and any non-matching responses
        // (e.g., server-initiated requests per later MCP spec
        // revisions) until we see our matching response.
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .expect("read stdout — server pipe closed? likely preview() panic");
            assert!(n > 0, "stdout EOF before response — server died");

            let resp: JsonRpcResponse = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("parse response '{line}': {e}"));
            assert_eq!(resp.jsonrpc, "2.0", "invalid JSON-RPC version");
            match resp.id {
                Some(rid) if rid == id => return resp,
                Some(_) | None => continue,
            }
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
///
/// Intentionally duplicated from mcp_protocol_test.rs — a shared
/// test-util module is a follow-up refactor on EPIC cas-c351. Keep
/// both lists in sync when adding new CAS_* vars.
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

/// Collect the concatenated `text` from every text-content block in a
/// `tools/call` result. Used to assert that the BM25-surfaced preview
/// actually contains the fixture marker — the discriminating signal
/// that proves `Entry::preview(60)` ran on our seeded entry.
fn collect_response_text(result: &Value) -> String {
    let Some(arr) = result.get("content").and_then(|c| c.as_array()) else {
        return String::new();
    };
    arr.iter()
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Check the `is_error: true` flag that the tool-call result envelope
/// uses to signal tool-level failures (distinct from JSON-RPC errors,
/// which surface in the `.error` field).
fn is_tool_error(result: &Value) -> bool {
    result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false)
}

// ============================================================================
// The regression
// ============================================================================

#[test]
fn search_with_multibyte_boundary_content_does_not_crash_server() {
    // Sanity: confirm the fixture actually lands a multi-byte char
    // across the byte-57 cut, so this test stays grounded in the real
    // bug. If the constant drifts and byte 57 becomes an ASCII
    // boundary, the panic path is not being exercised and the test
    // would be worthless.
    assert!(
        !CRASH_FIXTURE.is_char_boundary(57),
        "fixture lost its mid-char cut; test no longer reproduces the 2026-04-16 crash"
    );
    assert!(
        CRASH_FIXTURE.len() > 60,
        "fixture shorter than 60 bytes — preview(60) would return it unchanged"
    );
    // FIXTURE_PREVIEW_MARKER must appear in the pre-cut prefix so a
    // well-behaved `truncate_preview(..., 60)` still surfaces it.
    let prefix_end = {
        let mut e = 57usize.min(CRASH_FIXTURE.len());
        while e > 0 && !CRASH_FIXTURE.is_char_boundary(e) {
            e -= 1;
        }
        e
    };
    assert!(
        CRASH_FIXTURE[..prefix_end].contains(FIXTURE_PREVIEW_MARKER),
        "preview marker not in pre-cut prefix — assertion would miss even on a correct preview"
    );

    let dir = TempDir::new().expect("tempdir");
    init_cas_dir(&dir);

    let mut client = McpClient::spawn(dir.path());
    client.initialize();

    // Step 1: seed the crashing entry. `bypass_overlap=true` keeps the
    // test focused on the preview regression — without the bypass, an
    // overlap check could in principle short-circuit the store with
    // an `is_error: true` tool-result envelope that carries no
    // JSON-RPC error, making the seed silently no-op.
    let remember = client.call_tool(
        "memory",
        json!({
            "action": "remember",
            "content": CRASH_FIXTURE,
            "entry_type": "learning",
            "tags": "cas-f5e4,regression,factory",
            "bypass_overlap": true
        }),
    );
    assert!(
        remember.error.is_none(),
        "memory.remember failed at JSON-RPC layer: {:?}",
        remember.error
    );
    let remember_result = remember
        .result
        .as_ref()
        .expect("memory.remember must return a result");
    assert!(
        !is_tool_error(remember_result),
        "memory.remember returned a tool-level error: {}",
        collect_response_text(remember_result)
    );

    // Step 2: trigger the panic path. Before ad60df3, this call would
    // crash the server inside entry.preview(60). After ad60df3, the
    // truncate walks back to byte 55 and returns cleanly. If cas-a436
    // is in place but ad60df3 is reverted, the panic is caught and
    // surfaced as INTERNAL_ERROR — still a test failure.
    let search = client.call_tool(
        "search",
        json!({
            "action": "search",
            "query": "factory daemon boot"
        }),
    );
    assert!(
        search.error.is_none(),
        "search.search errored at JSON-RPC layer (preview truncation regression?): {:?}",
        search.error
    );
    let search_result = search
        .result
        .as_ref()
        .expect("search.search must return a result");
    assert!(
        !is_tool_error(search_result),
        "search.search returned a tool-level error: {}",
        collect_response_text(search_result)
    );

    // Discriminating assertion: the BM25 index must have surfaced our
    // seeded entry and `Entry::preview(60)` must have emitted text
    // containing the fixture marker. Without this, a "no results
    // found" success response would pass the earlier checks and the
    // preview path would never run — the regression would not be
    // caught even if introduced.
    let search_text = collect_response_text(search_result);
    assert!(
        search_text.contains(FIXTURE_PREVIEW_MARKER),
        "search result did not surface the seeded fixture — BM25 miss or reader-lag; \
         preview() was not exercised, test is vacuously green. Response text:\n{search_text}"
    );

    // Step 3: server-liveness probe on the SAME pipe. With the A2
    // panic catcher shipped (cas-a436), a preview panic no longer
    // closes the pipe, so this probe is a belt-and-suspenders check
    // for the pre-A2 failure mode (worker panic crashes the whole
    // server). If cas-a436 were removed AND ad60df3 were reverted,
    // the pipe would be dead and this call would error from the read
    // side of send_request.
    let list = client.call_tool("memory", json!({ "action": "list", "limit": 1 }));
    assert!(
        list.error.is_none(),
        "memory.list after search must succeed — server appears to have died"
    );
}
