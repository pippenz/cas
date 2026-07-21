# Close gates contradict: MERGE REQUIRED forces the state ZERO-COMMIT then rejects

**Date:** 2026-07-21
**Reporter:** nimble-octopus-55 (supervisor, ozer, factory session ozer-strong-jay-96)
**Severity:** major (every code-task close needs a supervisor bypass — 2/2 today)

## Symptom
Worker close on a code task is a guaranteed two-failure sequence:
1. Worker finishes, attempts `task action=close` → rejected: **MERGE REQUIRED**
   (worker branch has commits not on the epic branch).
2. Supervisor merges worker branch into the epic branch (--no-ff), pushes.
3. Worker retries close → rejected: **ZERO-COMMIT CLOSE ON CODE TASK — 0 commits
   on the worker branch** — because the merge absorbed them, the branch now has
   0 commits unique vs the epic branch.
4. Only path left: supervisor close with `bypass_code_review=true`.

Observed on cas-a889 (e7e354bc → merge 90734cff) and cas-8b07 (ae042b26 →
merge b0c53e78), same session, 100% reproduction.

## Expected
The ZERO-COMMIT check should count commits REACHABLE from the epic branch that
were authored on the worker branch (e.g. `git log epic..` is the wrong
direction; use merge-base ancestry of the worker's recorded commits), or the
close flow should record the commit SHAs at the MERGE REQUIRED rejection and
accept them as satisfied once they become ancestors of the epic branch.

## Notes
Known-adjacent: cas-52ec was the same false-positive pattern. The bypass
workaround functions but downgrades the code-review gate to manual supervisor
discipline on every single code task, which is exactly what the gate exists to
enforce automatically.


## Completion

- **completed:** 2026-07-21
- **epic:** cas-887b — Factory reliability: open docs/requests bugs → main
- **completed_by:** cas-127f
- **status:** Fixed on epic tip; report archived from `docs/requests/`.
