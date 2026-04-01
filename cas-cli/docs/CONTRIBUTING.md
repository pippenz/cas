# Contributing to CAS

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

## Skill & Rule Sync

CAS auto-syncs rules to `.claude/rules/` and skills to `.claude/skills/` as SKILL.md files with YAML frontmatter. The sync logic lives in `cas-cli/src/sync/`. Rule promotion: Draft -> Proven via `mcp__cas__rule action=helpful`.
