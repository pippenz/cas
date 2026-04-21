# CODEMAP
> Auto-generated codebase map. Update after structural changes.
> Project: CAS (Coding Agent System) — Rust workspace + TS slack bridge

## Overview

cas-cli/src/         — Main `cas` binary: CLI, MCP server, hooks, TUI, stores
crates/              — Workspace library crates (types, store, core, search, factory, mux, pty, mcp, diffs, code, ghostty_vt)
slack-bridge/        — TypeScript Slack<->cas serve bridge (router + per-user daemons)
migration/           — Server/infra migration scripts + phase reports (Hetzner, cloud-fixes)
scripts/             — Install, release, build, worktree-boot, Hetzner provisioning
site/                — Landing page + The-System-CAS.pdf
homebrew/            — Homebrew formula
vendor/              — Vendored dependencies
docs/                — Roadmaps, brainstorms, spikes, cross-team requests inbox

## Detail

### cas-cli/src/ — Main binary modules

cli/                 — Clap command definitions and handlers (incl. codemap_cmd, project_overview_cmd, open)
cli/known_repos.rs   — `cas known-repos list/seed` — host repo registry CLI
cli/sweep.rs         — `cas sweep-all` + `cas worktree sweep [--all-repos] [--dry-run] [--salvage-dirty]`
cli/worktree.rs      — Worktree subcommand grouping (sweep, status, cleanup)
mcp/server/          — CasCore MCP server with cached OnceLock stores
mcp/tools/           — 55+ MCP tool handlers split into core/ and service/
mcp/daemon.rs        — Embedded background maintenance (embeddings, cleanup)
mcp/socket.rs        — Unix socket for agent notification events
store/               — Store wrappers: notifying_*, syncing_*, layered, detect
hooks/handlers/handlers_events/  — Event-time handlers (pre_tool incl. supervisor Agent(isolation:worktree) block, codemap, notifications, attribution)
hooks/handlers/handlers_middle/  — Mid-flow handlers (post_tool, prompt_capture, session_stop/)
hooks/handlers/handlers_session.rs — SessionStart/SessionEnd handlers
hooks/handlers/handlers_state.rs   — Per-session state tracking
hooks/handlers/handlers_tests/     — Hook handler test suite
hooks/scorer.rs      — Context item ranking for SessionStart injection
hooks/context.rs     — Context building strategies (standard, AI-powered, plan)
hooks/types.rs       — HookOutput / HookSpecificOutput schema types
migration/           — Forward-only schema migrations (m001-m199+); m199_known_repos = host repo registry table
ui/factory/          — Ratatui TUI for factory supervisor view
ui/factory/daemon/   — Daemon runtime: relay, ws_client, pane snapshots
ui/components/       — Reusable TUI widgets (panels, tables, markdown)
config/              — Configuration loading from .cas/config.yaml
orchestration/       — Agent name generation and orchestration logic
worktree/            — Git worktree management for factory workers
worktree/salvage.rs  — Tracked-diff + untracked patch writer for dirty-worktree reclaim
worktree/discovery.rs — Cross-repo discovery via host KnownRepoStore
worktree/sweep.rs    — Multi-repo sweep loop (used by cli/sweep.rs; opportunistic daemon trigger pending U3)
store/known_repos.rs — CLI-side glue opening the host-scoped KnownRepoStore
cloud/               — CAS Cloud sync (optional remote backup)
sync/                — Filesystem sync to .claude/rules/ and .claude/skills/
consolidation/       — Memory consolidation and decay
extraction/          — AI-powered extraction of observations into memory
hybrid_search/       — Tantivy-based full-text search (filter grammar, frontmatter)
rules/               — Rule extraction and suggestion from entry patterns
bridge/              — Local helper server for external tool integration
daemon/              — Background maintenance tasks (process observations)
notifications/       — Real-time notification system for TUI events
tracing/             — AI operation tracing (search, rules, extraction, API calls)
telemetry/           — Anonymous usage tracking via PostHog (opt-in)
builtins/agents/     — Built-in subagent defs (git-history-analyzer, issue-intelligence-analyst, etc.)
builtins/skills/     — Built-in skill templates (cas-brainstorm, cas-ideate, cas-code-review, cas-memory-management, etc.)
builtins/codex/      — Codex-flavored agent + skill mirrors of the above
harness_policy.rs    — Worker harness capability detection (subagents support, etc.)
duplicate_check.rs   — Stale `cas` binary detection on PATH; warns once at startup if mtimes diverge

### crates/ — Workspace library crates

cas-types/           — Entry, Task, Rule, Skill, Agent, Session, Spec, Loop, CodeReview types
cas-store/           — Store/TaskStore/RuleStore/SkillStore traits + SqliteStore
cas-store/src/known_repo_store.rs — Host-scoped `known_repos` table (~/.cas/cas.db); upsert/list API
cas-store/src/code_review/ — Code-review pipeline: autofix, base_sha, close_gate, merge, review_to_task
cas-core/            — Hooks framework, memory module (overlap detection), search index, skill/rule sync
cas-core/src/memory/ — Memory model + overlap detection for dedup
cas-search/          — Tantivy search index, BM25 scoring, query parsing
cas-factory/         — FactoryCore, factory config, director, session recording
cas-factory-protocol/ — WebSocket message protocol (supervisor <-> worker)
cas-mcp/             — MCP JSON-RPC types, tool schemas, resource models
cas-mcp-proxy/       — Upstream MCP server proxying (Playwright, GitHub, etc.)
cas-mux/             — Terminal mux layout, pane rendering, style runs
cas-pty/             — PTY spawn, resize, I/O management
cas-recording/       — Terminal session recording and playback (asciicast)
cas-code/            — Tree-sitter code analysis (symbols, structure)
cas-diffs/           — Unified diff parsing, syntax-highlighted rendering
cas-tui-test/        — TUI testing framework (snapshot, interaction)
ghostty_vt/          — Virtual terminal emulator (Rust wrapper)
ghostty_vt_sys/      — Ghostty VT FFI bindings (Zig-compiled)

### slack-bridge/src/ — TypeScript Slack bridge

router.ts            — HTTP/slack-events router, thread routing by `from` field
router-main.ts       — Router process entrypoint
daemon.ts            — Per-user cas serve adapter daemon
daemon-main.ts       — Daemon process entrypoint
session-adapter.ts   — cas serve HTTP adapter (resume/max-turns handling)
session-manager.ts   — Session lifecycle + persistence
commands.ts          — Slash command handlers
file-handler.ts      — Slack file upload/download
message-formatter.ts — Slack mrkdwn <-> cas text rendering
user-filter.ts       — Allowlist / auth
config.ts            — Config loading from daemon.env / router.env

### migration/ — Server migration artifacts (Hetzner cutover)

cloud-fixes/         — Cloud sync / dispatch patches collected during cutover
phase2-*             — Target provisioning logs + scripts
phase3-*             — Rsync migration logs + scripts
phase7-*             — Global state sync logs + scripts
phase8-*             — Final verification, env audit, completion report
systemd/             — Systemd unit templates for Hetzner
