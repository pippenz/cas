# v2.25.0 — Slack release posts (#cas-internal)

## Post 1 — User

**v2.25.0 — mixed Claude + Codex teams now just work**

Before: running a team that mixed Claude and Codex agents was glitchy — handing work to an agent didn't always stick, status views were patchy for Codex, and finishing a Codex task could dead-end on instructions it couldn't follow. Now: a mixed Claude + Codex run goes smoothly from assignment to finish.

- Giving a task to a specific agent takes effect the first time — no more work that quietly stays unassigned.
- Status views show the same worktree/branch detail for Codex agents as for Claude.
- The guidance an agent gets while wrapping up a task now matches what that agent can actually do, so Codex work no longer stalls at the finish line.
- Bonus: browser (Playwright) testing no longer starts on its own during everyday work — it waits until you ask, so normal runs are faster.

## Post 2 — Dev

**v2.25.0 — harness-aware behavior across Claude + Codex**

Before: Codex-side agents drifted from Claude on assignment, status, and the verify/finish path — and a Codex-led run got handed `mcp__cas__*` tool names it can't call. Now: the right alias and the right surface for each harness.

- Assignment hints use agent display names instead of raw session IDs — assigning by ID no longer leaves a task stuck on the ready list.
- Status surfaces report worktree/branch/git for Codex agents, matching the Claude output.
- Verify-gate guidance points Codex at `mcp__cs__coordination` instead of a subagent flow that doesn't exist on that harness.
- `CAS_FACTORY_SUPERVISOR_CLI` is injected into the Codex `cs` MCP env, so verify/finish guidance resolves `mcp__cs__verification` for Codex-led runs; remaining hardcoded `mcp__cas__` alias sites in that guidance were swept, and free-text reasons embedded in suggested commands are quote-escaped.
- Codex recovery docs use the `mcp__cs__` alias, with a guardrail test that fails if the Claude and Codex copies drift.
- Test env-var guards snapshot and restore prior values instead of blind-removing them.
- The Nuxt + Playwright skill is now explicit opt-in (no proactive auto-trigger during dev/verification).
