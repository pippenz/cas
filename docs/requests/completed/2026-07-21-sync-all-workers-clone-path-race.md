# sync_all_workers skips freshly-spawned workers: "missing clone_path metadata"

**Date:** 2026-07-21
**Reporter:** warm-falcon-13 (supervisor, ozer project, session ozer-nimble-panda-75)
**Severity:** minor (manual git fallback works) / hits the critical first minute of every epic

## Symptom
Immediately after `spawn_workers` (4 × codex, isolate=true) acked and `worker_status` already displayed all four workers WITH clone paths (`Clone: .../.cas/worktrees/worker-ios` etc.), a `sync_all_workers branch=epic/...` call skipped every worker:

```
Sync target: epic/epic-ozer-sleep-rhythm-routine-native-audio-engine-cas-96cb
Skipped:
  - worker-ios (missing clone_path metadata)
  - worker-android (missing clone_path metadata)
  - worker-backend (missing clone_path metadata)
  - worker-frontend (missing clone_path metadata)
```

So `worker_status` and `sync_all_workers` read different metadata stores (or the latter reads before registration completes — the workers had spawned <60s earlier).

## Why it matters
This is exactly the moment sync is needed: workers spawn based on trunk (staging @ 0cfa3864) while the epic branch already carries frozen contract files. If the supervisor doesn't notice the skip, all workers start WITHOUT the contracts they were told to implement.

## Expected
Either resolve clone_path the same way worker_status does, or return a retryable "registration in progress" status instead of a silent skip — a skip reads like "nothing to do".

## Workaround used
```
for w in worker-*; do
  git -C .cas/worktrees/$w fetch origin <epic-branch> -q
  git -C .cas/worktrees/$w merge --ff-only origin/<epic-branch>
done
```
(clean because fresh workers have zero commits; verified all four at the contracts commit afterwards).


## Completion

- **completed:** 2026-07-21
- **epic:** cas-887b — Factory reliability: open docs/requests bugs → main
- **completed_by:** cas-f53c
- **status:** Fixed on epic tip; report archived from `docs/requests/`.
