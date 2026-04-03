# CODEMAP
> Auto-generated codebase map. Update after structural changes.
> Project: CAS (Coding Agent System) — Rust workspace

## Overview

cas-cli/src/         — Main binary: CLI commands, MCP server, hooks, TUI, stores
crates/cas-types/    — Shared data types (Entry, Task, Rule, Skill, Agent, etc.)
crates/cas-store/    — SQLite storage layer, trait definitions, SqliteStore impl
crates/cas-core/     — Core business logic, hooks framework, search index, sync
crates/cas-search/   — Full-text search via Tantivy (BM25 scoring)
crates/cas-factory/  — Factory session lifecycle, director, recording, notifications
crates/cas-mux/      — Terminal multiplexer layout and rendering (agent views)
crates/cas-pty/      — PTY management for agent terminal sessions
crates/cas-mcp/      — MCP protocol types and request/response models
crates/cas-diffs/    — Diff parsing, rendering, syntax highlighting
crates/cas-code/     — Code analysis via tree-sitter

## Detail

### cas-cli/src/ — Main binary modules

cli/                 — Clap command definitions and handlers (Commands enum)
mcp/server/          — CasCore MCP server with cached OnceLock stores
mcp/tools/           — 55+ MCP tool handlers split into core/ and service/
mcp/daemon.rs        — Embedded background maintenance (embeddings, cleanup)
mcp/socket.rs        — Unix socket for agent notification events
store/               — Store wrappers: notifying_*, syncing_*, layered, detect
hooks/handlers/      — Hook event handlers (SessionStart, Stop, PostToolUse, etc.)
hooks/scorer.rs      — Context item ranking for SessionStart injection
hooks/context/       — Context building strategies (standard, AI-powered, plan)
migration/           — Forward-only schema migrations (m001-m182+)
ui/factory/          — Ratatui TUI for factory supervisor view
ui/components/       — Reusable TUI widgets (panels, tables, markdown)
config/              — Configuration loading from .cas/config.yaml
orchestration/       — Agent name generation and orchestration logic
worktree/            — Git worktree management for factory workers
cloud/               — CAS Cloud sync (optional remote backup)
sync/                — Filesystem sync to .claude/rules/ and .claude/skills/
consolidation/       — Memory consolidation and decay
extraction/          — AI-powered extraction of observations into memory
hybrid_search/       — Tantivy-based full-text search integration
rules/               — Rule extraction and suggestion from entry patterns
bridge/              — Local helper server for external tool integration
daemon/              — Background maintenance tasks (process observations)
notifications/       — Real-time notification system for TUI events
tracing/             — AI operation tracing (search, rules, extraction, API calls)
telemetry/           — Anonymous usage tracking via PostHog (opt-in)
builtins/            — Built-in agent definitions and skill templates

### crates/ — Workspace library crates

cas-types/           — Entry, Task, Rule, Skill, Agent, Session, Spec, Loop types
cas-store/           — Store/TaskStore/RuleStore/SkillStore traits + SqliteStore
cas-core/            — Hooks framework, search index abstraction, skill/rule sync
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
