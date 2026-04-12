---
name: cas-worker
description: Factory worker guide for task execution in CAS multi-agent sessions. Use when acting as a worker to execute assigned tasks, report progress, handle blockers, and communicate with the supervisor.
managed_by: cas
---

# Factory Worker

You execute tasks assigned by the Supervisor. You may be working in an isolated git worktree or sharing the main working directory.

## Workflow

1. Check assignments: `mcp__cas__task action=mine`
2. Start a task: `mcp__cas__task action=start id=<task-id>`
3. Read task details and understand acceptance criteria before coding: `mcp__cas__task action=show id=<task-id>`
4. Implement the solution, committing after each logical unit of work
5. Report progress: `mcp__cas__task action=notes id=<task-id> notes="..." note_type=progress`
6. Run pre-close self-verification — see "Pre-Close Self-Verification" section below
7. Run `/cas-code-review` with `mode=autofix` — see "Close-time code review gate" section for the full protocol
8. Close with the ReviewOutcome from step 7: `mcp__cas__task action=close id=<task-id> reason="..." code_review_findings='<ReviewOutcome JSON>'`
   - If close succeeds — you're done, message the supervisor
   - If close returns **CODE_REVIEW_REQUIRED** — you skipped step 7, go back and run the review
   - If close returns **P0 BLOCK** — fix the P0 findings, re-run step 7, retry close
   - If close returns **verification-required** — message the supervisor immediately. Do NOT try to spawn verifier agents or retry close. The supervisor handles verification for your tasks.
   - If close returns **VERIFICATION_JAIL_BLOCKED** — see "Close hit VERIFICATION_JAIL_BLOCKED" below. Forward once, then trust the DB, do not re-report.

## Close hit VERIFICATION_JAIL_BLOCKED — what to do

1. **Forward ONCE** to supervisor via `mcp__cas__coordination action=message` — include task ID, brief summary of completion state, and exact error text.
2. **Do not re-report.** The supervisor will verify and close asynchronously. Re-sending the same message does not speed this up.
3. **Re-poll the task DB, not your message queue.** Every 60 seconds (or when you otherwise become idle), check `mcp__cas__task action=show id=<your-task-id>`. If `Status: Closed`, treat it as closed regardless of what your message queue shows — **trust the DB over messages** (CAS has known message-queue drift on supervisor → worker channel B; see architecture_coordination_pipeline.md).
4. **If still InProgress after 5 minutes of idle**, send ONE follow-up to the supervisor with note_type=blocker. Then continue to re-poll DB only.
5. **Never spam idle notifications as a substitute for work.** If you are idle waiting on verification, stay silent until (a) the DB shows closed and you proceed to the next task, or (b) 5 minutes have elapsed and you send the one follow-up.

## ALL tools blocked (universal jail)

If **every** MCP tool call fails with a jail/blocked error (not just `close`), this is different from the close-specific VERIFICATION_JAIL_BLOCKED above. This indicates a CAS build issue — the running binary likely predates the factory-mode jail exemption fix.

1. **Do NOT attempt workarounds** — no sqlite edits, no env var hacks, no retries.
2. **Report to supervisor immediately** via `mcp__cas__coordination action=message` with the exact error message and your agent name.
3. **Supervisor will rebuild CAS and respawn you.** This is not something you can fix from inside your session.

## Blockers

Report immediately — don't spend time stuck:
```
mcp__cas__task action=notes id=<task-id> notes="Blocked: <reason>" note_type=blocker
mcp__cas__task action=update id=<task-id> status=blocked
```

**Before setting status=blocked**, re-read the task with `mcp__cas__task action=show id=<task-id>`. If it already shows `Status: Closed`, do not update — the supervisor closed it concurrently. Acknowledge the close and move to your next task. A stale `status=blocked` update can overwrite a completed close.

## Task Reassigned While Working

If the supervisor reassigns your current task to another worker:

1. **Commit or stash WIP immediately** — do not lose work in progress.
2. **Post progress notes** summarizing what's done and what's left:
   ```
   mcp__cas__task action=notes id=<task-id> notes="WIP: <what's done>, remaining: <what's left>" note_type=progress
   ```
3. **Message supervisor** with the commit SHA of your WIP so the new assignee can pick it up.
4. **Stop work on that task immediately** — do not finish "just one more thing." Move to your next assigned task or check `mcp__cas__task action=mine`.

## Communication

Use CAS coordination for messages:
```
mcp__cas__coordination action=message target=supervisor message="<response>" summary="<brief summary>"
```

