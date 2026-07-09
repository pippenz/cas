# BUG: `task dep_add` direction is ambiguous — confirmation output reads opposite to actual effect

- **Date:** 2026-07-08
- **Reporter:** supervisor (quiet-tiger-24), gabber-studio factory session `gabber-studio-sharp-hawk-84`
- **Area:** `mcp__cas__task action=dep_add` / dependency-edge semantics + confirmation string
- **Severity:** MEDIUM (silent wrong-direction dependency chains; deadlocks or mis-ordered execution; caught here only because two workers manually flagged it)

## Summary
`dep_add id=A to_id=B dep_type=blocks` creates the edge **"A is blocked by B"**
(B must complete before A can start). But the confirmation string it prints —
`Added Blocks dependency: A -> B` — reads naturally as **"A blocks B"** (A → B,
"Blocks"), i.e. the *opposite* relationship. The arrow direction plus the word
"Blocks" together imply A is the blocker, when in fact A is the blocked one.

## Observed (this session)
Intended chain order: `cas-485a → cas-1807 → cas-fefe → cas-f4fd` (485a first).
Reasoning "485a blocks 1807 = 485a first", I issued:

```
dep_add id=cas-485a to_id=cas-1807 dep_type=blocks
→ "Added Blocks dependency: cas-485a -> cas-1807"
```

I read that as "485a blocks 1807" (485a runs first). **Actual effect:** cas-485a
became *blocked by* cas-1807 — the worker assigned cas-485a reported it as
`BlockedBy cas-1807` and went idle. Every edge in the chain (and the
`eaeb ← 7500/0060` edges) was created backwards the same way. It took two
workers (`wild-lion-17`, `sturdy-finch-54`) independently flagging "my task is
blocked by X but that's reversed / circular" to catch it; I then removed and
re-added all five edges in the correct direction.

Notably: my *first* instinct (`dep_add id=cas-1807 to_id=cas-485a` — "1807
blocks 485a") was actually correct, but the ambiguous output convinced me it was
backwards and I "fixed" it into being wrong. The output actively misleads.

## Impact
- Dependency chains are easy to file 180° wrong with no error — the graph is
  valid, just reversed. Result is either a deadlock (task A waits on downstream
  task B that waits on A) or silently mis-ordered execution.
- The only signal was workers noticing their `ready`/`blocked` state contradicted
  the intended plan. Without attentive workers this ships as a stalled factory.

## Expected
1. Make the confirmation string state the relationship in **plain, unambiguous
   words**, not an arrow. e.g.:
   `dep_add id=A to_id=B dep_type=blocks` →
   `"cas-A will not start until cas-B is done (cas-A blocked_by cas-B)."`
2. Consider renaming/aliasing for clarity: a `blocked_by` verb (`dep_add id=A
   blocked_by=B`) whose direction is self-evident, and/or a `blocks` verb whose
   `id` is the blocker. The current `id`+`to_id`+`dep_type=blocks` triple gives
   no directional cue at the call site either.
3. `dep_list` / task `show` should render dependencies as "blocked by: […]" and
   "blocks: […]" sections rather than raw `A -> B` arrows.

## Repro
```
task create ... → cas-A ; task create ... → cas-B
task dep_add id=cas-A to_id=cas-B dep_type=blocks
# prints: "Added Blocks dependency: cas-A -> cas-B"
task ready   # cas-A is NOT ready (blocked); cas-B IS ready — i.e. A blocked_by B,
             # the opposite of what the "A -> B blocks" output implies.
```

## Resolved (cas-ac2e)

Implemented Expected #1 and #3 directly (output-only, edge direction/semantics
unchanged):

- `dep_add`/`dep_remove` confirmations now go through a new
  `describe_dependency()` helper (`cas-cli/src/mcp/tools/core/task/dependencies.rs`)
  that states every dependency type in plain words with no arrow — e.g.
  `dep_type=blocks` now prints `"cas-A will not start until cas-B is done
  (cas-A blocked_by cas-B)."`, matching this doc's suggested wording exactly.
- `dep_list` (`cas-cli/src/mcp/tools/core/task_extensions.rs`) now renders
  "Blocked by:"/"Blocks:" (plus "Parent:"/"Children:"/"Related:"/etc.) sections
  instead of the raw `TypeDebug: from -> to` dump.
- `task show` already avoided bare arrows; its `BlockedBy:` label was renamed
  to `Blocked by:` for wording consistency with the new `dep_list` output.

Expected #2 (a self-evident `blocked_by=` verb alias at the call site) was
intentionally deferred — it would broaden `DependencyRequest`'s schema/dispatch
surface, a larger and separable change from the misleading-output fix. Noted
as a candidate follow-up rather than folded into this task.

Regression tests: `cas-cli/tests/mcp_tools_test/task_tools/dependencies.rs`
reproduces this doc's exact repro end-to-end through the real MCP handlers
(including a sanity check that the edge's actual blocked/ready effect matches
the new wording), plus unit tests for the new rendering helpers.
