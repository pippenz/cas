# BUG: cas-supervisor skill mandates a full multi-persona review at EVERY cherry-pick — should be once at epic code-complete

**From:** petra-stella-cloud team (supervisor session, 2026-07-07, operator directive from Daniel)
**Severity:** Medium — process/cost defect in supervisor guidance, not code
**Component:** cas-supervisor skill — `references/workflow.md` (Phase 3 step 5) and `references/code-review-queue.md`

## The defect

`workflow.md` Phase 3 ("Merge and Sync"), step 5, instructs the supervisor to run the full
`/cas-code-review mode=interactive` pipeline **after every worker's cherry-pick/merge**, and
`code-review-queue.md` reinforces it ("The full review fires at cherry-pick time"). The same
workflow file ALSO mandates an epic-level integration review in Phase 4 step 3.

Operator policy (Daniel, 2026-07-07): **the full multi-persona review runs ONCE, when the epic is
code-complete — not at every worker check-in.** Per-merge reviews are redundant with the Phase 4
integration review (which sees every diff again, in context) and multiply cost linearly with task
count: the multi-persona Workflow is a ~14-minute, many-agent run; an epic with 9 tasks would pay
it ~10 times to ship one branch.

Concrete instance: today a supervisor followed the skill as written and dispatched the full
Workflow review for a single task's merge (cas-6c1f) on epic cas-83ec; the operator had to
intervene mid-run and the workflow was killed to stop the burn.

## What the per-merge step SHOULD be

A lightweight supervisor gate, not the persona pipeline:
- direct diff read against the task spec (file-ownership boundaries, obvious defects),
- targeted mechanical verification where warranted (e.g. grep vendor CSS for variable existence — caught a real hallucination today),
- `verification action=add` for the audit trail.

Reserve the multi-persona Workflow for **one** Phase 4 integration review of `epic..base` when all
child tasks are closed (plus, at supervisor discretion, an early run for an exceptionally risky
single diff — discretion, not mandate).

## Suggested fix

1. Rewrite `workflow.md` Phase 3 step 5: replace "run /cas-code-review at every cherry-pick" with the lightweight gate above; keep the full review solely in Phase 4.
2. Update `code-review-queue.md` accordingly ("queue is a visibility tool" stays; "full review fires at cherry-pick time" goes).
3. If per-merge full review is meant to be configurable, make it an explicit opt-in (`[code_review] cadence = "per-merge" | "epic"`, default `epic`).

## Related reports (same session)

- BUG-spawn-workers-inherits-supervisor-model-2026-07-07.md (same cost-default theme)
- BUG-finished-notification-while-close-rejected-2026-07-07.md
- BUG-worker-cannot-invoke-task-close-2026-07-07.md
