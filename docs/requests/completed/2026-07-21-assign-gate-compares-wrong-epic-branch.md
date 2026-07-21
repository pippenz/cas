# Worker-assignment freshness gate compares against an unrelated epic's branch

**Date:** 2026-07-21
**Reporter:** nimble-octopus-55 (supervisor, ozer project, factory session ozer-strong-jay-96)
**Severity:** major (blocks supervisor task assignment entirely for affected worker)

## Symptom
`task action=update id=cas-3d23 assignee=telehealth-auditor` fails:

    Cannot assign to worker 'telehealth-auditor': 10 commits behind
    epic/widget-parity-saffron-light-mode-jill-request-cas-960d (threshold: 1).
    Ask the worker to rebase: `git rebase epic/widget-parity-saffron-light-mode-jill-request-cas-960d`

But cas-3d23 belongs to epic cas-60e3 (branch epic/jill-feedback-wave-2026-07-17-20-investigate-answe-cas-60e3). cas-960d is a different, older epic. The suggested remediation (rebase the worker onto the unrelated epic branch) would actively pollute the worker's branch.

## What didn't fix it
`coordination action=focus_epic id=cas-60e3` (pin accepted: "Pinned epic focus to cas-60e3 for factory session ozer-strong-jay-96") → identical error on retry immediately after.

## Context
- Multiple epics concurrently active in this repo (cas-60e3 jill-feedback, cas-96cb sleep-rhythm with its own supervisor warm-falcon-13 + 4 codex workers, and evidently cas-960d widget-parity with a live branch).
- Worker telehealth-auditor's branch: factory/telehealth-auditor, based on staging @ 0cfa3864, its work already merged into the cas-60e3 epic branch.
- Guess: the gate resolves "the epic branch" globally (most recently active? lexical? last-created?) instead of from the target task's parent epic. Possibly related to the c133b94 regression cluster.

## Expected
Freshness gate should compare the worker's branch against the epic branch of the TASK being assigned (cas-3d23 → cas-60e3), or honor the session's pinned focus epic.

## Related same-day symptom: reminders also cross epic scopes
A `remind` registered by the sleep-rhythm factory ("Reminder #27: Sleep-rhythm
task completed — run the review gate (…no crossfade settings…)") was delivered
to THIS supervisor, triggered by MY worker completing cas-3d23 (jill-feedback
epic, zero overlap with sleep-rhythm). remind_event filters appear to match on
bare `task_completed` across all factory sessions in the repo instead of the
registering session/epic. Same root theme as the assignment gate: multi-epic
concurrency in one repo isn't scoped.

## Same-day symptom #3: normal-priority message delivery stalls while workers idle
Workers heartbeat fresh (4-22s) but sit idle 10+ min with supervisor messages
stuck at status=pending (IDs 3631, 3633 verified pending via message_status).
Idle workers appear to never poll the inbox, and pending messages are only
injected on a poll → deadlock: worker idles waiting for work, work waits in a
queue the idle worker never reads. `interrupt`/urgent=true delivery works
(breaks the turn and injects), which is the workaround, but it discards
in-flight state and shouldn't be the only reliable channel. User-visible
symptom: "no workers are working."

## Workaround in use
Worker self-claims via `task action=start id=<task>` (different code path; pending confirmation it bypasses the gate).


## Completion

- **completed:** 2026-07-21
- **epic:** cas-887b — Factory reliability: open docs/requests bugs → main
- **completed_by:** cas-44e9
- **status:** Fixed on epic tip; report archived from `docs/requests/`.
