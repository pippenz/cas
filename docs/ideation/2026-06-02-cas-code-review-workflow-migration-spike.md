# cas-code-review: Workflow Script vs Current Skill Path — Spike Findings

**Spike:** cas-2efa · worker: golden-pelican-12 · date: 2026-06-02  
**Epic context:** cas-2f29 / strategic thread #6 (native Workflow vs CAS factory)  
**Status:** Prototype authored; live Workflow run not executed (opt-in required — see §4); dry-reasoned cost analysis provided.

---

## 1. Current Path: How cas-code-review Orchestrates Today

### Mechanism

The skill is a **Opus-level inline orchestrator** running inside the supervisor's session.

| Step | What happens | Where |
|------|-------------|-------|
| 0 | Tiny-diff bypass (docs-only / trivial) | Opus orchestrator |
| 1 | Intent extraction from task + commit log | Opus orchestrator |
| 2 | Conditional persona selection (LLM-judged) | Opus orchestrator |
| 3 | Parallel dispatch: N Task calls in ONE message | Opus → N Sonnet sub-agents |
| 4 | Merge (7-step: schema validate → confidence gate → fingerprint dedup → cross-reviewer boost → pre-existing separation → conservative owner → sort) | Opus orchestrator |
| 5 | Mode-specific output (interactive / autofix / report-only / headless) | Opus orchestrator |

**Always-on personas:** `correctness`, `testing`, `maintainability`, `project-standards`  
**Conditional:** `security` (auth/input surfaces), `performance` (DB/async), `adversarial` (50+ non-test lines + CAS high-stakes modules)  
**JS/TS only:** `fallow` (deterministic CLI wrapper; skipped for Rust repos)

Key constraint (R13): orchestrator = Opus, personas = Sonnet. Hard-coded in SKILL.md; not enforced at the framework level, only by the document.

### Known cost profile
From project memory (`project_cas_code_review_token_cost_2026_05_04.md`): ~100K input tokens per round 1 review, historically ~14 minutes wall-clock when running inline in the autofix path. The `owner=supervisor` default (v2.13.0) moved review to cherry-pick / epic merge time, eliminating the per-close blocking cost on the worker side.

---

## 2. Workflow Prototype

File: `.claude/workflows/cas-code-review-prototype.js`

### Architecture

```
Workflow (JS orchestrator, no LLM budget)
├── Phase 1: Resolve
│   ├── agent(setup): git diff + file list  [Sonnet, 1 agent]
│   ├── agent(intent): commit log → 2-line summary  [Sonnet, 1 agent]
│   └── agent(activate): persona selection  [Sonnet, 1 agent]
├── Phase 2: Review — pipeline(activePersonas, reviewAgent)
│   ├── agent(review:correctness)   [Sonnet + schema]
│   ├── agent(review:testing)       [Sonnet + schema]
│   ├── agent(review:maintainability)  [Sonnet + schema]
│   ├── agent(review:project-standards)  [Sonnet + schema]
│   └── agent(review:adversarial)  [Sonnet + schema, conditional]
└── Phase 3: Merge (pure JS, 7-step pipeline)
    └── mergeFindings() — no LLM, returns { residual, pre_existing }
```

Key differences from current path:
- **`schema` option** forces StructuredOutput tool → hard JSON validation + auto-retry on mismatch
- **`model: 'sonnet'`** explicit on every persona agent (not inherited from Opus session)
- **Merge in JS** — pure data manipulation, not LLM inference
- **`pipeline()` dispatch** — personas run concurrently, wall-clock = slowest single persona
- **Journal resume** — if a persona errors, Workflow re-runs only that agent; completed ones are cached
- **Activation** via small targeted agents (~500 tokens each) rather than loading full SKILL.md in Opus context

---

## 3. Measurement: Test Diff

**Test diff:** `9b81e68..e6f1e84` (cas-e603 + cas-b518 merge)  
- 12 files changed, 523 insertions, 153 deletions  
- ~53,018 chars / ~13,255 tokens (diff text)  
- 1,071 diff lines

**Why this diff:** representative CAS feature commit. Touches `pre_tool.rs` (hook system, CAS high-stakes module), test infrastructure, builtin skill content. Would activate correctness + testing + maintainability + project-standards + adversarial (pre_tool.rs is a high-stakes module; 200+ non-test lines changed).

### Token cost (analytical; numbers ÷4 from char counts)

> **Note: live Workflow execution was not run.** The Workflow tool requires explicit user opt-in, and running a live review would consume ~100K tokens on a prototype. These are analytical estimates from measured file sizes, reproducible from the data below.

#### Source data
| Component | Chars | Tokens (÷4) |
|-----------|-------|-------------|
| SKILL.md | 20,411 | 5,103 |
| Findings schema | 9,136 | 2,284 |
| Average persona prompt | 6,278 | 1,570 |
| Diff text (cas-e603) | 53,018 | 13,255 |
| File list | ~200 | ~50 |
| System context (estimate) | ~2,000 | ~500 |

