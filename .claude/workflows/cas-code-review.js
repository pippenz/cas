// cas-code-review.js — production Workflow for cas-code-review Steps 3-4
//
// Phase B of EPIC cas-b667. Handles Step 3 (parallel persona dispatch,
// schema-validated) and Step 4 (deterministic JS merge). Steps 1-2 and
// all CAS integration stay in the skill wrapper (SKILL.md).
//
// Self-contained: Workflow scripts run in a custom runtime that does not
// support ES module import statements. All helpers are inlined.
// For test imports of constants, see cas-code-review-constants.js.
//
// Called by the cas-code-review skill:
//   Workflow({ name: 'cas-code-review', args: {
//     diff_text,           // full git diff (pre-fetched by skill)
//     file_list,           // newline-separated changed file paths
//     base_sha,            // base commit SHA
//     intent_summary,      // 2-3 line synthesis from skill Step 1
//     activated_personas,  // array of names from skill Step 2 LLM judgment
//     fallow_skip_reason,  // non-null string if fallow should skip
//     mode,                // 'interactive'|'report-only'|'headless'|'autofix'
//     task_id,             // optional CAS task ID
//   }})
//
// Returns: { residual, pre_existing, activation, stats }

export const meta = {
  name: 'cas-code-review',
  description: 'cas-code-review Steps 3-4: schema-validated persona dispatch + deterministic merge',
  phases: [
    { title: 'Resolve', detail: 'validate args + fallow pre-check' },
    { title: 'Review', detail: 'parallel persona dispatch (schema-validated, Sonnet)' },
    { title: 'Merge', detail: 'deterministic 7-step merge (pure JS, no LLM)' },
  ],
}

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────────────────

const ALWAYS_ON_PERSONAS = ['correctness', 'testing', 'maintainability', 'project-standards']

// ─────────────────────────────────────────────────────────────────────────────
// SCHEMA — mirrors ReviewerOutput + Finding from crates/cas-types/src/code_review.rs
// ─────────────────────────────────────────────────────────────────────────────

const FINDING_SCHEMA = {
  type: 'object',
  required: ['title','severity','file','line','why_it_matters','autofix_class','owner','confidence','evidence','pre_existing'],
  additionalProperties: false,
  properties: {
    title:                { type: 'string', maxLength: 100 },
    severity:             { type: 'string', enum: ['P0','P1','P2','P3'] },
    file:                 { type: 'string' },
    line:                 { type: 'integer', minimum: 1 },
    why_it_matters:       { type: 'string' },
    autofix_class:        { type: 'string', enum: ['safe_auto','gated_auto','manual','advisory'] },
    owner:                { type: 'string', enum: ['review-fixer','downstream-resolver','human'] },
    confidence:           { type: 'number', minimum: 0.0, maximum: 1.0 },
    evidence:             { type: 'array', items: { type: 'string' }, minItems: 1 },
    pre_existing:         { type: 'boolean' },
    suggested_fix:        { type: 'string' },
    requires_verification:{ type: 'boolean' },
  },
}

const REVIEWER_OUTPUT_SCHEMA = {
  type: 'object',
  required: ['reviewer', 'findings'],
  additionalProperties: false,
  properties: {
    reviewer:       { type: 'string' },
    findings:       { type: 'array', items: FINDING_SCHEMA },
    residual_risks: { type: 'array', items: { type: 'string' } },
    testing_gaps:   { type: 'array', items: { type: 'string' } },
  },
}

// ─────────────────────────────────────────────────────────────────────────────
// PERSONA PROMPTS — condensed from references/personas/*.md
// Full verbatim versions in cas-code-review-constants.js (importable by tests)
// ─────────────────────────────────────────────────────────────────────────────

