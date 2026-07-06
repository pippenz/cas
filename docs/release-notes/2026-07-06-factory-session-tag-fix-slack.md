# Slack draft — factory session tags actually apply now (2026-07-06)

Channel: #cas-internal (C0B44GUKDK2)

---

## User thread

**Top-level:**

Live on production — **User**
The multi-factory fix we shipped this morning wasn't actually switching on when you started a factory the normal way. Now it is: two factories on the same project truly can't see or steal each other's workers anymore.

**Threaded reply:**

Was → Now:
- **Was:** The session-isolation upgrade only activated on a rarely-used launch path. Starting a factory the normal way left every agent untagged, so two factories on one project could still cross-nudge each other's workers and steal each other's spawn requests — the exact chaos the upgrade was meant to end.
- **Now:** Every agent a factory starts — the supervisor and all workers, on either harness — is stamped with its factory session from the moment it launches. Isolation works on every launch path, no special flags, no workarounds.
- One restart of any running factory picks up the fix.

---

## Dev thread

**Top-level:**

Live on production — **Dev**
`CAS_FACTORY_SESSION` was only exported on the legacy daemonize path — the default fork-first launch spawned every PTY without it, so `agents.factory_session` stayed NULL and the m203/m204 scoping was inert. Env propagation is now explicit through `MuxConfig` → PTY env.

**Threaded reply:**

Was → Now:
- **Was:** Only `FactoryDaemon::new` (legacy path) did `set_var("CAS_FACTORY_SESSION")`. The default fork-first path (`DaemonInitPhase::run_with_progress`) spawned supervisor + workers via `Mux::factory` with no tag, and `FactoryApp::from_init_result` hardcoded `factory_session: None`. Registration read the missing env → NULL tags → session-scoped polls treated every agent as legacy.
- **Now:** `MuxConfig.factory_session` threads the session name into every PTY env explicitly (claude + codex, startup and dynamic spawns), codex workers also get it injected into the `cs` MCP server env via `-c mcp_servers.cs.env.*` (session name sanitized to a safe slug charset before TOML interpolation), and both fork-first daemon paths set the process env + app field so director/factory_ops/message consumers see it. Legacy path unchanged.
- Regression tests assert the env + codex MCP injection on both the startup and dynamic-spawn config paths (fail on the old code). Full `cargo test --no-fail-fast` green.
