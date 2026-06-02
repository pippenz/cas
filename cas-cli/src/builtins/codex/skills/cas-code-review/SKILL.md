---
name: cas-code-review
description: Multi-persona code review orchestrator, invoked by the supervisor at cherry-pick into the EPIC branch (per-task) and at EPIC→base merge (integration sweep) under the default `[code_review] owner = "supervisor"` (v2.13.0+). Factory workers MUST NOT invoke this skill pre-close under the default ownership model — the supervisor owns review timing. Dispatches 4 always-on reviewer personas (correctness, testing, maintainability, project-standards) plus fallow (dispatched only on JS/TS diffs) and any activated conditional personas (security, performance, adversarial) in parallel against the current diff, merges their structured findings through a deterministic pipeline, and routes results according to the invocation mode. Use `mode=interactive` for the standard supervisor-driven path, `mode=report-only` for read-only scans, `mode=headless` for skill-to-skill calls. The legacy `mode=autofix` path that fires inline at `task.close` is opt-in only and applies when a project sets `[code_review] owner = "worker"` in `.cas/config.toml`.
managed_by: cas
---

# cas-code-review — Multi-persona code review orchestrator

This skill is the orchestrator for CAS's Phase 1 multi-persona code review pipeline. It does **not** perform the review itself — each reviewer persona is a separate Sonnet sub-agent with its own prompt under `references/personas/`. Your job in this skill is to:

