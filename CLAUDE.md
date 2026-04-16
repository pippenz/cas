<!-- CAS:BEGIN - This section is managed by CAS. Do not edit manually. -->
# IMPORTANT: USE CAS FOR TASK AND MEMORY MANAGEMENT

**DO NOT USE BUILT-IN TOOLS (TodoWrite, EnterPlanMode) FOR TASK TRACKING.**

Use CAS MCP tools instead:
- `mcp__cas__task` with action: create - Create tasks (NOT TodoWrite)
- `mcp__cas__task` with action: start/close - Manage task status
- `mcp__cas__task` with action: ready - See ready tasks
- `mcp__cas__memory` with action: remember - Store memories and learnings
- `mcp__cas__search` with action: search - Search all context

CAS provides persistent context across sessions. Built-in tools are ephemeral.
<!-- CAS:END -->

# CLAUDE.md

## Build & Test

```bash
cargo build                          # Dev build
cargo build --release                # Release build (LTO, strip)
cargo build --profile release-fast   # Fast release (thin LTO, 16 codegen units)
cargo test                           # All tests
cargo test test_name                 # Single test by name
cargo test --test cli_test           # Tests in a specific file
cargo bench --bench code_indexing    # Benchmarks
make test-release-panic              # Verify A2/A3/B3 panic isolation under release profiles
```

The `mcp-server` feature is enabled by default. Binary is `cas` (lib + bin in `cas-cli/`). Build script embeds git hash and build date.

**Build profiles must use `panic = "unwind"`.** The MCP tool-dispatch panic catcher (EPIC cas-c351) relies on `tokio::spawn` + `JoinError::is_panic`, which only observes a panic if the worker thread unwinds. A compile-time guard in `cas-cli/src/lib.rs` refuses non-test builds with `panic = "abort"` — do not work around it; the entire point of that catcher is to keep `cas serve` alive across handler bugs.

## Rust Version

Minimum supported Rust version: **1.85** (edition 2024).

## Architecture & Contributing

Module layout, crate purposes, store traits, CasCore, hook scoring:
-> See [cas-cli/docs/ARCHITECTURE.md](cas-cli/docs/ARCHITECTURE.md)

Adding CLI commands, MCP tools, migrations, testing setup, skill/rule sync:
-> See [cas-cli/docs/CONTRIBUTING.md](cas-cli/docs/CONTRIBUTING.md)

Codebase navigation map (breadcrumb index of all modules):
-> See [.claude/CODEMAP.md](.claude/CODEMAP.md)

## CAS system bugs are in-repo fixes

This repo **is** the CAS source. When a bug is reported in the verifier, hooks, factory orchestration, MCP dispatch, the task-verifier agent, worker prompts, or built-in skills — regardless of which downstream project (gabber-studio, OpenClaw, etc.) surfaced it — the fix lands here as a Rust or markdown change via a task assigned to a worker. Do not file the bug with team-lead, do not "report upstream", do not treat cas-src IS CAS as an external dependency. Other projects consume CAS; they do not modify it. If you catch yourself wanting to escalate a CAS bug, stop and create the fix task in this repo instead.
