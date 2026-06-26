# BUG: `mcp__cas__rule action=create` fails with `UNIQUE constraint failed: rules.id`

**Reported:** 2026-06-26 (gabber-studio project)
**Severity:** High — the rule store is unwritable; users cannot create new CAS rules.

## Symptom
Every `mcp__cas__rule` `create` call returns:

```
MCP error -32603: Failed to create rule: database error: UNIQUE constraint failed: rules.id
```

Reproduced 3× with different content/tags/length. Deterministic, not flaky.

## Context
- `mcp__cas__rule action=list_all` shows **46 rules** (rule-001 … rule-046), many of them duplicate "Draft" rows (e.g. rule-037–rule-044 are all "Always use descriptive variable names in tests").
- `check_similar` works; `list_all` works. Only `create` fails.

## Likely cause
The `rules.id` generator appears to derive the next id from a counter/max that is **out of sync** with existing rows (e.g. computes `rule-0NN` that already exists, or a sequence not advanced past manually/duplicate-inserted rows). The many duplicate draft rows suggest prior partial inserts left the id allocator inconsistent.

## Impact
Users cannot persist rules via CAS. Workaround used in gabber-studio: stored the intended rule as a file-based auto-memory (`feedback_feature_flag_env_parity`) + a `.claude/rules/feature-flag-env-parity.md` file. The rule should be re-created properly via `mcp__cas__rule` once this is fixed.

## Suggested fix
- Make `rules.id` allocation collision-proof (UUID, or `MAX(id)+1` computed from the table at insert time inside the transaction, or rely on an autoincrement/sequence).
- Add a dedupe/repair migration for the existing duplicate draft rows.
- `create` should retry-on-conflict or surface a clearer error.
