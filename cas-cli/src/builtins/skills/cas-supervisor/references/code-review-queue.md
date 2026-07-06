# Supervisor-Owned Code Review Queue (cas-b51a / cas-865b)

When the project config has `[code_review] owner = "supervisor"` (the default as of cas-865b), workers skip the full multi-persona review at close and instead transition their tasks to `pending_supervisor_review`. This eliminates the ~14-minute per-close blocking cost on the worker side.

## The queue is a visibility tool — not the trigger

**The full review fires at cherry-pick time** (see workflow.md Phase 3, step 5), not at queue intake. Use this queue page to see what is awaiting cherry-pick, not to decide when to run the review.

```
mcp__cas__task action=list status=pending_supervisor_review
```

This shows you which tasks have been closed by workers and are waiting for you to cherry-pick and review.

## Review workflow

See **workflow.md Phase 3, step 5** for the exact review invocation sequence:
- Capture pre-cherry-pick HEAD: `git rev-parse HEAD@{1}`
- Invoke: `/cas-code-review mode=interactive base_sha=<pre_cp> task_id=<task-id>`
- Address P0 findings before notifying other workers to sync

## After review

1. **If clean, record the approval** — Tell the worker the review passed and, optionally, add `mcp__cas__verification action=add task_id=<id> status=approved summary="..."` for the audit trail.
2. **If changes are required, create the task first** — File an epic-child task with the finding, expected fix, acceptance criteria, and proof command in the task description. Then send a short coordination message that points at the task ID and tells the worker to run `mcp__cas__task action=show id=<id>`.

Do this for both per-task review findings and epic-level review fix rounds. Do not deliver actionable findings only as a coordination message: messages are not durable task state, and a one-shot Codex worker recovering through `task mine` will otherwise see nothing to do.

## Config

Default as of cas-865b is `owner = "supervisor"` — no config entry is needed for new projects. To opt out to the legacy inline worker dispatch, add:
```toml
[code_review]
owner = "worker"
```