const PERSONA_PROMPTS = {

correctness: `# Persona: correctness
Run as a Sonnet sub-agent. Do not inherit caller model.

Hunt for defects that make the changed code wrong — logic errors, broken execution paths, failure modes the author did not consider. Trace the full execution path: inputs, branches, early returns, error propagation, invariants. If you cannot construct a concrete input that triggers the bug, confidence must reflect that.

In scope: off-by-one/boundary errors, None/null propagation to unchecked dereferences, race conditions (check-then-act, lease/lock handling, async cancellation), broken error handling (swallowed errors, Result ignored, retry without backoff/bound), contract violations, resource leaks (file handles, DB connections, locks, temp files), arithmetic bugs (overflow, truncation, float equality). Structural red-flags: Rust bare .unwrap()/.expect() on fallible input, todo!()/unimplemented!(), #[allow(dead_code)] on new code, let _ = <fallible>. TypeScript: $EXPR as any, // @ts-ignore without justification, empty catch. Dead/unwired new public code with zero references.

Out of scope: testing→testing, naming/duplication→maintainability, JS imports→fallow, auth/input→security, DB/async hot paths→performance, blast-radius→adversarial, CAS rules→project-standards.

Output ONLY: {"reviewer":"correctness","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
confidence ≥ 0.80: reproducible from code alone. 0.60–0.79: sound, inference gap stated. <0.60: use residual_risks. P0 threshold: ≥ 0.50. No prose outside the JSON envelope.`,

testing: `# Persona: testing
Run as a Sonnet sub-agent. Do not inherit caller model.

Hunt for gaps and weaknesses in test coverage of the changed code. Answer: if this diff broke, would a test fail? For every new/modified non-test symbol, verify a test would catch a plausible regression.

In scope: missing coverage for new/modified paths, branches, error paths; weak assertions (would pass on wrong return value); over-mocking hiding integration bugs; flaky patterns (time-dependent, hash-map iteration order, sleep instead of sync primitive); test anti-patterns introduced (#[ignore]/it.skip/pytest.mark.skip without linked issue, commented-out assertions, assert true); new public API with no test file.

Out of scope: logic bugs in production code→correctness, test-file rule-compliance→project-standards. Test duplication is often intentional — lenient.

Output ONLY: {"reviewer":"testing","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
file/line point at the production symbol with missing coverage. Most: manual, review-fixer or downstream-resolver. confidence 0.80+: confirmed absence by reading test files. No prose outside JSON.`,

maintainability: `# Persona: maintainability
Run as a Sonnet sub-agent. Do not inherit caller model.

Hunt for changes that make the codebase harder to read, reason about, or extend six months from now.

In scope: duplication (block exists elsewhere — grep the repo); naming drift (convention conflicts with neighbors); dead code (branches, params, fields, imports never read — grep-verified); premature/broken abstraction (helper for one caller, interface with one implementor); inappropriate abstraction level (business logic in serializer, SQL in handler); comment rot (contradicts code, stale doc-comment names old param); oversized functions introduced (400+ lines); backwards-compatibility cruft without justification in new code.

Out of scope: logic bugs→correctness, test quality→testing, rule violations→project-standards, security→security, performance→performance. Subjective style (tabs/spaces, import order, line length) — do not flag.

Output ONLY: {"reviewer":"maintainability","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
Most: P2/P3, advisory or manual. P0/P1 rare. No prose outside JSON.`,

'project-standards': `# Persona: project-standards
Run as a Sonnet sub-agent. Do not inherit caller model.

Hunt for violations of the project's explicit, enforceable standards — CAS rules from mcp__cas__rule plus CLAUDE.md/AGENTS.md conventions. Enforce what this project has decided. Do not invent rules.

In scope: CAS rule compliance (run mcp__cas__rule action=list at start; check active rules against changed files; cite rule ID in title prefix, rule body in evidence); CLAUDE.md/AGENTS.md conventions enforceable objectively; managed-file headers (file with managed_by: cas modified without going through generator); module-boundary rules when documented; forbidden API calls listed in rules (e.g., no println! in library code, no TodoWrite); naming conventions when codified in a rule.

Out of scope: logic bugs→correctness, test coverage→testing, subjective readability without stated rule→maintainability. Inactive/draft/archived rules.

Output ONLY: {"reviewer":"project-standards","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
Rule ID in title prefix (e.g., "rule-1234: ..."). confidence 0.80+: explicit rule + clear violation. <0.60: suppress. No prose outside JSON.`,

security: `# Persona: security
Run as a Sonnet sub-agent. Do not inherit caller model.
ACTIVATION: Confirmed by caller — diff touches auth boundaries, user input parsing/deserialization, or permission surfaces.

Hunt for exploitable defects — where malicious/malformed input, stolen credential, or authorization misuse lets an attacker read, write, or execute something they should not. Think in threat models: attacker input → boundary → target. Evidence-grounded, reproducible-from-code reasoning required.

In scope: injection (SQL, command, path traversal, template, header); broken authentication (missing/weak session validation, non-constant-time comparison, weak hashing); broken authorization (missing permission check, IDOR, capability upgrade, jail escape); sensitive data exposure (hardcoded secrets, secrets logged, PII to analytics); cryptographic misuse; deserialization of untrusted input; SSRF/open redirect; CAS-specific: new MCP tool without jail/permission checks, hook with elevated privileges, worker path influencing supervisor state; TOCTOU on permission checks.

Out of scope: pure correctness with no threat model→correctness. Theoretical with no reachable path.

Output ONLY: {"reviewer":"security","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
Default P0/P1, owner:human, manual autofix. confidence 0.80+: trace attacker input to sink. <0.60: suppress. No prose outside JSON.`,

performance: `# Persona: performance
Run as a Sonnet sub-agent. Do not inherit caller model.
ACTIVATION: Confirmed by caller — diff touches DB queries, data transforms on large inputs, caching, or async code paths.

Hunt for code that will be slower, more wasteful, or less scalable than a reasonable alternative. Care about asymptotic complexity, unbounded work, async pitfalls. Every finding must point at a concrete cost scenario.

In scope: N+1 queries; unbounded queries/collections (SELECT without LIMIT, find_all without pagination); missing/wrong indexes; blocking work in async runtime (std::fs, std::thread::sleep in tokio); lock contention / await-while-holding-lock (Mutex held across .await); algorithmic complexity (O(n²) where O(n) straightforward); cache invalidation bugs; wasteful allocation in hot paths; thundering herd/retry storms (no jitter, backoff multiplier=1); connection pool misuse.

Out of scope: correctness→correctness, attacker-controlled DoS→security, test speed→testing.

Output ONLY: {"reviewer":"performance","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
Most: P1/P2, manual or gated_auto. confidence 0.80+: traced data flow + concrete cost scenario. <0.60: suppress. No prose outside JSON.`,

adversarial: `# Persona: adversarial
Run as a Sonnet sub-agent. Do not inherit caller model.
ACTIVATION: Confirmed by caller — 50+ changed non-test lines AND touches CAS high-stakes modules (close_ops, verify_ops, factory coordination, SQLite stores, hook system, MCP dispatch). Skip for diffs under 20 non-test lines.

Red-team reader. Ask: what is the worst this change could plausibly do, and how would we know? Surface risks the other personas miss because they are in-lane. Reason about blast radius, reversibility, multi-component interactions, failures that appear only under concurrent factory sessions or production state.

In scope: blast-radius misjudgment (small refactor changes function used by 30 callers); reversibility gaps (migration without rollback, destructive op without dry-run); invariant erosion (existing invariants weakened — e.g., task in pending_verification cannot be closed); cross-component coupling (implicit assumption another module doesn't guarantee); state machine corruption (unmapped state, missing guard, exhaustive match missing arm); concurrency traps at system level (two workers racing on lease, supervisor/worker seeing different task state); failure-mode asymmetry (error path leaves ghost tasks, leaked processes); operational surprises (log on hot path, metric break); if CAS project memory records a past incident class this diff reopens, call it out explicitly.

Out of scope: narrow single-lane findings→owning persona, aesthetic concerns, speculation untethered from diff.

Output ONLY: {"reviewer":"adversarial","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
Almost always manual, owner:human or downstream-resolver. Severity = blast radius, not likelihood. confidence 0.80+: specific invariant broken + historical incident class. <0.60: suppress. No prose outside JSON.`,

fallow: `# Persona: fallow
Run as a Sonnet sub-agent. Do not inherit caller model. Adapter, not auteur — run the CLI and translate output.
ACTIVATION: JS/TS repo with JS/TS files in diff (pre-checked by caller).

Skip rules — return clean envelope with residual_risks entry:
1. No package.json or tsconfig.json at repo root (outside node_modules).
2. No .ts/.tsx/.js/.jsx/.mjs/.cjs/.vue/.svelte/.astro/.mdx in changed_files.
3. fallow CLI not available (command -v fallow and npx fallow --version both fail).
4. Fallow runtime error (exit code 2).

Run: fallow audit --format json --quiet --explain --base <base_sha>
Exit 0 (pass) or 1 (issues) are normal; only 2 is error.

Translation: file→file (relative), line→start_line, issue-type→title "[fallow] <type>: <symbol>" (≤100 chars). error→P1, warning→P2, info→P3. auto_fixable→safe_auto/review-fixer; else manual/downstream-resolver. pre_existing: true if fallow attribution shows introduced: false. Confidence: 0.95 introduced, 0.80 pre-existing.

Output ONLY: {"reviewer":"fallow","findings":[...],"residual_risks":[...],"testing_gaps":[]}
reviewer MUST be "fallow". No prose outside JSON.`,

}

