# Changelog

All notable changes to CAS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Team Memories
- `cas cloud team set|show|clear` subcommands to configure the active team
  (UUID input; slug resolution deferred pending cloud-side endpoint).
- `cas memory share <id>|--since <duration>|--all [--dry-run]` for retroactive
  backfill of pre-existing personal memories to the team push queue.
- `cas memory unshare <id>` to mark a memory `share=Private` (blocks future
  team dual-enqueue; does not retract cloud-side copies).
- `share: Option<ShareScope>` (`Private`/`Team`) persisted on Entry, Rule,
  Skill, and Task via SQLite migrations `m037`/`m060`/`m082`/`m121`.
- Automatic dual-enqueue: when a team is configured via
  `cas cloud team set`, `cas memory remember` in any Project-scoped
  non-Preference context queues the entry to both personal and team
  push queues. `cas cloud sync` drains both.
- Coarse kill-switch: `cloud.json.team_auto_promote: false` disables the
  automatic promotion without requiring the team to be cleared.
- Integration test suite: `team_sync_test.rs`, `memory_share_test.rs`,
  `team_memories_e2e_test.rs` cover the full push → pull pipeline.

### Changed

- `cas cloud team-memories`'s "no team configured" error now correctly
  directs users to `cas cloud team set <uuid>` (previously referenced a
  non-existent subcommand with `<slug>` argument).

## [2.0.0] - 2026-04-12

### Added

#### Factory System
- Multi-agent factory with supervisor/worker architecture and isolated git worktrees.
- Director event system for task dispatch, worker lifecycle, and epic completion notifications.
- Worker startup confirmation flag to detect crash-on-startup failures.
- Orphaned task reclamation — supervisor can claim tasks from dead workers.
- Coordinator messaging system with priority levels, delivery confirmation, and outbox replay.
- Verification jail exemption for factory workers to prevent universal tool blocking.
- Worker idle/stale notification dedup and suppression.
- Minions theme with ASCII art and themed boot screen for factory workers.

#### Cloud Sync
- Bidirectional cloud sync with Petra Stella Cloud — push/pull tasks, memories, rules.
- Cloud sync queue with shutdown drain, startup push, 10s idle gate, 60s interval.
- Circuit breaker for TLS retry spam with capped event buffer.
- `cas cloud projects` and `cas cloud team-memories` commands.
- `cas cloud purge-foreign` for orphaned dependency cleanup.
- Project-scoped pull requests to prevent cross-project data leaks.

#### MCP Proxy
- `cas-mcp-proxy` crate — proxies upstream MCP servers (Playwright, Neon, GitHub, Vercel, Context7) through CAS. Workers get 2 tools instead of 50+.
- Config-aware hot-reload for proxy server connections.
- Search with keyword matching and server filtering.
- Integration tests, catalog caching, and README.

#### TUI
- Tokyo Night theme variant.
- OSC 52 clipboard copy and auto-inject on image paste.
- `cas open` interactive TUI project picker.
- Tab forwarding to PTY for autocomplete (Ctrl+P for sidecar).
- Clipboard fallback via client-side write with visual feedback.
- Mouse click to focus panes, Ctrl+Arrow pane cycling, Shift+drag text selection.
- Native terminal selection (replaces custom selection implementation).

#### Compound Engineering
- `cas-code-review` skill — multi-persona code review with 7 reviewer personas (correctness, testing, maintainability, project-standards + conditional security, performance, adversarial). Includes bounded autofix loop, confidence gates, fingerprint dedup, and review-to-task routing.
- `cas-brainstorm` and `cas-ideate` skills for structured ideation.
- `git-history-analyzer` and `issue-intelligence-analyst` agent types.
- Multi-persona review merge pipeline with cross-reviewer agreement boost.
- Pre-insert memory overlap detection with configurable threshold actions.
- Implementation Unit Template for EPIC subtask specifications.
- `execution_note` field on tasks: `test-first`, `characterization-first`, `additive-only` postures with enforcement at close.

