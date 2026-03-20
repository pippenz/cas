# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.
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

## What is CAS

CAS (Coding Agent System) is a multi-agent coding factory and persistent context system for AI agents. Written in Rust.

Two core capabilities:
1. **Factory** — Terminal UI orchestrating multiple Claude Code instances in parallel via isolated git worktrees, with supervisor/worker coordination.
2. **Context System** — MCP server providing persistent memory, tasks, rules, skills, and search (55+ tools) backed by SQLite + Tantivy BM25 search.

## Build & Test

```bash
# Build (from repo root or cas-cli/)
cargo build                          # Dev build
cargo build --release                # Release build (LTO, strip)
cargo build --profile release-fast   # Fast release (thin LTO, 16 codegen units)

# Run all tests
cargo test

# Run a single test by name
cargo test test_name

# Run tests in a specific file
cargo test --test cli_test

# Run tests matching a pattern
cargo test migration

# Run benchmarks
cargo bench --bench code_indexing

# Run with specific feature
cargo test --features claude_rs_e2e
```

The `mcp-server` feature is enabled by default. The binary is `cas` (both lib and bin targets in `cas-cli/`).

The build script (`cas-cli/build.rs`) embeds git hash and build date into the binary, and loads telemetry keys from `.env` if present.

## Architecture

### Workspace Layout

The root `Cargo.toml` defines a workspace. `cas-cli/` is the main binary crate; `crates/` contains library crates.

**Core data flow**: CLI commands and MCP tool calls both go through the store trait abstractions in `cas-cli/src/store/`, which wraps `cas-store` (SQLite) with notification and sync layers.

### cas-cli (main crate) — `cas-cli/src/`

| Module | Purpose |
|--------|---------|
| `main.rs` / `lib.rs` | Entry point, module declarations |
| `cli/` | Clap command definitions and handlers. `mod.rs` has the `Commands` enum — add new subcommands here. |
| `mcp/` | MCP server: `server/` (CasCore with cached OnceLock stores), `tools/` (55 tool handlers split into `core/` and `service/`), `daemon.rs` (embedded background maintenance), `socket.rs` (notification socket) |
| `store/` | Re-exports from `cas-store` + wrappers: `notifying_*.rs` (emit change notifications), `syncing_*.rs` (sync to `.claude/` filesystem), `layered.rs` (project + global store composition), `detect.rs` (find `.cas/` root) |
| `hooks/` | Claude Code hook event handlers (SessionStart, Stop, PostToolUse, etc.). `handlers/` has session, state, event, and middleware handlers. `scorer.rs` ranks context items for injection. |
| `migration/` | Forward-only schema migrations. `migrations/` has individual migration files (m001-m182+). `detector.rs` introspects existing schema. |
| `ui/` | Ratatui TUI components for factory view: `factory/`, `components/`, `widgets/`, `theme/`, `markdown/` |
| `config/` | Configuration loading from `.cas/config.yaml` |
| `orchestration/` | Agent name generation and orchestration logic |
| `worktree/` | Git worktree management for factory workers |
| `consolidation/` | Memory consolidation and decay |
| `extraction/` | AI-powered extraction of observations into structured memory |
| `bridge/` | Local helper server for external tool integration |
| `cloud/` | CAS Cloud sync (optional) |
| `sync/` | Filesystem sync to `.claude/rules/` and `.claude/skills/` |

### Workspace Crates — `crates/`

| Crate | Purpose |
|-------|---------|
| `cas-types` | Shared data types (Entry, Task, Rule, Skill, Agent, etc.) |
| `cas-store` | SQLite storage layer — trait definitions (`Store`, `TaskStore`, `RuleStore`, etc.) and `SqliteStore` implementation |
| `cas-search` | Full-text search via Tantivy (BM25 scoring) |
| `cas-core` | Core business logic, hooks framework, search index abstraction, skill/rule syncing |
| `cas-mcp` | MCP protocol types and request/response models |
| `cas-factory` | Factory session lifecycle: `FactoryCore`, config, director, recording, notifications |
| `cas-factory-protocol` | WebSocket message protocol between supervisor and worker agents |
| `cas-mux` | Terminal multiplexer layout and rendering (side-by-side/tabbed agent views) |
| `cas-pty` | PTY management for agent terminal sessions |
| `cas-recording` | Terminal session recording and playback |
| `cas-code` | Code analysis via tree-sitter |
| `cas-diffs` | Diff parsing, rendering, syntax highlighting |
| `cas-tui-test` | TUI testing framework |
| `ghostty_vt` / `ghostty_vt_sys` | Virtual terminal parser (based on Ghostty) |

### Key Patterns

**Store trait hierarchy**: `cas-store` defines traits (`Store`, `TaskStore`, `RuleStore`, `SkillStore`, `EntityStore`, `AgentStore`, `VerificationStore`, `WorktreeStore`). `SqliteStore` implements all of them. `cas-cli/src/store/` wraps these with notification and sync decorators.

**CasCore (MCP server)**: Lives in `cas-cli/src/mcp/server/mod.rs`. Caches all store instances in `OnceLock` fields — each store type opened exactly once per server lifetime. Has an embedded daemon for background maintenance (embedding generation every 2min, full maintenance every 30min).

**CasContext**: In `cas-cli/src/store/mod.rs`. Resolves the `.cas/` directory once at CLI entry points and passes it through — enables deterministic test behavior.

**Hook scoring**: `cas-cli/src/hooks/scorer.rs` ranks context items (memories, tasks, rules, skills) by relevance for injection into SessionStart context, staying within a token budget.

## Adding Features

**New CLI command**: Add variant to `Commands` enum in `cas-cli/src/cli/mod.rs`, create handler file in `cli/`, add integration test in `tests/cli_test.rs`.

**New MCP tool**: Add handler in `cas-cli/src/mcp/tools/core/` (data tools) or `cas-cli/src/mcp/tools/service/` (orchestration tools). Request types go in `cas-cli/src/mcp/tools/types/`. Register in the tool list via the `CasService` impl.

**New migration**: Create file in `cas-cli/src/migration/migrations/` following naming convention `m{NNN}_{table}_{description}.rs`. Add to the `MIGRATIONS` array in `migrations/mod.rs`. Each migration needs: unique sequential ID, up SQL, and a detect query. See `cas-cli/docs/MIGRATIONS.md` for full details. Migration ID ranges: Entries 1-50, Rules 51-70, Skills 71-90, Agents 91-110, Entities/Worktrees 111+, Verification 131+, Loops/Events 151+.

## Testing

Integration tests are in `cas-cli/tests/`. Key test files:
- `cli_test.rs` — CLI command integration tests
- `mcp_tools_test.rs` — MCP tool handler tests
- `mcp_protocol_test.rs` — MCP protocol compliance
- `factory_server_test.rs` — Factory WebSocket server tests
- `distributed_factory_test.rs` — Multi-agent factory tests
- `proptest_test.rs` — Property-based tests
- `e2e_test.rs` / `e2e/` — End-to-end tests

Dev dependencies include: `insta` (snapshot testing), `wiremock` (HTTP mocking), `rstest` (parametrized tests), `proptest` (property-based), `criterion` (benchmarks), `cas-tui-test` (TUI testing).

## Rust Version

Minimum supported Rust version: **1.85** (edition 2024).

## Skill & Rule Sync

CAS auto-syncs rules to `.claude/rules/` and skills to `.claude/skills/` as SKILL.md files with YAML frontmatter. The sync logic lives in `cas-cli/src/sync/`. Rule promotion: Draft → Proven via `mcp__cas__rule action=helpful`.
