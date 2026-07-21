# ZERO-COMMIT CLOSE gate false-positive after supervisor merges the worker branch

**Date:** 2026-07-21
**Reporter:** warm-falcon-13 (supervisor, ozer project, session ozer-nimble-panda-75)
**Severity:** minor (supervisor escape-hatch close works) / hits every supervisor-merged task, so it's a per-task tax

## Symptom
Standard factory flow: worker pushes → close attempt → `MERGE REQUIRED` → supervisor merges worker branch into the epic branch → worker retries close. The retry is rejected with `ZERO-COMMIT CLOSE ON CODE TASK`.

Observed on cas-05af: worker commit 0b52e266 exists on factory/worker-android AND is merged into the epic (merge f7f16f40); fresh proof notes on the task; retry close → zero-commit rejection. Worker (correctly) refused to bypass and escalated; supervisor closed manually.

## Likely cause
The zero-commit check appears to count commits on the worker branch **not reachable from the epic branch**. Before the merge that count is >0 (hence MERGE REQUIRED); after the merge it becomes 0 — which the gate misreads as "this code task produced no commits" instead of "all commits are merged".

## Why it matters
The two gates are mutually contradictory in the canonical flow: MERGE REQUIRED forces the merge, and the merge triggers ZERO-COMMIT. Every supervisor-merged task therefore needs a supervisor close, defeating worker self-close and adding a round-trip per task (observed on cas-cab1, cas-8805b, cas-05af in one epic).

## Expected
Zero-commit detection should distinguish "no commits ever" from "commits exist and are already reachable from the epic branch" — e.g. count commits on the worker branch since the task started (task-window `git log`), or treat `merged_count > 0` as satisfying the gate.


## Completion

- **completed:** 2026-07-21
- **epic:** cas-887b — Factory reliability: open docs/requests bugs → main
- **completed_by:** cas-127f
- **status:** Fixed on epic tip; report archived from `docs/requests/`.
