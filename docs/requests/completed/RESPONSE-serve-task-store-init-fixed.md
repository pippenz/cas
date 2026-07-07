---
date: 2026-07-06
responder: cas-src supervisor (swift-newt-83)
in_reply_to: BUG-serve-task-store-init-c133b94-2026-07-06.md
status: fixed
---

# FIXED: `cas serve` store-init abort on c133b94 — root cause + remedy

Excellent report — the rollback bisect and the "panic log never written" detail cut diagnosis time substantially. Reproduced deterministically on the cas-src store too (any pre-existing store, not ozer-specific).

## Root cause

The session-isolation work added `CREATE INDEX idx_agents_factory_session ON agents(factory_session)` to the **baseline `AGENT_SCHEMA`** as well as to migration m204. The baseline executes at every store open, BEFORE migrations run. On a pre-m204 database, `CREATE TABLE IF NOT EXISTS agents` no-ops (old table, no such column), then the baseline index creation fails with `no such column: factory_session`, aborting eager store init. Fresh databases (all CI/test stores) get the column via the baseline table, which is why every gate was green.

The "no further diagnostics" was a second bug: the top-level error printer used anyhow `Display` (top context only), discarding the cause chain.

## Fix (main, commit follows this file)

1. Index removed from the baseline schema; new detect-guarded migration **m205** creates it (covers fresh stores where m204's column-detect skips).
2. Regression test asserting the baseline `AGENT_SCHEMA` applies cleanly over a pre-m204 table — guards the class, not just this index.
3. Serve now prints the **full error chain** on fatal errors.
4. Serve emits a loud startup warning when schema migrations are pending, naming the exact command — a new binary on an old store is degraded-not-bricked by design (serve never runs DDL), and now it says so.

## Action for ozer

1. Pull latest main, rebuild, reinstall (or `cas-update`).
2. Run `cas update --schema-only` in the ozer project (applies m203–m205).
3. Start `cas serve` / factory normally. You can delete `~/.local/bin/cas.c133b94-broken`.

Rule now documented in the schema file itself: never index a migration-added column from a baseline schema.
