# spawn_workers task_id pre-assignment not applied at worker boot

**Date:** 2026-07-21
**Reporter:** nimble-octopus-55 (supervisor, ozer project)
**Severity:** minor (workaround exists) / annoying (breaks unattended assignment)

## Symptom
`mcp__cas__coordination action=spawn_workers` with `task_id=<id>` acks with
"Task: <id> will be pre-assigned once the worker boots", but the worker boots
idle with 0 tasks and the director emits "idle with no assigned tasks" pings.
Supervisor must then manually `task action=update id=<id> assignee=<worker>`.

## Repro (2/2 today, same session)
1. Request 387: `spawn_workers count=1 cli=codex model=gpt-5.6-codex effort=medium isolate=true task_id=cas-7f61 worker_names=recipes-fixer` → booted, `agent_list` shows `recipes-fixer ... 0 tasks`.
2. Request 389: same but `model=gpt-5.6-sol worker_names=recipes-fixer-2` → same result.

Possibly related: request 377 (`claude` cli, `task_id=cas-5f5e`, no isolate) DID
end up working the task, but that worker may have self-claimed from ready rather
than receiving a pre-assignment — can't distinguish from my side.

## Expected
Worker boots with the task leased/assigned, starts without supervisor round-trip.

## Update (same day): the inverse bug on the claude path
For `cli=claude` spawns the pre-assignment DOES apply — and then SURVIVES
`shutdown_workers` issued before the worker starts. Observed: cas-3d23 stayed
assigned to dead worker sow-auditor and cas-5d7a to dead ui-bugs-fixer
(status=InProgress, no live agent, no active lease — `transfer` fails with
"No active lease found"; only `reset` clears them). Blocked reassignment with a
confusing ownership error on the next worker's `start`. shutdown_workers should
release pre-assignments/leases of workers that never started.

## Notes
- Both failing spawns used `cli=codex` + `isolate=true`; the succeeding one was
  `cli=claude` without isolate. Suspect the pre-assign hook isn't wired for the
  codex boot path or races the worktree provisioning.
- Workaround in use: explicit `task update assignee=` + coordination message
  after the idle ping.


## Completion

- **completed:** 2026-07-21
- **epic:** cas-887b — Factory reliability: open docs/requests bugs → main
- **completed_by:** cas-7a94
- **status:** Fixed on epic tip; report archived from `docs/requests/`.