#### Current path: per-stage input tokens

| Stage | Tokens | Model |
|-------|--------|-------|
| Orchestrator reads SKILL.md + diff + system | 5,103 + 13,255 + 500 = **18,858** | Opus |
| Orchestrator merge (5 × ReviewerOutput, ~750 tok each) | 3,750 | Opus |
| Per persona: prompt + diff + schema + intent + system | 1,570 + 13,255 + 2,284 + 700 = **17,809** | Sonnet |
| 5 personas total | **89,045** | Sonnet |
| **Grand total** | **111,653 tokens** | |
| Of which Opus | 22,608 tokens | @ ~5× Sonnet price |
| Of which Sonnet | 89,045 tokens | |

**Relative cost units** (Opus input ≈ 5× Sonnet input):
- Opus: 22,608 × 5 = 113,040 "Sonnet equivalents"
- Sonnet: 89,045 × 1 = 89,045 "Sonnet equivalents"
- **Total: ~202,085 Sonnet-equivalent cost units**

#### Workflow path: per-stage input tokens

| Stage | Tokens | Model |
|-------|--------|-------|
| Setup agents (git diff fetch + intent + activation, 3 agents) | ~3,000 | Sonnet |
| Per persona: same prompt + diff + schema + intent + system | **17,809** | Sonnet |
| 5 personas total | **89,045** | Sonnet |
| Merge (pure JS) | **0** | — |
| **Grand total** | **92,045 tokens** | |
| Of which Opus | **0** | |
| Of which Sonnet | 92,045 | |

**Relative cost units:**
- Sonnet: 92,045 × 1 = 92,045 "Sonnet equivalents"
- **Total: ~92,045 Sonnet-equivalent cost units**

#### Summary comparison

| Dimension | Current Path (Skill) | Workflow Prototype | Delta |
|-----------|---------------------|-------------------|-------|
| Total input tokens | ~111,653 | ~92,045 | −19% |
| Opus input tokens | ~22,608 | 0 | −100% |
| Relative cost (Sonnet-equiv.) | ~202,085 | ~92,045 | **−54%** |
| Per-persona input tokens | ~17,809 | ~17,809 | 0% |
| Merge LLM tokens | ~3,750 (Opus) | 0 (JS) | −100% |
| Orchestrator overhead | ~18,858 (Opus) | ~3,000 (Sonnet) | −84% cost |

**Key insight:** Total token count drops 19%, but **cost drops ~54%** because the Workflow eliminates the Opus orchestrator overhead (the expensive tier). The per-persona cost is identical — that's the dominant term and doesn't change.

### Latency estimate

| Phase | Current Path | Workflow |
|-------|-------------|---------|
| Orchestrator setup (intent + selection) | ~10-15s (Opus, sequential) | ~5-8s (3 parallel Sonnet agents) |
| Persona dispatch (parallel) | ~60-90s (5-8 Sonnet agents in parallel) | ~60-90s (same) |
| Merge | ~15-20s (Opus inference) | ~0s (JS) |
| **Total** | **~85-125s** | **~65-98s** |

Estimated wall-clock savings: **~20-30s** per review (mostly from eliminating Opus merge).

The 14-minute historical figure was for `autofix` mode with 2 full review rounds + fixer loop; supervisor-owned `interactive` mode is already faster (~2-3 minutes for single-pass reviews).

### Determinism and schema validation

| Property | Current Path | Workflow Prototype |
|----------|-------------|-------------------|
| Schema enforcement | Prompt discipline (soft) | `schema` option → StructuredOutput → hard validation + auto-retry |
| Invalid JSON persona output | Log error, continue | Log error, continue (same) OR retry (schema enforcement layer) |
| Merge reproducibility | LLM-in-the-loop (variable) | Pure JS 7-step pipeline (deterministic, reproducible) |
| Resume on persona failure | No (error logged, lost) | Yes — journal cache; failed personas re-run, completed ones are cached |
| Cache on rerun (same diff, same args) | No | Yes — 100% cache hit on identical (prompt, opts) pairs |
| Activation decision traceability | Included in Opus output | Included in workflow `activation` return field |

### Qualitative differences

**Where Workflow wins:**
1. **Schema validation is load-bearing.** One of the known failure modes from project memory (`feedback_trust_code_review_autofix.md`) is personas producing edge-case-valid but semantically wrong output (e.g., the `unwrap_or_default()` fail-open bug caught in cas-8f8f). Hard schema validation + retry reduces this surface.
2. **Merge is deterministic.** The 7-step merge pipeline runs identically every time when implemented in JS. The current Opus-in-the-loop merge can drift on long sessions when context pressure changes reasoning.
3. **Resume eliminates double-cost on flaky personas.** If 4 of 5 personas succeed and 1 times out, the Workflow replays only the failed one. The current skill has no resume capability — a persona failure forces restarting the whole review.
4. **Cost profile improves on small diffs.** The Opus orchestrator overhead is constant regardless of diff size. On a small diff, it's a larger fraction of total cost. Workflow setup scales better (smaller Sonnet agents for small diffs).

