# Changelog

All notable changes to CAS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/codingagentsystem/cas/compare/v0.5.7...HEAD
[0.5.7]: https://github.com/codingagentsystem/cas/compare/v0.5.6...v0.5.7
[0.5.6]: https://github.com/codingagentsystem/cas/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/codingagentsystem/cas/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/codingagentsystem/cas/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/codingagentsystem/cas/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/codingagentsystem/cas/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/codingagentsystem/cas/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/codingagentsystem/cas/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/codingagentsystem/cas/compare/v0.3.0...v0.4.0
