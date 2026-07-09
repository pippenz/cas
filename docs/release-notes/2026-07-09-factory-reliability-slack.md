# Release — Factory reliability & readability (2026-07-09, main 729e917)

Channel: #cas-internal (C0B44GUKDK2). Two top-level posts.

---

## Post 1 — User

🚀 **Live on production** — the factory got much harder to lose work in, and much quieter.

**Was → Now:**
- **Finishing work.** Was: a task could be reported "done and verified" while its code was never actually saved or merged — so a routine cleanup could quietly delete it. Now: nothing is called done until the work is genuinely committed and integrated; unfinished work can't slip through as "verified."
- **Stopping a stuck worker.** Was: asking to stop a stuck worker could hit the wrong thing and either do nothing or yank a task away from a perfectly healthy worker. Now: it finds the right worker, confirms it actually stopped before touching anything, and leaves healthy ones alone.
- **Less nagging.** Was: constant "this worker is stalled!" alerts about workers that were simply thinking or running tests, plus prompts to start work you had deliberately paused. Now: it recognizes real progress, waits longer for deeper reasoning, and respects holds.
- **Right horsepower per job.** Was: teams of workers tended to spin up at one default power level. Now: matching the right model and effort to each task is the obvious default.
- **Fewer papercuts.** Dependency messages now read in plain English ("A won't start until B is done" instead of a confusing arrow), and epic branches start from the latest code instead of a stale local copy.

---

## Post 2 — Dev

🚀 **Live on production** — a reliability pass on the close gate, the kill path, and the stall detector: latent correctness holes closed, with an adversarial review that caught five more before they shipped.

**Was → Now:**
- **Close / merge gate.** Was: merge-state resolution only checked the config-gated worktree store, so isolated worker worktrees fell through and tasks could close on uncommitted/unpushed code; branch-HEAD anchoring stranded serial tasks; tasks with no epic targeted the trunk. Now: real isolated-worktree resolver, task-commit anchor with a proper clear-on-close/reopen lifecycle, origin/<parent> reachability fallback, a backstop for standalone tasks, plus option-injection and path-traversal hardening.
- **Kill / liveness.** Was: the worker process was resolved by an env var inherited by every child process, then signalled with `killpg` without a group-leader conversion — so a kill could silently no-op on ESRCH or reset a live worker's task. Now: command-line-first resolution, process-group-leader signalling, ps-verified death before any lease reset, and a two-signal "dead" classification with an explicit "unverified" state.
- **Stall / idle detector.** Was: it ignored transcript progress, used a flat timeout, and surfaced dependency-blocked tasks as assignable. Now: transcript activity counts as progress, thresholds scale with configured effort, ready-counts respect the dependency graph, and a first-class worker-hold primitive exists.
- **Also:** worktree-merge decoupled from the experimental flag and now targets the correct epic branch; fetch-before-branch with a local-ahead guard; plain-words dependency direction; and an inline model/effort tiering rubric in the always-loaded supervisor guidance.
