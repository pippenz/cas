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

## Output hygiene — avoid Claude Code Ink crash

Claude Code's React-Ink UI throws `<Box> can't be nested inside <Text>` when streamed markdown produces a Box-in-Text layout. The process stays alive (Bun keeps the event loop) but the pane is dead — tool calls after that point never complete. Until Claude Code ships a fix (cas-97ba tracks), avoid these output shapes when responding in chat:

- **Do not echo the contents of a markdown file back in your response** after writing it with `Write` — confirm with a short prose summary instead. Streaming long generated markdown (CODEMAP.md, PRODUCT_OVERVIEW.md, skill bodies, etc.) is a common trigger.
- **Avoid nested fenced code blocks** (a ` ```markdown ` block whose contents include headings, blockquotes, bullets, or a second fence). This is the most reproducible tripwire today. Describe the inner shape in prose or use backticks for inline samples.
- **Keep fenced blocks minimal in chat output** — use them for plain shell commands or short snippets, not for richly-structured markdown previews.

Writing to disk is always safe; the risk is only when the content streams back through the Ink renderer.

## CAS system bugs are in-repo fixes

This repo **is** the CAS source. When a bug is reported in the verifier, hooks, factory orchestration, MCP dispatch, the task-verifier agent, worker prompts, or built-in skills — regardless of which downstream project (gabber-studio, OpenClaw, etc.) surfaced it — the fix lands here as a Rust or markdown change via a task assigned to a worker. Do not file the bug with team-lead, do not "report upstream", do not treat cas-src IS CAS as an external dependency. Other projects consume CAS; they do not modify it. If you catch yourself wanting to escalate a CAS bug, stop and create the fix task in this repo instead.
