# Factory multi-epic + close/merge reliability (2026-07-21)

Channel: #cas-internal (`C0B44GUKDK2`). Two top-level posts.

Shipped on `main` as merge `cf81ea6` (PR #51; cargo version still 2.27.0 — rebuild release binary to pick up).

---

## Post 1 — User

Factory used to fight concurrent work and “done” closes: assigning work could demand a rebase onto the wrong epic, finishing a code task meant merge-then-zero-commit thrash, mid-epic merges could delete a live worker’s home directory, and Codex workers looked “starved” while they were still thinking. Now multi-epic assignment, post-merge close, mid-epic merge, and Codex health checks behave like they mean it.

- Assigning a task compares freshness against *that* task’s epic, not some other active branch in the repo.
- After you merge a worker’s commits into the epic, close no longer pretends the task produced zero commits.
- Merging a worker mid-epic keeps their worktree and branch; cleanup is explicit when you want consume-on-merge.
- Codex workers report real process/transcript signals instead of false “starved” spam.
- Spawning with a task id actually hands the worker the task; shutdown releases ghost assignments.
- Right after spawn, sync no longer skips every worker as “missing clone path” when the worktree is already there.
- Reminder events stay in the factory session that registered them.
- Empty assignee clear unassigns — it does not reassign to a random live worker.
- Pre-flight version check recipe reads the git hash from `cas --version`, not the build date.

## Post 2 — Dev

Factory reliability cluster for multi-epic concurrency, close/merge gates, spawn/sync metadata, worktree consume semantics, and Codex liveness.

- Assignment staleness: preferred sync ref from parent epic (then focus pin / base); no global `epic/*` last-branch pick; two-epic unit coverage.
- Zero-commit close: merge-satisfied path when parked `factory_branch_anchor` is ancestor of parent after MERGE REQUIRED merge; genuine zero-commit without anchor still rejects.
- worktree_merge: default preserve for live mid-session merge; `cleanup=true` for end-of-lane consume; `force` remains dirty-only.
- is-wedged: Codex PID prefer harness binary over `cas serve`; `~/.codex/sessions` rollout resolve by cwd; CPU + worktree activity before Starved; longer Codex freshness window.
- spawn pre-assign + shutdown release; sync_all_workers shares `resolve_worker_clone_path` with worker_status (convention fallback + retryable skip).
- Event reminds: registration-time factory session id; fire-time session filter.
- Assignee update: empty/whitespace → clear (None), never session-id normalize to a live worker.
- Supervisor checklist/preflight: hash extract from `cas X (hash[-dirty] date)`.
- Release mechanics: merge `cf81ea6` on `main` via PR #51; rebuild `cargo build --release` + restart `cas serve` for factory paths.
