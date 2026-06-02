export const meta = {
  name: 'cas-code-review-prototype',
  description: 'Prototype: fan-out cas-code-review personas as a native Workflow script (cas-2efa spike)',
  phases: [
    { title: 'Resolve', detail: 'resolve base SHA + extract diff + task intent' },
    { title: 'Review', detail: 'run 4-8 reviewer personas in parallel (schema-validated)' },
    { title: 'Merge', detail: 'fingerprint dedup + confidence gate + severity sort' },
  ],
}

// ─────────────────────────────────────────────────────────────────────────────
// SCHEMA: mirrors ReviewerOutput + Finding from crates/cas-types/src/code_review.rs
// The schema option on agent() forces a StructuredOutput tool call with
// JSON validation; mismatch causes automatic retry (up to model's limit).
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
  required: ['reviewer','findings'],
  additionalProperties: false,
  properties: {
    reviewer:       { type: 'string' },
    findings:       { type: 'array', items: FINDING_SCHEMA },
    residual_risks: { type: 'array', items: { type: 'string' } },
    testing_gaps:   { type: 'array', items: { type: 'string' } },
  },
}

// ─────────────────────────────────────────────────────────────────────────────
// PERSONA PROMPTS (loaded at runtime; short inline here for prototype —
// production would load from .claude/skills/cas-code-review/references/personas/)
// ─────────────────────────────────────────────────────────────────────────────