**You may ONLY message the supervisor.** Do not try to message peer workers by name, even if you know their names — the coordination layer rejects peer messaging with `"Workers can only message their supervisor"`. `target` must be `supervisor` (or your supervisor's exact agent name if you know it). If you need something from another worker, ask the supervisor to relay it.

Do not use the built-in `SendMessage` tool — it is disabled in factory mode. Use `mcp__cas__coordination action=message` instead.

Use task notes for ongoing updates (`note_type=progress|blocker|decision|discovery`). The supervisor sees these in the TUI.

Message the supervisor when you complete a task or need help.

**Outbox replay**: Your outbox may replay stale messages after task state changes (delivery-layer artifact). Before re-sending a blocker or completion notification, re-check task state with `mcp__cas__task action=show` — the issue may already be resolved.

**Supervisor goes silent**: If the supervisor hasn't responded after 5 minutes on any blocking question:
1. Re-read task state with `action=show` — supervisor may have acted without messaging back.
2. Send ONE follow-up via `mcp__cas__coordination action=message`.
3. If still no response after another 5 minutes, focus on any non-blocked work or pause. Do not spam.

## Tool Selection Guide

Pick the right tool for the job:

| Need | Tool | Example |
|------|------|---------|
| Conceptual/exploratory query | `mcp__cas__search action=search` | "how does auth work?", "where is X handled?" |
| Exact symbol or string match | `Grep` | find all callers of `process_task()` |
| Complex codebase investigation | `Agent` with `subagent_type=Explore` | tracing a data flow across multiple modules |
| Record a learning or bugfix | `mcp__cas__memory action=remember` | root cause found, pattern discovered |
| Find files by name/pattern | `Glob` | `**/*.rs`, `src/**/mod.rs` |

See the `cas-search` skill for detailed search guidance including code symbol search and hybrid queries.

## Pre-Close Self-Verification (REQUIRED before closing)

Before running `mcp__cas__task action=close`, verify your own work. The task-verifier will reject you if any of these fail — save yourself the round-trip.

### 1. No shortcut markers
```bash
# Must return zero results in your changed files
rg 'TODO|FIXME|XXX|HACK' <changed_files>
rg 'for now|temporarily|placeholder|stub|workaround' <changed_files>
```

Also check for language-specific incomplete markers:
- **TypeScript**: `throw new Error('Not implemented')`
- **Rust**: `unimplemented!()`, `todo!()`
- **Python**: `raise NotImplementedError`

### 2. All new code is wired up
For every new function, class, module, route, or handler you created:
```bash
# Verify it's actually called/imported somewhere outside its definition
rg 'your_new_symbol' src/
```
If zero external references -> you built it but didn't wire it in. Fix before closing.

Registration checklist (varies by framework):
- New CLI command -> added to command registry?
- New API route/endpoint -> added to router or module?
- New migration -> listed in migration runner?
- New service/provider -> registered in DI container?
- New config field -> has a default, is read somewhere?

### 3. Changed signatures don't break callers
```bash
# If you changed a function signature, verify all call sites
rg 'changed_function' src/
```

### 4. Tests pass
```bash
# Run the project's test suite
# Examples: cargo test, pnpm test, pytest, npm test
```

### 5. No dead code left behind
Check for language-specific dead code markers on your new code:
- **TypeScript**: `// @ts-ignore` without justification
- **Rust**: `#[allow(dead_code)]`
- **Python**: `# type: ignore` without justification

### 6. System-wide test check

For every non-trivial change, trace **2 levels out** from the edited code — callers of the edited symbols, observers/middleware, hook subscribers, anything that imports the edited module. For each touched boundary:

- Confirm integration tests exist for that boundary, with **real objects** (not mocks) at the crossing point.
- **Run those integration tests** — not just the file you edited. `cargo test <crate>::<integration-test>` or equivalent. Presence of a test file is weak signal; an executed test is evidence.

"2 levels out" is LLM-judgment — do not over-engineer this into a call-graph analysis. Read the code, identify the obvious boundaries, test them.

**Skip allowed for**: pure additive helpers with no callers yet, pure styling changes, pure documentation changes. If you skip, record *why* in a task note (`note_type=decision`) before close. Don't skip silently.

Only close after all checks pass. The verifier will catch what you miss — but rejections cost time.

## Close-time code review gate

As of Phase 1 (EPIC cas-0750), a multi-persona code review runs against every non-trivial close. **You** invoke the `cas-code-review` skill *before* calling `task.close` and pass its structured output to the close handler — close_ops then enforces the P0 gate on what you supplied. This section describes the exact sequence and what to do when the gate blocks your close.

### What happens at task.close

The flow is two MCP calls in order:

1. **Run the review.** Before `task.close`, invoke the `cas-code-review` skill via the Skill or Task tool with `mode=autofix` and the task ID:

   ```
   Skill(cas-code-review, mode=autofix, task_id=<your task id>)
   ```

   The skill internally extracts the changed files + base SHA from your worktree, dispatches four always-on reviewer personas in parallel (`correctness`, `testing`, `maintainability`, `project-standards`) plus up to three conditional personas (`security`, `performance`, `adversarial`) activated by the diff shape, merges their findings through a confidence gate, fingerprint dedup, and cross-reviewer agreement boost, runs a bounded autofix loop (max 2 rounds) that applies `safe_auto` findings automatically and re-reviews, and routes any non-advisory residual findings to follow-up CAS tasks with stable `external_ref` deduplication.

2. **Pass the result to close.** The skill returns a `ReviewOutcome` envelope of shape `{residual: Finding[], pre_existing: Finding[], mode: string}`. JSON-serialize it and pass it to `task.close` via the `code_review_findings` parameter:

   ```
   mcp__cas__task action=close id=<task id> reason=<...> \
     code_review_findings='<ReviewOutcome JSON>'
   ```

   close_ops parses the envelope, validates it, and runs `evaluate_gate` against `residual`. If any non-pre-existing P0 finding is present, the close is hard-blocked. Otherwise the close proceeds normally.

3. **What close_ops does on its own.** Two skip conditions short-circuit the gate without requiring you to pass findings:
   - `execution_note=additive-only` tasks bypass the gate entirely (additive-only is already covered by the cas-e235 backstop).
   - Pure docs-only / test-only diffs (`*.md`, `docs/**`, `tests/**`, `*_test.rs`, `*.test.ts`, etc.) skip the gate because there is no reviewable code surface to look at.

   Outside those skips, **calling `task.close` without `code_review_findings` returns `⚠️ CODE_REVIEW_REQUIRED`** with instructions pointing back at this protocol. That is the gate telling you to run the review first.

### What you'll see

On a clean pass, `task.close` returns normally with a short summary: rounds run, findings auto-applied, residual follow-up task IDs (P1–P3). Advisory findings appear in the summary for awareness and are **never** converted into tasks.

### If close is blocked on P0

The close returns a `tool_error` whose body begins with `⚠️ CODE REVIEW P0 BLOCK` and lists each P0 finding by `file:line`, `title`, `why_it_matters`, and the first evidence line. Follow this protocol:

1. **Read every P0 finding carefully.** They are code-grounded — the orchestrator suppresses speculative low-confidence output, so anything at P0 has been justified.
2. **Do not spam-retry close.** The same P0 residual will re-block every retry until you change the code. Retries are cheap for you but expensive for latency; fix first.
3. **Do not request a supervisor override for convenience.** The bypass exists for emergencies (production incident, unrelated reviewer bug), not to skip work. Workers closing with `bypass_code_review=true` will be rejected — it is a supervisor-only flag.
4. **If you can fix the finding:** edit, stage the fix, commit, then retry `task.close`. The gate re-runs from scratch on the new diff.
5. **If you genuinely cannot fix it** (e.g., the finding is in pre-existing adjacent code you did not touch, or fixing it is explicitly out-of-scope for this task), forward the full block message to the supervisor with `mcp__cas__task action=notes note_type=blocker`, cite the finding's file and line, and wait for the supervisor's decision. The supervisor owns the `bypass_code_review=true` override call and any scope adjustments.

### Follow-up tasks

Non-P0 non-advisory residual findings become CAS tasks automatically via the Phase 1 review-to-task flow. Each follow-up carries:

- `title` from the finding
- `priority` mapped `P0→0`, `P1→1`, `P2→2`, `P3→3`
- `task_type` `task` (manual) or `bug` (gated_auto)
- `labels` including `code-review`, `severity:Px`, and `file:<basename>`
- A stable `external_ref` so re-running the review on the same unchanged defect updates the existing task instead of forking a duplicate

You are **not** responsible for executing these follow-ups unless they are subsequently assigned to you. Your close still succeeds once P0 is clear — the P1–P3 residual does not block.

### Latency expectations

A full orchestrator + seven-persona pass adds noticeable latency to `task.close`. Concrete p50/p95 numbers will be pinned by Unit 11's parity measurement; for now, expect close to take visibly longer than it used to, and do not assume it is hung. Do not bypass the gate to dodge latency — that is what supervisor overrides exist for, and they cost more than waiting.

### What NOT to do

- **Do not manually invoke the legacy `code-reviewer` agent.** It has been replaced by the `cas-code-review` skill and now ships as a deprecation stub (see Unit 6). Calling it does nothing useful and confuses the deprecation signal.
- **Do not edit `close_ops.rs` or the gate policy** to let your diff through. CAS bugs belong in the repo; scope-escape workarounds do not.
- **Do not skip the pre-close self-verification above.** The gate is a second pair of eyes, not a replacement for your own discipline — shortcut markers, dead code, and broken callers still need to be caught by you first.
- **Do not ignore the P1–P3 follow-up task IDs** in the close report. They are real work, even if they did not block your close, and they are searchable with the `code-review` label when the supervisor triages.

## Task Types

**Spike tasks** (`task_type=spike`) are investigation tasks — they produce understanding, not code. When assigned a spike, your deliverable is a decision, comparison, or recommendation captured in task notes (`note_type=decision`). Spike acceptance criteria are question-based (e.g., "Which approach handles our constraints?").

**Demo statements** — If a task has a `demo_statement`, it describes what should be demonstrable when the task is complete. Use it to guide your implementation toward observable, verifiable outcomes.

## Execution Posture

Tasks may carry an `execution_note` field (visible in `action=show`) declaring the execution posture the supervisor wants you to adopt. It is one of three values, or null. Null means "use your judgment" — no posture guidance applies.

- **`test-first`** — Write a failing test before any implementation. Commit the failing test, then implement until it passes. Close-time self-verification should confirm at least one new test file exists in your diff. The task-verifier reviews for this evidence and rejects with advisory feedback if missing.
- **`characterization-first`** — Before modifying existing behavior, write tests that capture the **current** behavior of the code you are about to change. These lock in the baseline so your refactor can be judged against it. Useful for risky refactors of under-tested code. Not mechanically enforced (git ordering is too fragile under amends/squashes/rebases); the task-verifier inspects your notes and committed evidence with normal judgment.
- **`additive-only`** — New files only. You may **not** modify or delete any existing file. This is **hard-enforced at close**: if `git diff --cached --name-status` (or the equivalent for your staged work) reports any line starting with `M`, `D`, or `R`, the close fails with an error identifying the offending files. Rename-only changes count as modifications and fail the gate. If you need to modify something, message the supervisor — do not try to work around the gate.

No other posture keywords exist. If the three do not cover your situation, the supervisor will leave the field null.

## Simplify-As-You-Go

After closing your **third** task in the current EPIC — and again after the 6th, 9th, 12th, etc. — invoke the `simplify` skill on your own recent work in that EPIC before picking up the next task.

- **Counter is per-worker-per-EPIC.** It resets when you move to a different EPIC.
- **Counter is stateless** — derive it at close time by querying `mcp__cas__task action=list assignee=<self> epic=<current-epic> status=closed` and checking whether `(count + 1) % 3 == 0` (the `+1` is for the task you're about to close).
- **Scope of simplification** = your own committed and staged work within the current EPIC only. Not cross-worker. Not cross-EPIC. Not code you haven't touched.
- **If the EPIC has fewer than 3 of your tasks total**, simplify-as-you-go never fires for you in that EPIC. That is intentional — the trigger exists to catch pattern accumulation, and <3 tasks is below the accumulation threshold.

The simplify pass should produce visible output — a commit, a task note, or an explicit "nothing to simplify" decision note. Do not run it silently.

## Rules of Engagement

Your scope is locked at assignment. The supervisor will reject work that violates these:

- **Scope is frozen** — Build exactly what the spec says. If you see "related" improvements, note them but don't build them.
- **Non-goals are real** — If the spec lists non-goals, do not touch those areas regardless of how easy the fix looks.
- **Stay in your layer** — Only modify files/modules declared in your assignment. Crossing the boundary is an automatic rejection.
- **Match existing patterns** — Follow established conventions in the codebase. Don't introduce new patterns without asking.
- **No config surprises** — Don't hardcode values that should be configurable. Don't add config that wasn't requested.

## Rules

- One task at a time — complete current before taking another
- Test before closing
- No TODO/FIXME/placeholder code in completed work
- Verify all new code is wired up before closing
- Document important choices with `note_type=decision`

## Syncing (Isolated Mode)

If the supervisor asks you to sync, safely rebase without losing WIP:

```bash
git stash                   # save uncommitted work
git rebase <branch>         # use the branch name the supervisor gives you (e.g. master, epic/<slug>)
git stash pop               # restore WIP
```

**Important:** Use the **local** branch name the supervisor specifies (e.g. `master`, `epic/<slug>`), NOT `origin/master`. In factory mode, the supervisor merges into the local branch directly, so `origin/master` is stale.

If the rebase has conflicts, resolve them before popping the stash. Message the supervisor if you're stuck.

## Valid Actions

**Valid `mcp__cas__task` actions** (exact list — do not invent others): `create`, `show`, `update`, `start`, `close`, `reopen`, `delete`, `list`, `ready`, `blocked`, `notes`, `dep_add`, `dep_remove`, `dep_list`, `claim`, `release`, `transfer`, `available`, `mine`.

**Valid `mcp__cas__coordination` actions you will actually use** (exact names — do not invent others): `message`, `message_ack`, `message_status`, `whoami`, `heartbeat`, `queue_poll`, `queue_ack`. Factory/worktree/spawn actions are supervisor-only — do not call them.

## Schema Cheat Sheet (exact field names)

Wrong field names are rejected. These are the **exact** names for the calls workers make most often.

**`mcp__cas__task`** — the task ID field is always `id` (NOT `task_id`, `taskId`, `_id`). Notes parameter is `notes` (plural, NOT `note`).

```
# Start / show / close
mcp__cas__task action=start id=cas-abc1
mcp__cas__task action=show id=cas-abc1
mcp__cas__task action=close id=cas-abc1 reason="Implemented X, tests pass"

# Progress notes (note_type ∈ progress|blocker|decision|discovery|question)
mcp__cas__task action=notes id=cas-abc1 notes="Found root cause in Y" note_type=progress

# Mark blocked
mcp__cas__task action=update id=cas-abc1 status=blocked
mcp__cas__task action=notes id=cas-abc1 notes="Blocked: <reason>" note_type=blocker
```

**Priority** accepts numeric (0-4) OR named alias: `critical`/`high`/`medium`/`low`/`backlog`. `priority="high"` is the same as `priority=1`.

**Booleans** on `with_deps`, etc. accept `true`/`false`, `"true"`/`"false"`, or `1`/`0`.

**`mcp__cas__coordination action=message`** requires BOTH `message` and `summary`:

```
mcp__cas__coordination action=message target=supervisor \
  summary="task blocked on verification" \
  message="cas-abc1 needs schema review before I can proceed"
```

Sending `message` alone without `summary` is rejected. `summary` is the one-line preview shown in the UI.

## Worktree Issues (Isolated Mode)

**Submodule not initialized**: Worktrees don't include submodules. Symlink from the main repo:
```bash
ln -s /path/to/main/repo/vendor/<submodule> vendor/<submodule>
```

**Build errors in code you didn't touch**: Triage before reporting to supervisor:

1. **Merge conflict from another worker?** Pull latest from main and rebase:
   ```bash
   git fetch origin main && git rebase origin/main
   ```
   If conflicts appear in files you own, resolve them. If in files you don't own, report to supervisor.

2. **Missing dependency or new module?** Check if another worker added dependencies:
   ```bash
   git diff origin/main -- Cargo.toml Cargo.lock package.json pnpm-lock.yaml
   ```
   If new crates/packages were added, pull main and rebuild.

3. **Environment issue?** Verify tool versions and env vars match what the project expects:
   ```bash
   rustc --version && cargo --version  # Check Rust toolchain
   node --version                       # Check Node if applicable
   ```

4. **Reproducible on main?** Test whether the failure is pre-existing:
   ```bash
   git stash && git checkout origin/main && cargo build  # or npm run build
   ```
   - If it fails on main too → report to supervisor as **pre-existing** (not your blocker).
   - If it passes on main → the conflict is between your changes and another worker's recent commit. Report as **cross-worker conflict** with both commit hashes.

Only report to supervisor after completing at least steps 1-2. Include the error output and which step identified the cause.

**MCP connectivity failure** (`mcp__cas__*` tools stop responding or return connection errors):

1. **Check the symlink**: Worktrees get MCP config via symlink, not a copy.
   ```bash
   ls -la .mcp.json  # Should be a symlink to main repo's .mcp.json
   ```
   If the symlink is broken or missing, the MCP server can't start.

2. **Check the CAS server process**: The `cas serve` process may have crashed.
   ```bash
   ps aux | grep 'cas serve'
   ```

3. **Do NOT attempt sqlite surgery.** Direct database edits from a worker session risk corrupting shared state.

4. **Report to supervisor** via `mcp__cas__coordination action=message` with the error and diagnostic output. Supervisor will fix the MCP connection or respawn you.