#### Skills & Agents
- Comprehensive `cas-worker` skill with build failure triage, MCP connectivity guidance, tool selection guide, context exhaustion detection, task reassignment protocol, and section reorder for critical-path-first flow.
- Adversarial supervisor posture with intake gate, scope lock, and rejection authority.
- Partnership posture for supervisor — counter-propose, trajectory gate, situational awareness.
- `cas-supervisor` skill with EPIC sizing heuristics, worker failure recovery, and merge conflict guidance.
- `cas-memory-management` skill with multi-file schema and overlap workflow.
- `cas-search` skill with filter grammar, code symbol search, and module-scoped candidate API.
- CODEMAP system — auto-maintained breadcrumb navigation map with structural change detection hooks.

#### Infrastructure
- Hetzner CCX23 provisioning script for remote CAS server (Ashburn VA).
- Slack bridge: Bolt app scaffolding with per-user daemon architecture, SSE adapter, message formatter, file upload passthrough with security sanitization.
- `cas-install.sh` — portable curl one-liner installer.
- WebSocket endpoint for factory daemon.
- SSE plain-text pane output and tail endpoint.
- Auto-attach prompt with `--attach`/`--new` flags for existing sessions.
- `cas serve` HTTP bridge for Slack integration.

#### Store & Performance
- Sequence table for ID generation (replaces per-insert MAX+LIKE scan).
- SQLite `prepare_cached()` for all statement caching.
- Jitter on SQLite write-retry backoff to break convoy pattern.
- Recursive CTE for dependency cycle-check (replaces iterative BFS).
- Tantivy IndexWriter caching (saves 50MB per write allocation).
- BM25 search index caching and QueryParser reuse.
- Batch code symbol DB inserts in indexing daemon.
- `ImmediateTx` wrapper for atomic store operations.

### Changed

- Bumped version to 2.0.0 with simplified release workflow targeting `pippenz/cas`.
- Config format migrated from YAML to TOML (automatic merge of stale settings).
- `project_canonical_id` derived from folder name instead of git remote URL (required on all cloud pushes).
- Default cloud sync interval reduced from 300s to 60s.
- MCP tool prefix standardized to `mcp__cas__`.
- Worker skill reordered for critical-path-first flow: Task Types and Execution Posture before close procedures.
- Code review section compressed from 65 to 30 lines — pipeline internals moved to `cas-code-review` skill.
- Rules section merged into Rules of Engagement; Valid Actions merged into Schema Cheat Sheet.
- Legacy `code-reviewer` agent deprecated in favor of `cas-code-review` skill.

### Fixed

- **TUI**: Off-by-one in Ghostty VT style run column indices clipping left edge of pane content. Tab click detection using variable-width positions instead of equal-width assumption. Scroll viewport double-compensation when Ghostty preserves viewport position. Task panel flashing empty due to read race between task list and dependency queries. Dark theme contrast — `border_default`, `border_muted`, `hint_description` bumped for readability. Epic state updated before filter in `refresh_data()`.
- **Factory**: Verification jail cascade where one task's pending verification blocked all tools. `CAS_FACTORY_MODE` phantom env var — `pre_tool.rs` required it alongside `CAS_AGENT_ROLE` but no code ever set it. Director dispatching blocked/closed tasks (terminal-status guard added). Supervisor self-verification deadlock. Worktree workers missing MCP access due to gitignored `.mcp.json`/`.claude/` (fixed with symlinks). Duplicate hooks causing PreToolUse errors (`cas hook cleanup` added).
- **Cloud**: WebSocket TLS for `tokio-tungstenite`. HTTP TLS for `ureq` client. Fallback `project_id` for filesystem-root CAS projects. 403/404 error handling with pluralized labels.
- **Store**: N+1 queries in `task_store.rs`. Unbounded `IN` clauses and `LIKE` scans. 8 excessive indexes dropped to reduce write amplification. Lease races and cleanup/prune methods with transaction safety.
- **Close**: Additive-only gate now diffs worker branch commits (not main). Skip close-gate checks for non-isolated tasks. Reject close when worker tree has uncommitted work. Status-update race condition where `status=blocked` overwrites concurrent supervisor close.
- **Other**: `rustls` CryptoProvider installed at startup to prevent daemon crash. Secrets moved from provision script to `~/.config/cas/env` (push protection). GitHub auth token used in self-update to avoid API rate limits.

