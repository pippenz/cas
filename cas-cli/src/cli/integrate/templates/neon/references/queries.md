# Neon — Common Queries & Branch Routing

Companion reference for the `neon-database` skill. Routing snippets use the
IDs from `SKILL.md`'s `<!-- keep neon-ids -->` block — keep them in sync.

## Branch routing recipes

Default-branch (production) reads:

```
mcp__neon__run_sql({
  projectId: "<projectId>",
  databaseName: "<databaseName>",
  sql: "SELECT 1;"
})
```

Non-default branch (staging / dev / preview) — always pass `branchId`:

```
mcp__neon__run_sql({
  projectId: "<projectId>",
  databaseName: "<databaseName>",
  branchId: "<branchId>",
  sql: "SELECT 1;"
})
```

## Schema introspection

```
mcp__neon__describe_table_schema({
  projectId: "<projectId>",
  databaseName: "<databaseName>",
  tableName: "users"
})
```

## Migration (3-step flow)

1. `mcp__neon__prepare_database_migration` — runs `migrationSql` on a temp branch.
2. Verify on the temp branch with `run_sql` / `describe_table_schema`.
3. `mcp__neon__complete_database_migration` — promotes the change to the default branch.

Never skip step 2. Never run step 3 directly against production without verifying step 2.

## Slow-query inspection

```
mcp__neon__list_slow_queries({ projectId: "<projectId>" })
mcp__neon__explain_sql_statement({
  projectId: "<projectId>",
  databaseName: "<databaseName>",
  sql: "SELECT ..."
})
```

## Branch lifecycle

- `mcp__neon__create_branch` — feature branches off a parent branch.
- `mcp__neon__describe_branch` — confirm a branch still exists (used by `cas integrate neon verify`).
- `mcp__neon__reset_from_parent` — discard branch changes; return to parent state.
- `mcp__neon__delete_branch` — clean up after merging.

## Connection strings

Use `mcp__neon__get_connection_string` only when external tools need a URL;
prefer MCP tool calls for inspection/migration so credentials stay scoped.
