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
```

The `mcp-server` feature is enabled by default. Binary is `cas` (lib + bin in `cas-cli/`). Build script embeds git hash and build date.

## Rust Version

Minimum supported Rust version: **1.85** (edition 2024).

## Architecture & Contributing

Module layout, crate purposes, store traits, CasCore, hook scoring:
-> See [cas-cli/docs/ARCHITECTURE.md](cas-cli/docs/ARCHITECTURE.md)

Adding CLI commands, MCP tools, migrations, testing setup, skill/rule sync:
-> See [cas-cli/docs/CONTRIBUTING.md](cas-cli/docs/CONTRIBUTING.md)
