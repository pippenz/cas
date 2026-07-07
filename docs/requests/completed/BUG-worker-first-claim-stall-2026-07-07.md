# BUG: fresh workers stall idle after spawn — pre-registration messages don't land and idle workers never self-claim ready tasks

**From:** Ozer supervisor session (clever-bear-67, 2026-07-07)
**Severity:** Medium — every fresh factory spawn loses minutes and requires manual supervisor intervention; chronic (documented as a known workaround pattern since at least 2026-06)
**Component:** factory worker bootstrap / coordination message delivery / task claiming

## What happened

Today's instance, fully reproducible from the ledger:

1. ~10:57 supervisor created three chained tasks on epic cas-82a94 (cas-d9e0 ready/unblocked, cas-e544 and cas-299f blocked behind it), then ran `spawn_workers count=1 isolate=true` (request ID 199).
2. Immediately after spawn confirmation, supervisor queued an assignment message to the worker by name (message ID 1594: "start with `task action=start id=cas-d9e0`", epic-branch instructions, proof commands). Tool replied "Message queued".
3. Worker `zealous-hawk-40` (codex CLI) came up healthy — worktree created, heartbeats fresh — and **sat idle**. The director then sent the supervisor two escalating notifications ("ready and waiting for tasks", "idle with no assigned tasks — Ready tasks exist"), and the worker itself messaged "active and ready. No open tasks are currently assigned to this agent."
4. Supervisor manually ran `task update id=cas-d9e0 assignee=zealous-hawk-40` + a second direct message repeating the instructions. Worker picked the task up within ~2 minutes and got on the correct epic base.

So: a ready, unblocked, P0 task existed; a spawn-time assignment message existed; and the worker still needed a human-in-the-loop re-ping to do anything. This is the "first-claim stall" pattern — it has hit enough sessions that "explicit re-ping after spawn" is recorded as standing supervisor guidance in project memory, which is a workaround masquerading as a convention.

## The defect

Three gaps compound:

1. **Pre-registration message delivery is unreliable (or silently pre-empted).** A message queued to a worker name between `spawn_workers` and the worker's registration/first-poll either never reaches the worker's prompt loop or arrives before the worker's task machinery is initialized. The supervisor gets "Message queued" either way — no signal that the target isn't registered yet, no redelivery-on-registration guarantee. Message 1594's instructions were only demonstrably acted on after being re-sent as message 1596 post-registration.
2. **Idle workers don't self-claim.** The worker's own idle report ("No open tasks are currently assigned to this agent") shows it filters by `assignee == me` only. It never consults the ready/available pool (`task action=available` exists precisely for this), even when the director can see "Ready tasks exist" in the same breath.
3. **The director escalates to the supervisor instead of resolving.** It correctly detects the condition (idle worker + ready tasks) and then sends a *human* a how-to ("Assign work: task action=update ..."). If the fix is mechanical enough to put in a message template, it's mechanical enough for the director to just do — assign highest-priority ready task, notify supervisor afterwards.

## Suggested fixes (any one of these kills the pattern; 1+2 together is the right shape)

1. **Deliver-on-register:** queue messages addressed to a not-yet-registered worker name and flush them into the worker's prompt loop as part of registration, after task machinery is up. Make the `message` tool result honest: "queued (target not yet registered — will deliver on registration)" vs "delivered".
2. **Idle self-claim:** when a worker's idle loop finds no assigned tasks, have it call `task action=available` (scoped to the focused epic if `focus_epic` is set) and claim the highest-priority unblocked task before declaring itself idle. The claim/lease machinery already exists.
3. **Spawn-with-assignment:** `spawn_workers` already accepts a `task_id` param in its schema — either wire it so spawned workers boot with the task pre-assigned (eliminating the race entirely), or remove the param so it stops implying this works.
4. **Director auto-assign:** as a fallback, the "worker idle + ready tasks exist" detector should assign rather than advise, and its notification to the supervisor should read "assigned cas-XXXX to worker-Y" instead of a manual runbook.

## Impact if unfixed

Every factory session pays a fixed tax: spawn → wait → notice the stall (or get director-nagged) → manual assign → re-send instructions. Worse, the failure is silent from the supervisor's seat — "Message queued" reads as success, so supervisors who *haven't* internalized the workaround lose 10+ minutes before investigating, and the spawn-time instructions (branch base, proof commands) may never reach the worker at all, producing work on the wrong base rather than no work.

## Related

- Project memory "Factory workers reliable for build, unreliable for skill-invoking review" (2026-06) — already documents "first-claim stall pattern requires explicit re-ping" as standing guidance.
- BUG-director-stall-detector-false-alarms (2026-07-07) — same theme from the opposite direction: director notifications asserting worker state without acting on (or verifying) ground truth.
