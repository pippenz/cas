# Filing CAS-system bugs

A standing directive: **file every CAS-system bug you observe as a tracked task, by reflex.** Do not just mention it in chat, defer it to "later", or "report it upstream". A CAS-system bug you noticed but didn't capture is a bug that resurfaces.

"CAS-system" means a defect in CAS itself: the verifier, hooks, factory/director orchestration, MCP dispatch, the task-verifier agent, worker/supervisor prompts, or builtin skills — regardless of which downstream project (gabber-studio, OpenClaw, etc.) surfaced it.

## Where the bug goes depends on the repo

- **If this repo is cas-src (the CAS source):** create an in-repo task (`task action=create task_type=bug`) and let a worker fix it here. Other projects consume CAS; they do not modify it — the fix lands in cas-src. This mirrors cas-src's `CLAUDE.md` → "## CAS system bugs are in-repo fixes".
- **In any other project:** drop a `BUG-<slug>.md` file in cas-src's `docs/requests/` inbox (the cross-team relay convention), then continue your own work — don't block your project's task on the CAS fix.

Never silently ignore a CAS-system bug, leave it chat-only, or treat cas-src as an untouchable external dependency. Capture it, route it, move on.