// ─────────────────────────────────────────────────────────────────────────────
// MERGE PIPELINE — Phase A validated (merge-findings.js, 30 unit tests)
// Inlined here since Workflow scripts cannot use import statements.
// ─────────────────────────────────────────────────────────────────────────────

const OWNER_RANK = { 'human': 2, 'downstream-resolver': 1, 'review-fixer': 0 }

function fingerprint(f) {
  const title = f.title.toLowerCase().replace(/[^a-z0-9]/g, ' ').replace(/\s+/g, ' ').trim()
  const bucket = Math.floor(f.line / 3)
  return `${f.file}|${bucket}|${title}`
}

function mergeFindings(reviewerOutputs) {
  const allFindings = reviewerOutputs.filter(Boolean).flatMap(r => r.findings || [])
  const gated = allFindings.filter(f =>
    f.severity === 'P0' ? f.confidence >= 0.50 : f.confidence >= 0.60
  )
  const byFp = new Map()
  for (const f of gated) {
    const fp = fingerprint(f)
    if (!byFp.has(fp)) {
      byFp.set(fp, { finding: { ...f }, count: 1 })
    } else {
      const entry = byFp.get(fp)
      entry.count++
      const boosted = Math.min(1.0, entry.finding.confidence + 0.10)
      const currentRank = OWNER_RANK[entry.finding.owner] ?? 0
      const incomingRank = OWNER_RANK[f.owner] ?? 0
      entry.finding = {
        ...entry.finding,
        confidence: boosted,
        owner: incomingRank > currentRank ? f.owner : entry.finding.owner,
      }
    }
  }
  const deduped = Array.from(byFp.values()).map(e => e.finding)
  const residual = deduped.filter(f => !f.pre_existing)
  const pre_existing = deduped.filter(f => f.pre_existing)
  const SEV_ORDER = { P0: 0, P1: 1, P2: 2, P3: 3 }
  residual.sort((a, b) =>
    (SEV_ORDER[a.severity] - SEV_ORDER[b.severity]) || (b.confidence - a.confidence)
  )
  return { residual, pre_existing }
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

function buildPersonaPrompt(name, diffText, fileList, intentSummary, baseSha) {
  const body = PERSONA_PROMPTS[name] ?? `# Persona: ${name}\n(Unknown persona — emit empty envelope)`
  return `${body}

---

## Change being reviewed

**Intent:** ${intentSummary}

**Base SHA:** ${baseSha}

**Changed files:**
${fileList}

**Full diff:**
\`\`\`diff
${diffText}
\`\`\`

**Findings contract** — output MUST be a single JSON object:
{"reviewer":"${name}","findings":[...],"residual_risks":[...],"testing_gaps":[...]}
The canonical schema for Finding and ReviewerOutput is in references/findings-schema.md.
Each finding needs: title (≤100 chars), severity (P0-P3), file (relative path),
line (1-based int), why_it_matters, autofix_class (safe_auto|gated_auto|manual|advisory),
owner (review-fixer|downstream-resolver|human), confidence (0.0-1.0),
evidence (array ≥1 code-grounded string), pre_existing (bool).
Do NOT emit any prose outside the JSON envelope.`
}

// ─────────────────────────────────────────────────────────────────────────────
// SETUP_SCHEMA — Phase C (inlined; Workflow scripts cannot import from ES modules)
// Combined Steps 1-2 agent output: intent extraction + persona selection in one call.
// ─────────────────────────────────────────────────────────────────────────────

const SETUP_SCHEMA = {
  type: 'object',
  required: ['intent_summary', 'activate_security', 'activate_adversarial', 'activate_performance', 'fallow_skip_reason'],
  additionalProperties: false,
  properties: {
    intent_summary:       { type: 'string' },
    activate_security:    { type: 'boolean' },
    activate_adversarial: { type: 'boolean' },
    activate_performance: { type: 'boolean' },
    fallow_skip_reason:   { type: ['string', 'null'] },
  },
}

// ─────────────────────────────────────────────────────────────────────────────
// WORKFLOW BODY — Steps 1-4 (Phase C: Steps 1-2 now inside Workflow)
//
// Skill wrapper passes: diff_text, file_list, base_sha, commit_log,
//   task_context (optional), mode, task_id (optional)
// Workflow handles: intent extraction, persona selection, dispatch, merge
// ─────────────────────────────────────────────────────────────────────────────

phase('Resolve')

const {
  diff_text: diffText,
  file_list: fileList,
  base_sha: baseSha,
  commit_log: commitLog,
  task_context: taskContext,
  mode = 'headless',
  task_id: taskId,
} = args ?? {}

if (!diffText || !diffText.trim() || diffText.trim() === 'EMPTY_DIFF') {
  log('Diff is empty — returning clean envelope')
  return { residual: [], pre_existing: [], mode, skipped_reason: 'empty diff', stats: { personas_run: 0 } }
}

if (!baseSha) {
  log('ERROR: base_sha required — pass from skill')
  return { residual: [], pre_existing: [], mode, error: 'missing base_sha', stats: { personas_run: 0 } }
}

const changeLines = diffText.split('\n').filter(l => l.startsWith('+') || l.startsWith('-')).length
const fileCount = fileList ? fileList.split('\n').filter(Boolean).length : '?'
log(`Diff: ${changeLines} changed lines, ${fileCount} files`)

// ── COMBINED SETUP AGENT (Steps 1 + 2 in one call) ───────────────────────
// Intent extraction + persona activation. One round-trip instead of 2-3.
// Schema-validated → activation flags are hard booleans, not freeform text.

const intentContext = taskContext
  ? `CAS task context:\n${taskContext}`
  : `Commit messages:\n${commitLog ?? '(no commit log provided)'}`

const setup = await agent(`You are the cas-code-review setup agent. Analyze the diff and decide:
1. A 2-3 line intent summary (Goal + Scope marker + Non-goals if any)
2. Which conditional personas to activate (LLM judgment — read the diff, do not pattern-match paths)
3. Whether the fallow persona should run (JS/TS detection)

## Source of truth for intent
${intentContext}

## File list
${fileList ?? '(not provided)'}

## Diff header (first 1500 chars)
${diffText.slice(0, 1500)}

## Activation rules — LLM-judged, not path pattern matching
Do NOT grep for /auth/ in paths and call it security activation. Read the diff, understand what it does, decide whether the heuristic fires. This is an LLM-judged decision, not path pattern matching.
- activate_security: diff touches auth/session/token boundaries, user input parsing/deserialization, or permission surfaces (MCP tool dispatch, jail/sandbox logic, factory-mode tool restriction)
- activate_adversarial: diff has 50+ non-test changed lines (you have ${changeLines}) AND touches CAS high-stakes modules (close_ops, verify_ops, factory coordination, SQLite stores, hook system, MCP dispatch). Always false if fewer than 20 non-test lines.
- activate_performance: diff touches DB queries, data transforms on large inputs, caching, or async hot paths
- fallow_skip_reason: null if this is a JS/TS repo with JS/TS files in the diff; a short string reason if fallow should skip (e.g. "non-JS/TS repo: no package.json and no JS/TS files in diff")

Return a single JSON object matching this schema exactly. No prose outside the JSON.`,
  {
    label: 'setup',
    phase: 'Resolve',
    schema: SETUP_SCHEMA,
    model: 'sonnet',
  }
)

const intentSummary = setup?.intent_summary ?? '(intent extraction failed)'
const isFallowSkipped = !!setup?.fallow_skip_reason
const fallowRuns = !isFallowSkipped

// Build the active persona list from setup flags + always-on
const toRun = [...ALWAYS_ON_PERSONAS]
if (setup?.activate_security) toRun.push('security')
if (setup?.activate_performance) toRun.push('performance')
if (setup?.activate_adversarial) toRun.push('adversarial')
if (fallowRuns) toRun.push('fallow')

const personasToDispatch = toRun.filter(name => name !== 'fallow')

log(`Intent: ${intentSummary.split('\n')[0]}`)
log(`Active personas: ${toRun.join(', ')}`)
log(`Conditional: security=${setup?.activate_security}, performance=${setup?.activate_performance}, adversarial=${setup?.activate_adversarial}, fallow=${fallowRuns}`)

// ─────────────────────────────────────────────────────────────────────────────
// PHASE 2: PARALLEL PERSONA DISPATCH (Step 3)
// ─────────────────────────────────────────────────────────────────────────────

phase('Review')

const personaResults = await pipeline(
  personasToDispatch,
  (name) => agent(
    buildPersonaPrompt(name, diffText, fileList ?? '', intentSummary, baseSha),
    { label: `review:${name}`, phase: 'Review', schema: REVIEWER_OUTPUT_SCHEMA, model: 'sonnet' }
  )
)

let fallowResult = null
if (fallowRuns) {
  fallowResult = await agent(
    buildPersonaPrompt('fallow', diffText, fileList ?? '', intentSummary, baseSha),
    { label: 'review:fallow', phase: 'Review', schema: REVIEWER_OUTPUT_SCHEMA, model: 'sonnet' }
  )
}

const allOutputs = [...personaResults, fallowResult].filter(Boolean)

// ─────────────────────────────────────────────────────────────────────────────
// PHASE 3: DETERMINISTIC MERGE (Step 4 — pure JS, Phase A validated)
// ─────────────────────────────────────────────────────────────────────────────

phase('Merge')

const { residual, pre_existing } = mergeFindings(allOutputs)

const p0 = residual.filter(f => f.severity === 'P0').length
const p1 = residual.filter(f => f.severity === 'P1').length
const p2 = residual.filter(f => f.severity === 'P2').length
const p3 = residual.filter(f => f.severity === 'P3').length

log(`Merged: ${residual.length} new (P0:${p0}, P1:${p1}, P2:${p2}, P3:${p3}), ${pre_existing.length} pre-existing`)

return {
  residual,
  pre_existing,
  mode,
  intent_summary: intentSummary,
  activation: {
    activated: toRun,
    fallow_skipped: isFallowSkipped,
    fallow_skip_reason: setup?.fallow_skip_reason ?? null,
    personas_run: personasToDispatch.length + (fallowRuns ? 1 : 0),
  },
  stats: {
    total_findings: residual.length + pre_existing.length,
    p0, p1, p2, p3,
    personas_run: personasToDispatch.length + (fallowRuns ? 1 : 0),
    task_id: taskId ?? null,
  },
}
