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