1. Figure out **what** is being reviewed (the diff + the author's intent).
2. Decide **who** reviews it (the 4 always-on personas, plus fallow when the repo is JS/TS, plus any activated conditional ones).
3. Run them in **parallel**, collect their structured output envelopes.
4. Hand the merged findings to the merge pipeline (Unit 5) and then route them according to the invocation mode.

Everything in this document is the orchestrator's responsibility. The personas themselves are authoritative on *what counts as a finding in their lane* — you do not second-guess their judgment here, you just marshal their inputs and outputs.

**Model tier:** The orchestrator and merge logic run on Opus. All 8 reviewer personas run on Sonnet. The fixer sub-agent (Unit 7) also runs on Opus. Do not inherit model choice from the caller — these are fixed per R13.

## Purpose

The primary trigger for this skill is **manual invocation by the supervisor** during cherry-pick to the epic branch (per-task) and at EPIC→base merge (integration sweep). Use `mode=interactive` for both. This is the path most invocations will take under the default `[code_review] owner = "supervisor"` configuration.

The `autofix` mode still exists as the legacy worker-owned path for projects that opt back in via `[code_review] owner = "worker"` in `.cas/config.toml`. In that mode the review runs *before* worker close completes and gates it (P0 findings hard-block).

Three other modes exist and are described in the Mode reference table:

- `autofix` — legacy, worker-owned, opt-in only; gates `task.close` inline.
- `report-only` — read-only scan, safe to run in parallel (never writes, never mutates tasks).
- `headless` — called from another skill, returns the merged envelope as structured text.

Do **not** invoke this skill manually as a substitute for `mcp__cas__verification` — that is a separate system, still owned by the task-verifier path. cas-code-review is a pre-close *quality gate*, not a verification record.

## Inputs

| Name | Required | Description |
|---|---|---|
| `base_sha` | optional | Commit to diff against. If absent, fall through to the Unit 3 helper at `crates/cas-store/src/code_review/base_sha.rs` (`code_review::base_sha::resolve`). That helper tries caller override → `GITHUB_BASE_REF` → `origin/HEAD` → `gh` default branch → common branches (`main`/`master`/`develop`/`trunk`) → `HEAD~1` in order and returns a full commit SHA. Never hand-roll base resolution here. |
| `changed_files` | computed | The set of files changed between `base_sha` and `HEAD`, computed via `git diff --name-only <base_sha>...HEAD`. You pass the full diff text to each persona, not just the file list. |
| `task_id` | optional | The CAS task ID being closed. Required in `autofix` mode (used for review-to-task linking, supervisor override path, and the task-note trail). Optional in other modes. |
| `mode` | default `autofix` | One of `autofix`, `interactive`, `report-only`, `headless`. See the Mode reference table below. |

If you receive none of these (bare manual invocation), default `mode=interactive`, resolve `base_sha` via the Unit 3 helper, and proceed.

## Step 0: Tiny-diff bypass

Before any other work, check whether the diff is large enough or non-trivial enough to warrant the full multi-persona pipeline. The Rust-side `cas_task_close` gate already skips the multi-persona reviewer on docs-only / test-only / empty diffs via `has_reviewable_changes` (see `cas-cli/src/mcp/tools/core/task/lifecycle/close_ops.rs`). The orchestrator skill MUST mirror that check at the prompt level so the cost is paid neither in the skill nor in the gate. Specifically:

1. Run `git diff --name-only <base_sha>..HEAD`. If every changed file matches a docs path (`*.md`, anything under `docs/`) OR a test path (`tests/`, `test/`, `*_test.rs`, `*.test.ts`, `*.spec.ts`, `*.test.tsx`, `*.spec.tsx`) — return a clean Allow envelope WITHOUT dispatching any personas. Output: `{"residual": [], "pre_existing": [], "mode": "<mode>", "skipped_reason": "diff is docs-only / test-only — no reviewable code paths"}`.
2. Otherwise, run `git diff --shortstat <base_sha>..HEAD`. If the result shows fewer than **5 lines changed total** AND the changed files are all already-existing files (no new files), return a clean Allow envelope as above with `"skipped_reason": "diff is trivial (<5 lines, no new files) — full review pipeline not warranted"`. (5-line threshold: anything smaller is typically a typo fix, a const tweak, or a one-line guard. Bigger jobs deserve the full pipeline.)
3. Otherwise, proceed to Step 1 as written.

This bypass exists because the orchestrator + 4–8 personas + full diff feed costs ~100K input tokens; running it on a 2-line typo fix is pure waste. The mode-specific gating (autofix vs. interactive vs. report-only) still works — the bypass returns the same envelope shape, so callers cannot tell the difference structurally.

## Step 1: Intent extraction

Before dispatching any reviewer, write a **2–3 line intent summary** describing *what the author was trying to accomplish*. This becomes an input to every persona so their calibration matches the author's stated goal — a "refactor only" change is judged differently from a "new security-sensitive endpoint".

Sources to mine, in order of priority:

1. The associated CAS task (`mcp__cas__task action=show id=<task_id>`) — its title, description, acceptance criteria, and notes are the strongest signal.
2. The commit messages on the diff (`git log --format=%B <base_sha>..HEAD`).
3. Any linked PR description if one is discoverable without shelling out to `gh` (Phase 1 does not make `gh` a hard dependency).
4. The actual diff as a last resort — only if the above are silent.

The intent summary is your own synthesis, not a quote. It must capture:

- **Goal:** one line — what behavior should exist after this change that did not exist before.
- **Scope marker:** one line — "refactor / no behavior change", "new feature", "bug fix", "dependency bump", etc.
- **Non-goals (optional):** one line — anything the author explicitly said they were *not* doing.

Keep it tight. Personas use this to calibrate severity, not to learn the problem domain from scratch.

## Step 2: Conditional persona selection (LLM-judged, not path pattern matching)

The **4 always-on personas always run**: `correctness`, `testing`, `maintainability`, `project-standards`. No conditions, no exceptions.

The `fallow` persona is a thin wrapper around the deterministic `fallow audit` CLI. **Skip dispatching `fallow` entirely** when the orchestrator detects the repo is non-JS/TS (no `package.json` at the repo root AND no `*.ts`/`*.tsx`/`*.js`/`*.jsx` in the diff). Record this skip explicitly in the output envelope as `"skipped": ["fallow"], "skip_reason": {"fallow": "non-JS/TS repo: no package.json and no JS/TS files in diff"}` so the audit trail honestly reflects that fallow was considered and excluded by rule, not silently dropped. When the repo is JS/TS, dispatch fallow and let its internal `fallow audit` CLI return whatever it returns (including a clean no-finding envelope on a small diff).

The **3 conditional personas** — `security`, `performance`, `adversarial` — activate based on the R2 heuristics below. **Critically: you activate them by reading the diff and judging whether the heuristic applies. This is an LLM judgment, LLM-judged, not path pattern matching.** Do not grep for `/auth/` in paths and call it security activation. Do not count lines with `regex`. Read the diff, understand what it does, decide whether the heuristic fires.

Why the stridency: past iterations of automated review have drifted into "activate security if file name contains login" — which is both false-positive noisy (renaming a file does not make it security-relevant) and false-negative dangerous (a SQL-injection fix in a file called `util.rs` is security-relevant regardless of its path). The whole point of running an LLM orchestrator here is to apply *judgment*. Use it.

**Activation heuristics (from brainstorm R2):**

- **`security`** — activate when the diff touches authentication boundaries, user input handling, or permission surfaces. Concretely: session/token issuance, signature checks, role/permission gates, anywhere external input is parsed or deserialized, anywhere a privilege decision is made. Changes that merely *pass through* auth-adjacent code without touching its logic do not require security review.
- **`performance`** — activate when the diff touches DB queries, data transforms on potentially large inputs, caching, or async code paths. Concretely: SQL/Prisma query construction, sort/group/aggregate logic, cache read/write/invalidation, new `async`/`await` usage in hot paths, anything touching a loop whose bound comes from untrusted data. Changes to one-shot, small-N, non-hot-path code usually do not.
- **`adversarial`** — activate when **both** (a) the diff is **50+ changed non-test lines**, **and** (b) the diff touches any CAS high-stakes module: task verification flow (`close_ops`, `verify_ops`), factory coordination (spawn / message / queue / lifecycle), SQLite store mutations, hook system (`pre_tool`, `post_tool`), MCP tool dispatch. Both conditions must hold. Additionally, **always skip adversarial** when the total diff is **under 20 changed lines** regardless of which files are touched — small surgical fixes do not warrant the heaviest persona's stress-testing budget. Tiny diffs that genuinely warrant adversarial scrutiny (e.g., a 5-line concurrency primitive change) are vanishingly rare and `correctness` already covers logic bugs in that range. Record the activation decision and reason in the output envelope as before. This is the persona that stress-tests for "what could go wrong under concurrent factory sessions", "what happens when this lease expires mid-operation", "what's the cascade when this assertion fires in production" — exactly the failure modes the Phase 0 debugging history documents.

Record your activation decision explicitly in the output envelope so a reader can see *which* personas ran and *why* the conditional ones were included. Example: `"activated": ["security", "adversarial"], "activation_reason": {"security": "diff adds a new MCP tool handler that parses untrusted input", "adversarial": "diff is 83 non-test lines and touches close_ops.rs"}`.

## Step 3 + Step 4: Workflow dispatch + deterministic merge (Phase B — cas-b667)

**Steps 3 and 4 now run inside the `cas-code-review` Workflow** (`.claude/workflows/cas-code-review.js`). You no longer spawn Task tools manually for personas. Instead:

1. Pre-fetch the diff and file list:
   ```bash
   git diff <base_sha>..HEAD          # → diff_text
   git diff --name-only <base_sha>..HEAD  # → file_list
   ```

2. Determine whether fallow should run (JS/TS repo check — no package.json AND no .ts/.js in diff → skip, record reason).

3. Call the Workflow:
   ```
   Workflow({
     name: 'cas-code-review',
     args: {
       diff_text, file_list, base_sha,
       intent_summary,        // from Step 1
       activated_personas,    // from Step 2 selection
       fallow_skip_reason,    // string or null
       mode, task_id,
     }
   })
   ```

4. The Workflow returns `{ residual, pre_existing, activation, stats }`. Use this as the merged findings envelope for Step 5.

**What the Workflow does internally (Steps 3-4):**
- Fans out all activated personas in parallel (`pipeline()`) with schema-validated `ReviewerOutput` output (hard enforcement + auto-retry on mismatch). All personas on Sonnet per R13.
- Merges with Phase-A-validated `mergeFindings()` (7-step deterministic JS pipeline, 30 unit tests, byte-identical on re-run).
- Re-run on same diff = 100% cache hit, 0 tokens (~10ms).

**What stays in this skill:** Steps 1-2 (intent + activation), Step 5 (mode routing), all CAS integration (modes, task notes, fix loop, review→task routing, P0 hard-block gate).

**Workflow delivery:** `.claude/workflows/cas-code-review.js` and `cas-code-review-constants.js` are shipped via `cas update --sync` / `cas init` (builtin-workflows delivery mechanism). See Gap 2 note in Phase B results — delivery path is a Phase C item if not yet wired.

## Step 5: Mode-specific output

With merged findings in hand, branch on `mode`:

- **`autofix`** — the primary path. Feed the merged output to Unit 7 (the autofix loop). Unit 7 runs at most 2 rounds: it applies `safe_auto` findings via the fixer sub-agent, re-runs this orchestrator on the patched tree, and exits after round 2 regardless of residual findings. After Unit 7 returns, any residual non-`safe_auto` findings are routed to CAS tasks via the Phase 1 review-to-task subsystem (Unit 8), with priority mapping `P0→0`, `P1→1`, `P2→2`, `P3→3`. `advisory` findings never become tasks. Any surviving `P0` finding hard-blocks the close; the worker must either fix it and retry, or record a downgrade note and request supervisor override (R9).
- **`interactive`** — render the merged output to the user in a readable format (severity-sorted, grouped by reviewer, file+line anchored), offer the bounded 2-round fix loop as an explicit choice, and wait for the human to decide.
- **`report-only`** — write the merged envelope to a file under `docs/reviews/<YYYY-MM-DD>-<short-ref>.md` (or a caller-provided path) and exit. No edits. No task creation. No `task.close` side effects. Safe to run in parallel with other reviews.
- **`headless`** — return the merged envelope as a single structured text blob to the caller. No side effects beyond that return value. The caller decides what to do with the findings.

In every mode, the output envelope includes the activation decision from Step 2 and a per-persona status table so the caller can tell which reviewers ran, which succeeded, and which errored.

## Review ownership model (cas-b51a)

CAS supports two review ownership modes, configured via `[code_review] owner = "worker" | "supervisor"` in `.cas/config.toml`:

| `owner` | Worker behavior at close | Supervisor responsibility |
|---|---|---|
| `supervisor` **(default)** | Runs lightweight structural lint (<1s); task transitions to `pending_supervisor_review` | Supervisor runs `/cas-code-review mode=interactive` at cherry-pick and at EPIC→base merge |
| `worker` (opt-out / legacy) | Runs the full `autofix` pipeline inline; close blocks until review completes (~14 min) | None — workers self-certify |

The default is `supervisor`. Pin to legacy worker-owned behavior with `[code_review] owner = "worker"` in `.cas/config.toml`.

## Mode reference

The four invocation modes, per brainstorm R8 + R5 + R9–R11:

| Mode | Trigger | Edits files? | Creates tasks? | Gates close? | Fix loop | Notes |
|---|---|---|---|---|---|---|
| `autofix` | Automatic at factory worker `task.close` (legacy, `owner=worker` only — opt-in) | Yes, via fixer sub-agent on `safe_auto` | Yes, residual non-`safe_auto` → CAS tasks with `P0→0…P3→3` | Yes — any P0 hard-blocks; supervisor override required to downgrade | Bounded `max_rounds=2` | R5, R9, R10, R11. Legacy opt-in path for `owner=worker` projects. |
| `interactive` | Used by supervisor at cherry-pick (per-task) and at EPIC→base merge (integration sweep) — primary path under default `owner=supervisor`. Also available for manual human invocation. | Only via fixer if user accepts the offered loop | Only if user accepts | No | Bounded 2-round on user consent | R8. Full UX; show findings, let the human drive. |
| `report-only` | Manual or scheduled | No | No | No | None | R8. Safe for parallel runs; strictly read-only. |
| `headless` | Skill-to-skill call | No (orchestrator itself does not edit) | No | No | None | R8. Returns merged envelope as structured text; caller decides next steps. |

## Inputs from upstream units

This skill consumes, but does not implement:

- **Unit 1 (`cas-cfb5`)** — findings schema at `crates/cas-types/src/code_review.rs` + the human-readable doc at `references/findings-schema.md`. The personas and the merge pipeline both validate against this.
- **Unit 2 (`cas-1e98`)** — the 7 persona prompt files under `references/personas/`. You load these verbatim and hand them to the Task tool.
- **Unit 3 (`cas-c663`)** — the base-SHA resolution helper at `crates/cas-store/src/code_review/base_sha.rs`. You call this when the caller did not supply `base_sha`.

Downstream units this skill hands off to (not implemented here):

- **Unit 5** — merge pipeline. See Step 4.
- **Unit 6** — distribution (BuiltinFile registration and legacy `code-reviewer` cutover).
- **Unit 7** — autofix fixer sub-agent loop.
- **Unit 8** — review-to-task routing.

## Failure modes and how to handle them

- **A persona returns invalid JSON.** Record as a reviewer error in the envelope and continue. Do not retry. Do not fabricate findings.
- **Every persona returns no findings.** This is a clean pass — report it honestly; do not invent noise to justify the latency.
- **The Unit 3 helper returns `AllStrategiesFailed`.** Surface the error to the caller; do not fall back to a made-up base. A review without a base is worse than no review.
- **The diff is empty (no changed files).** Return an empty merged envelope with a clear note; do not run personas against nothing.
- **The activation judgment is genuinely uncertain** (e.g., "is this async code in a hot path?"). Prefer activation — false positives cost wall-clock; false negatives cost correctness.