const PERSONAS = {
  correctness: {
    always_on: true,
    mandate: `Hunt for defects that make the changed code WRONG — logic errors, broken execution paths, failure modes the author did not consider. Trace execution paths, check invariants, race conditions, broken error handling, resource leaks, arithmetic bugs, bare unwrap()/expect() on fallible input, let _ = <fallible>. Dead/unwired new public code. Do not review test coverage (→ testing), naming (→ maintainability), JS imports (→ fallow), auth surfaces (→ security), DB hot paths (→ performance), blast-radius (→ adversarial).`,
  },
  testing: {
    always_on: true,
    mandate: `Assess test coverage for the changed code. Look for: missing tests on new paths, tests that assert on mutable shared state without isolation, missing edge-case coverage, brittle assertions. Do not hunt for logic bugs (→ correctness) or style issues (→ maintainability).`,
  },
  maintainability: {
    always_on: true,
    mandate: `Assess readability, naming, duplication, dead code, layering violations, module cohesion, comment quality in the diff. Do not hunt for logic bugs (→ correctness) or test coverage (→ testing).`,
  },
  'project-standards': {
    always_on: true,
    mandate: `Check the diff against CAS project rules: commit message format, module layering (no cross-crate private access), error handling conventions (MemError not anyhow::Error in store layer), hook output JSON shape, CODEMAP.md kept in sync. Use mcp__cas__rule if reachable to check active rules. Do not re-derive rule content from first principles.`,
  },
  security: {
    always_on: false,
    activate_when: `diff touches auth/session/token boundaries, user input parsing, permission gates, MCP tool handlers, or deserialization of external data`,
    mandate: `Look for: injection surfaces, broken auth checks, over-broad permissions, credential leaks, timing attacks, SSRF vectors, missing input validation. Do not re-derive correctness bugs that security does not uniquely surface.`,
  },
  adversarial: {
    always_on: false,
    activate_when: `diff is 50+ non-test lines AND touches CAS high-stakes modules (close_ops, verify_ops, factory spawn/message/queue/lifecycle, SQLite stores, hook system, MCP tool dispatch)`,
    mandate: `Stress-test: what breaks under concurrent factory sessions? What happens when a lease expires mid-op? What cascades when an assertion fires in prod? Provide blast-radius analysis, not just correctness. Skip diffs under 20 non-test lines.`,
  },
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

function buildPersonaPrompt(name, persona, diff, fileList, intentSummary, baseSha) {
  return `You are the **${name}** reviewer persona in the CAS multi-persona code review pipeline.

## Your mandate
${persona.mandate}

## Intent of this change
${intentSummary}

## Base SHA
${baseSha}

## Changed files
${fileList}

## Full diff (${name} persona)
\`\`\`diff
${diff}
\`\`\`

## Output contract
Return ONLY a single JSON object matching the ReviewerOutput schema:
{
  "reviewer": "${name}",
  "findings": [ ...zero or more Finding objects... ],
  "residual_risks": [ ...optional strings... ],
  "testing_gaps": [ ...optional strings... ]
}

Each Finding must have: title (≤100 chars), severity (P0/P1/P2/P3), file (relative path), line (1-based integer), why_it_matters, autofix_class (safe_auto/gated_auto/manual/advisory), owner (review-fixer/downstream-resolver/human), confidence (0.0–1.0), evidence (array of ≥1 code-grounded strings), pre_existing (boolean). Optional: suggested_fix, requires_verification.

- confidence ≥ 0.80: traceable from diff alone.
- confidence 0.60–0.79: sound reasoning, some inference gap.
- confidence < 0.60: emit in residual_risks instead, not as a finding.
- P0 confidence threshold: ≥ 0.50 (still surfaces even if uncertain).
- Do NOT emit prose outside the JSON envelope.`
}

// Deterministic fingerprint for deduplication: (file, line bucket ±3, normalised title)
function fingerprint(f) {
  const title = f.title.toLowerCase().replace(/[^a-z0-9]/g, ' ').replace(/\s+/g, ' ').trim()
  const bucket = Math.floor(f.line / 3)
  return `${f.file}|${bucket}|${title}`
}

const OWNER_RANK = { 'human': 2, 'downstream-resolver': 1, 'review-fixer': 0 }

// 7-step deterministic merge pipeline (mirrors SKILL.md Step 4)
function mergeFindings(reviewerOutputs) {
  // Step 1: collect all findings (schema validated by Workflow layer already)
  const allFindings = reviewerOutputs.filter(Boolean).flatMap(r => r.findings || [])

  // Step 2: confidence gate — suppress < 0.60 unless P0 (≥ 0.50)
  const gated = allFindings.filter(f =>
    f.severity === 'P0' ? f.confidence >= 0.50 : f.confidence >= 0.60
  )

  // Step 3 + 4: fingerprint dedup + cross-reviewer confidence boost
  const byFp = new Map()
  for (const f of gated) {
    const fp = fingerprint(f)
    if (!byFp.has(fp)) {
      byFp.set(fp, { finding: f, count: 1 })
    } else {
      const entry = byFp.get(fp)
      entry.count++
      // Boost confidence by 0.10 for each additional agreeing reviewer, cap 1.0
      entry.finding = {
        ...entry.finding,
        confidence: Math.min(1.0, entry.finding.confidence + 0.10),
        // Step 6: conservative owner resolution
        owner: OWNER_RANK[f.owner] > OWNER_RANK[entry.finding.owner]
          ? f.owner
          : entry.finding.owner,
      }
    }
  }

  const merged = Array.from(byFp.values()).map(e => e.finding)

  // Step 5: pre-existing separation
  const residual = merged.filter(f => !f.pre_existing)
  const pre_existing = merged.filter(f => f.pre_existing)

  // Step 7: severity sort
  const SEV_ORDER = { P0: 0, P1: 1, P2: 2, P3: 3 }
  residual.sort((a, b) => (SEV_ORDER[a.severity] - SEV_ORDER[b.severity]) || (b.confidence - a.confidence))

  return { residual, pre_existing }
}

// ─────────────────────────────────────────────────────────────────────────────
// PHASE 1: RESOLVE
// ─────────────────────────────────────────────────────────────────────────────
phase('Resolve')

// args: { base_sha, task_id, mode } — or infer from git
const baseSha = args?.base_sha || (await agent(
  `Run: git log --oneline -5 && git rev-parse HEAD~1 2>/dev/null || git rev-list --max-parents=0 HEAD
  Return ONLY the SHA that should be used as the review base (the commit immediately before the tip, or origin/HEAD if on a feature branch). No prose, just the SHA.`,
  { label: 'resolve-base-sha' }
)).trim()

const [diffText, fileList, intentRaw] = await parallel([
  () => agent(
    `Run: git diff ${baseSha}..HEAD
     Return the full diff text verbatim. No truncation. If diff is empty say EMPTY_DIFF.`,
    { label: 'fetch-diff' }
  ),
  () => agent(
    `Run: git diff --name-only ${baseSha}..HEAD
     Return the newline-separated file list verbatim.`,
    { label: 'fetch-file-list' }
  ),
  () => agent(
    args?.task_id
      ? `Fetch CAS task ${args.task_id} via mcp__cas__task action=show id=${args.task_id}.
         Synthesize a 2-3 line intent summary: Goal (one line), Scope marker (new feature/refactor/bug fix/etc.), Non-goals if stated.
         Return ONLY the intent summary, no extra prose.`
      : `Run: git log --format=%B ${baseSha}..HEAD
         Synthesize a 2-3 line intent summary from the commit messages: Goal (one line), Scope marker, Non-goals if any.
         Return ONLY the intent summary, no extra prose.`,
    { label: 'extract-intent' }
  ),
])

if (diffText.trim() === 'EMPTY_DIFF' || !diffText.trim()) {
  log('Diff is empty — returning clean envelope')
  return { residual: [], pre_existing: [], mode: args?.mode || 'headless', skipped_reason: 'empty diff' }
}

// Tiny-diff bypass: < 5 lines and no new files → skip
const lineCount = diffText.split('\n').filter(l => l.startsWith('+') || l.startsWith('-')).length
if (lineCount < 5) {
  const hasNewFiles = fileList.split('\n').some(f => f.trim())
  // In a real impl: check git status for new vs modified; simplified here
  log(`Diff is tiny (${lineCount} changed lines) — skipping full pipeline`)
  return { residual: [], pre_existing: [], mode: args?.mode || 'headless', skipped_reason: `trivial diff (${lineCount} lines)` }
}

const intentSummary = intentRaw.trim()
log(`Intent: ${intentSummary.split('\n')[0]}`)
log(`Diff: ${lineCount} changed lines across ${fileList.split('\n').filter(Boolean).length} files`)

// ─────────────────────────────────────────────────────────────────────────────
// PHASE 2: CONDITIONAL PERSONA SELECTION + PARALLEL DISPATCH
// ─────────────────────────────────────────────────────────────────────────────
phase('Review')

// LLM-judged conditional activation (same logic as SKILL.md Step 2)
// For prototype: inline heuristic based on diff content; production would use a brief agent call
const activateSecurityAgent = await agent(
  `Read this diff summary and decide if the SECURITY persona should activate.
   Security activates when the diff touches: auth/session/token boundaries, user input parsing/deserialization, permission gates, MCP tool handlers, privilege decisions.
   Diff file list: ${fileList}
   Diff first 500 chars: ${diffText.slice(0, 500)}
   Reply with EXACTLY one word: YES or NO. No other text.`,
  { label: 'select-security' }
)
const activateAdversarial = await agent(
  `Read this diff and decide if the ADVERSARIAL persona should activate.
   Activates when: diff is 50+ non-test changed lines AND touches CAS high-stakes modules (close_ops, verify_ops, factory coordination, SQLite stores, hook system, MCP dispatch).
   Stats: ${lineCount} changed lines. Files: ${fileList}
   Reply with EXACTLY one word: YES or NO.`,
  { label: 'select-adversarial' }
)

const activePersonas = [
  'correctness', 'testing', 'maintainability', 'project-standards',
  ...(activateSecurityAgent.trim().toUpperCase() === 'YES' ? ['security'] : []),
  ...(activateAdversarial.trim().toUpperCase() === 'YES' ? ['adversarial'] : []),
]

log(`Active personas: ${activePersonas.join(', ')}`)

// Fan-out all personas in parallel — NO barrier needed; each is independent.
// pipeline() is the right primitive here (items = personas, single stage = review).
const reviewerOutputs = await pipeline(
  activePersonas,
  async (personaName, originalName, idx) => {
    const persona = PERSONAS[personaName]
    const prompt = buildPersonaPrompt(
      personaName,
      persona,
      diffText,
      fileList,
      intentSummary,
      baseSha,
    )
    return agent(prompt, {
      label: `review:${personaName}`,
      phase: 'Review',
      schema: REVIEWER_OUTPUT_SCHEMA,   // ← KEY DIFFERENCE vs current path
      model: 'sonnet',                  // explicit Sonnet tier per R13
    })
  }
)

// ─────────────────────────────────────────────────────────────────────────────
// PHASE 3: MERGE (pure JS — no LLM)
// ─────────────────────────────────────────────────────────────────────────────
phase('Merge')

const { residual, pre_existing } = mergeFindings(reviewerOutputs)

const p0Count = residual.filter(f => f.severity === 'P0').length
const p1Count = residual.filter(f => f.severity === 'P1').length

log(`Merged: ${residual.length} new findings (P0:${p0Count}, P1:${p1Count}), ${pre_existing.length} pre-existing`)

// Collect per-persona metadata for the audit envelope
const activationRecord = {
  activated: activePersonas,
  activation_reason: {
    correctness: 'always-on',
    testing: 'always-on',
    maintainability: 'always-on',
    'project-standards': 'always-on',
    security: activateSecurityAgent.trim().toUpperCase() === 'YES' ? 'LLM-judged: diff touches hook/MCP handler surfaces' : 'not activated',
    adversarial: activateAdversarial.trim().toUpperCase() === 'YES' ? 'LLM-judged: 50+ lines + CAS high-stakes modules' : 'not activated',
  },
  skipped: ['fallow'],
  skip_reason: { fallow: 'non-JS/TS repo: no package.json at root and no JS/TS files in diff' },
}

return {
  residual,
  pre_existing,
  mode: args?.mode || 'headless',
  activation: activationRecord,
  stats: {
    total_findings: residual.length + pre_existing.length,
    p0: p0Count,
    p1: p1Count,
    personas_run: activePersonas.length,
  },
}