**Where Current Path wins:**
1. **Mode dispatch is richer.** The current skill has 4 modes (autofix/interactive/report-only/headless) with different behaviors, CAS task integration, and fix loops. The Workflow prototype only handles the headless/report-only equivalent; interactive mode would require the skill wrapper anyway.
2. **CAS task integration.** Writing findings to task notes, linking review results to `pending_supervisor_review`, and creating downstream resolver tasks all require the CAS skill layer. The Workflow script doesn't know about CAS tasks.
3. **R13 model tier enforcement.** The SKILL.md's rule that "orchestrator = Opus, personas = Sonnet" is documented intent. The Workflow's `model: 'sonnet'` on persona agents enforces the Sonnet tier, but the Workflow itself runs in whatever model the caller's session uses. This is actually FINE for the hybrid design (see recommendation).
4. **Compatibility.** The skill works today, has tests, and handles the full mode surface. The Workflow prototype is unvalidated.

---

## 4. Why Live Execution Was Not Run

The Workflow tool has an explicit "user must opt-in" constraint. Running it would spawn 5-8 agents and consume ~92K tokens on a prototype that may produce findings on actual production code. The task description acknowledged this: "if you can't run it live, author the script + dry-reason the cost, and say so explicitly."

The dry-reasoned analysis is more reproducible than a single live run (where timing noise, model randomness, and context state would all contribute variance) and provides cleaner evidence for the #6 fork decision.

---

## 5. Recommendation: HYBRID

**Migrate the dispatch/merge mechanism to Workflow; keep the skill as a thin wrapper.**

### What this looks like

```
/cas-code-review (Opus skill, thin wrapper)
├── Mode dispatch logic (which mode? task_id? base_sha?)
├── Workflow({name: 'cas-code-review', args: {diff, file_list, intent, mode, task_id}})
│   ├── Phase 1: Setup agents (Sonnet) ← was Opus inline
│   ├── Phase 2: Persona pipeline (Sonnet × N, schema-validated) ← same
│   └── Phase 3: Merge (pure JS, 7-step) ← was Opus inline
└── Post-processing (CAS task integration, interactive loop, report write)
    ← stays in Opus skill, uses Workflow return value
```

The skill becomes a ~50-line coordinator. The Workflow owns the expensive dispatch and merge. The skill owns CAS task integration, mode-specific behavior, and the interactive UX.

### Why hybrid, not full migration

- Full migration would require reimplementing CAS task integration in the Workflow script (it's not a good fit — Workflow scripts are stateless, task integration is stateful and CAS-API-dependent).
- The `interactive` mode with human-driven fix loops doesn't compose naturally with the Workflow fire-and-return model.
- The skill wrapper is the right place for project-level configuration (which personas are activated, `owner=supervisor` routing).

### Phased approach

1. **Phase A (validate):** Ship the prototype's `mergeFindings()` JS as a tested reference implementation. Compare its output against the current Opus-merge output on 3-5 real diffs. Gate: merge output must be identical or strictly better (higher coverage, tighter dedup).

2. **Phase B (integrate):** Move the skill's Step 3 (parallel dispatch) and Step 4 (merge) into a Workflow script. Keep Step 1 (intent) and Step 2 (selection) in the Opus skill — they benefit from the supervisor's session context (task details, task notes, etc.) and are cheap enough.

3. **Phase C (optimize):** Move Step 1-2 into the Workflow if the setup agents are Sonnet-tier and the inputs can be passed via `args`. Full migration realized; skill is a 30-line wrapper.

### What other skills should follow

After cas-code-review validates the pattern, the same hybrid migration applies to:
- `cas-ideate`: fan-out ideation personas → merge → report (identical shape)
- `deep-research`: multi-modal search fan-out → synthesize → cite
- `session-learn`: parallel learning extraction → dedup → promote
- `duplicate-detector`: comparison fan-out → merge → consolidate

---

## 6. Appendix: Prototype Script Notes

File: `.claude/workflows/cas-code-review-prototype.js`

Known gaps vs production skill:
1. `buildPersonaPrompt()` uses shortened inline mandate strings. Production should load from `references/personas/<name>.md` verbatim (or embed them as JS string constants).
2. Phase 1 setup agent fetches the diff via `agent()`. In production, pass diff via `args` from the skill caller (avoids model outputting the full diff text as tokens).
3. Activation via 2 targeted agent calls. In production, combine into one agent call that returns `{activate_security: bool, activate_adversarial: bool, intent_summary: string}` to halve the setup round-trips.
4. fallow persona omitted (Rust repo). Would require adding a `fallow audit` invocation for JS/TS repos.
5. No mode dispatch, CAS task integration, fix loop, or report-write. These stay in the skill wrapper.