## [1.0.0] - 2026-03-12

### Added
- Initial open-source release of CAS.
- Factory TUI screenshot in README.
- `.env.worktree.template` for worker environment setup.

### Changed
- Release workflow updated for GitHub Actions with Homebrew auto-update.
- MCP config sync added to `cas update` flow.

### Fixed
- Migration v165 crash when `verifications` table doesn't exist.
- Release workflow secret check moved from job-level to step script.

## [0.6.2] - 2026-02-25

### Added
- Interactive terminal dialog (Ctrl+T) in factory TUI with show/hide/kill.
- MCP proxy catalog caching for SessionStart context injection.
- Billing interval switching buttons (monthly/yearly) with savings display.
- Resume subscription button on cancellation notice.
- `cas changelog` command to show release notes from GitHub releases.

### Changed
- Cloud sync on MCP startup runs in background with 5s timeout (non-blocking).
- Heartbeat uses shorter 5s timeout and spawn_blocking to avoid stalling async loop.
- Refactored cloud routes: org_billing_settings → billing_settings, org_members → members.
- Release bump workflow now requires a matching CHANGELOG.md section.

### Fixed
- Debounced Ctrl+C interrupt to prevent accidental double-sends.
- Update version check now compares versions properly.
- Stripe portal return URL redirects back to billing page instead of settings.
- Removed duplicate type export in types/index.ts.

## [0.5.7] - 2026-02-15

### Fixed
- Avoided macOS factory startup crash by using subprocess daemon mode with attach/socket retries.
- Hardened UTF-8-safe truncation behavior in touched UI/tooling paths to prevent char-boundary panics.

### Changed
- Standardized release-train crate versions to `0.5.7`.

## [0.5.6] - 2026-02-15

### Fixed
- Cleared clippy warnings under `-D warnings` across touched workspace crates.

### Changed
- Standardized release-train crate versions to `0.5.6`.
- Updated local git hook rustfmt invocation to use Rust 2024 edition.

## [0.5.5] - 2026-02-15

### Changed
- Published `0.5.5` release and synchronized release-train crate versions.

## [0.5.4] - 2026-02-15

### Changed
- Improved Supabase auth login UX and callback branding.

## [0.5.3] - 2026-02-15

### Changed
- Initial release carrying Supabase auth login UX and callback branding improvements.

## [0.5.2] - 2026-02-13

### Changed
- Bumped release-train versions to `0.5.2`.

## [0.5.1] - 2026-02-11

### Fixed
- Fixed Sentry transport panic triggered during `cas login`.

## [0.5.0] - 2026-02-11

### Fixed
- Added missing Sentry transport feature to prevent login-time crash.

## [0.4.0] - 2026-01-10

### Added
- Consolidated MCP tool format with unified naming.
- Sort and task type filtering for MCP and CLI.
- ID-based search and CLI/MCP feature parity.
- Git worktree support for task isolation.
- Schema migration system for database upgrades.
- Verification system with task-based exit blocking.
- Statusbar anchoring support.

### Changed
- Extracted `cas-core` and `cas-mcp` crates for better modularity.
- Removed `#[tool_router]` macro from CasCore for compile-time improvement.
- MCP enabled by default in `cas init --yes`.
- Removed legacy MCP mode and added `list_changed` notifications.

### Fixed
- Removed duplicate store implementations from `cas-cli`.
- Fixed scope persistence in crate extraction.
- Task verifier now uses CLI and checks project rules.

## [0.3.0]

### Added
- Initial stable release with core functionality.

[Unreleased]: https://github.com/pippenz/cas/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/pippenz/cas/compare/v1.0...v2.0.0
[1.0.0]: https://github.com/pippenz/cas/compare/v0.6.2...v1.0
[0.6.2]: https://github.com/pippenz/cas/compare/v0.5.7...v0.6.2
[0.5.7]: https://github.com/pippenz/cas/compare/v0.5.6...v0.5.7
[0.5.6]: https://github.com/pippenz/cas/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/pippenz/cas/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/pippenz/cas/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/pippenz/cas/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/pippenz/cas/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/pippenz/cas/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/pippenz/cas/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/pippenz/cas/compare/v0.3.0...v0.4.0
