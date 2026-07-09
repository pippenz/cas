# Supervisor-Owned Code Review Queue (cas-b51a / cas-865b)

When the project config has `[code_review] owner = "supervisor"` (the default as of cas-865b), workers skip the full multi-persona review at close and instead transition their tasks to `pending_supervisor_review`. This eliminates the ~14-minute per-close blocking cost on the worker side.

## The queue is a visibility tool — not the full-review trigger

Use this queue page to see what is awaiting cherry-pick. It does not trigger
the full multi-persona review. Phase 3 uses a lightweight per-merge gate; the
single required full `/cas-code-review` run happens in Phase 4 after the epic
is code-complete.

```
cas__task action=list status=pending_supervisor_review
```

This shows you which tasks have been closed by workers and are waiting for you to cherry-pick and inspect.

## Per-merge gate

See **workflow.md Phase 3, step 5** for the lightweight gate:
- Read the direct diff against the task spec and acceptance criteria.
- Check ownership boundaries, obvious defects, missing files/tests, and proof.
- Run targeted mechanical verification only when the diff warrants it.
- Add a `cas__verification action=add` row for the audit trail.

Reserve `/cas-code-review mode=interactive` for Phase 4's assembled epic diff,
unless a single merge is exceptionally risky and you explicitly choose to spend
the full review there.

## After review

1. **If clean, record the approval** — Tell the worker the review passed and, optionally, add `cas__verification action=add task_id=<id> status=approved summary="..."` for the audit trail.
2. **If changes are required, create the task first** — File an epic-child task with the finding, expected fix, acceptance criteria, and proof command in the task description. Then send a short coordination message that points at the task ID and tells the worker to run `cas__task action=show id=<id>`.

Do this for both per-merge gate findings and epic-level review fix rounds. Do not deliver actionable findings only as a coordination message: messages are not durable task state, and a one-shot Codex worker recovering through `task mine` will otherwise see nothing to do.

## Config

Default as of cas-865b is `owner = "supervisor"` — no config entry is needed for new projects. To opt out to the legacy inline worker dispatch, add:
```toml
[code_review]
owner = "worker"
```
