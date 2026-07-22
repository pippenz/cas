/**
 * merge-findings.js — standalone deterministic merge pipeline for cas-code-review
 *
 * Extracted from .claude/workflows/cas-code-review-prototype.js (cas-e4d4).
 * Implements the 7-step merge defined in cas-code-review SKILL.md §Step 4:
 *
 *   1. Schema validation (pre-assumed: inputs are valid ReviewerOutput objects)
 *   2. Confidence gate — suppress confidence < 0.60 except P0 ≥ 0.50
 *   3. Fingerprint deduplication (file + line-bucket ±3 + normalised title)
 *   4. Cross-reviewer confidence boost — +0.10 per additional agreeing reviewer (cap 1.0)
 *   5. Pre-existing separation — split into residual / pre_existing buckets
 *   6. Conservative owner resolution — human > downstream-resolver > review-fixer
 *   7. Severity-sorted presentation — P0→P1→P2→P3, then confidence desc within tier
 *
 * Exported for use in:
 *   - merge-findings.test.js (unit tests)
 *   - cas-code-review-prototype.js (Workflow script Phase 3)
 *   - Future skill wrapper (Phase B integration)
 *
 * No runtime dependencies. ES module.
 */

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Owner restrictiveness rank. Higher = more restrictive = wins on conflict.
 * Mirrors the conservative-route resolution rule in SKILL.md §Step 4 / R4.
 */
export const OWNER_RANK = Object.freeze({
  'human': 2,
  'downstream-resolver': 1,
  'review-fixer': 0,
})

const FINDING_REQUIRED_FIELDS = Object.freeze([
  'title', 'severity', 'file', 'line', 'why_it_matters', 'autofix_class',
  'owner', 'confidence', 'evidence', 'pre_existing',
])

const FINDING_ALLOWED_FIELDS = new Set([
  ...FINDING_REQUIRED_FIELDS, 'suggested_fix', 'requires_verification',
])

const VALID_SEVERITIES = new Set(['P0', 'P1', 'P2', 'P3'])
const VALID_AUTOFIX_CLASSES = new Set(['safe_auto', 'gated_auto', 'manual', 'advisory'])
const VALID_OWNERS = new Set(['review-fixer', 'downstream-resolver', 'human'])

function findingValidationErrors(finding) {
  if (!finding || typeof finding !== 'object' || Array.isArray(finding)) {
    return ['finding must be an object']
  }

  const errors = []
  for (const field of FINDING_REQUIRED_FIELDS) {
    if (!Object.hasOwn(finding, field)) errors.push(`missing required field: ${field}`)
  }

  if (Object.hasOwn(finding, 'title') &&
      (typeof finding.title !== 'string' || finding.title.length > 100)) {
    errors.push('title must be a string of at most 100 characters')
  }
  if (Object.hasOwn(finding, 'severity') && !VALID_SEVERITIES.has(finding.severity)) {
    errors.push('severity must be one of P0, P1, P2, P3')
  }
  if (Object.hasOwn(finding, 'file') && typeof finding.file !== 'string') {
    errors.push('file must be a string')
  }
  if (Object.hasOwn(finding, 'line') &&
      (!Number.isInteger(finding.line) || finding.line < 1)) {
    errors.push('line must be an integer greater than or equal to 1')
  }
  if (Object.hasOwn(finding, 'why_it_matters') && typeof finding.why_it_matters !== 'string') {
    errors.push('why_it_matters must be a string')
  }
  if (Object.hasOwn(finding, 'autofix_class') &&
      !VALID_AUTOFIX_CLASSES.has(finding.autofix_class)) {
    errors.push('autofix_class must be safe_auto, gated_auto, manual, or advisory')
  }
  if (Object.hasOwn(finding, 'owner') && !VALID_OWNERS.has(finding.owner)) {
    errors.push('owner must be review-fixer, downstream-resolver, or human')
  }
  if (Object.hasOwn(finding, 'confidence') &&
      (typeof finding.confidence !== 'number' || !Number.isFinite(finding.confidence) ||
       finding.confidence < 0 || finding.confidence > 1)) {
    errors.push('confidence must be a number between 0.0 and 1.0')
  }
  if (Object.hasOwn(finding, 'evidence') &&
      (!Array.isArray(finding.evidence) || finding.evidence.length < 1 ||
       finding.evidence.some(item => typeof item !== 'string'))) {
    errors.push('evidence must be a non-empty array of strings')
  }
  if (Object.hasOwn(finding, 'pre_existing') && typeof finding.pre_existing !== 'boolean') {
    errors.push('pre_existing must be a boolean')
  }
  if (Object.hasOwn(finding, 'suggested_fix') && typeof finding.suggested_fix !== 'string') {
    errors.push('suggested_fix must be a string when present')
  }
  if (Object.hasOwn(finding, 'requires_verification') &&
      typeof finding.requires_verification !== 'boolean') {
    errors.push('requires_verification must be a boolean when present')
  }

  for (const field of Object.keys(finding)) {
    if (!FINDING_ALLOWED_FIELDS.has(field)) errors.push(`unexpected field: ${field}`)
  }
  return errors
}

// ─────────────────────────────────────────────────────────────────────────────
// FINGERPRINT
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Compute a stable deduplication key for a Finding.
 *
 * Key components:
 *   - file (exact, case-sensitive)
 *   - line bucket: Math.floor(line / 3)   ← collapses lines within ±2 of each other
 *   - normalised title: lowercase, strip non-alphanumeric, collapse whitespace
 *
 * Rationale for line-bucket ±3: different personas reading the same code
 * often cite slightly different anchor lines for the same logical issue.
 * ±2 tolerance (bucket width 3) prevents phantom duplicates while still
 * matching close-proximity citations of the same bug.
 *
 * @param {object} f - A Finding object with .file, .line, .title fields
 * @returns {string} Stable fingerprint string
 */
export function fingerprint(f) {
  const title = f.title
    .toLowerCase()
    .replace(/[^a-z0-9]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
  const bucket = Math.floor(f.line / 3)
  return `${f.file}|${bucket}|${title}`
}

// ─────────────────────────────────────────────────────────────────────────────
// MERGE PIPELINE
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Merge an array of ReviewerOutput envelopes into a deduplicated, sorted
 * finding set, split into residual (new) and pre_existing buckets.
 *
 * @param {Array<ReviewerOutput|null|undefined>} reviewerOutputs
 *   Each element may be null/undefined (skipped) or a ReviewerOutput with:
 *     - reviewer: string
 *     - findings: Finding[]
 *     - residual_risks?: string[]
 *     - testing_gaps?: string[]
 * @returns {{ residual: Finding[], pre_existing: Finding[], dropped: object[] }}
 */
export function mergeFindings(reviewerOutputs) {
  // Step 1: Collect and validate findings. Workflow schemas should normally
  // enforce this contract, but adapter output must still be lossless if an
  // under-filled finding reaches the deterministic merge.
  const collected = []
  reviewerOutputs.filter(Boolean).forEach((output, index) => {
    const reviewer = typeof output.reviewer === 'string'
      ? output.reviewer
      : `unknown-reviewer-${index}`
    const findings = Array.isArray(output.findings) ? output.findings : []
    for (const finding of findings) collected.push({ reviewer, finding })
  })

  const dropped = []
  const valid = []
  for (const item of collected) {
    const validationErrors = findingValidationErrors(item.finding)
    if (validationErrors.length > 0) {
      dropped.push({
        reviewer: item.reviewer,
        reason: 'schema_validation_failed',
        validation_errors: validationErrors,
        finding: item.finding,
      })
    } else {
      valid.push(item)
    }
  }

  // Step 2: Confidence gate.
  //   P0:  confidence ≥ 0.50 (stakes too high to suppress at 0.60)
  //   P1–P3: confidence ≥ 0.60
  const gated = []
  for (const item of valid) {
    const threshold = item.finding.severity === 'P0' ? 0.50 : 0.60
    if (item.finding.confidence >= threshold) {
      gated.push(item.finding)
    } else {
      dropped.push({
        reviewer: item.reviewer,
        reason: 'confidence_below_threshold',
        threshold,
        finding: item.finding,
      })
    }
  }

  // Steps 3 + 4 + 6: Fingerprint dedup, cross-reviewer boost, owner resolution.
  // All three are processed in a single pass for efficiency.
  /** @type {Map<string, {finding: object, count: number}>} */
  const byFp = new Map()

  for (const f of gated) {
    const fp = fingerprint(f)
    if (!byFp.has(fp)) {
      byFp.set(fp, { finding: { ...f }, count: 1 })
    } else {
      const entry = byFp.get(fp)
      entry.count++

      // Step 4: Cross-reviewer agreement boost — +0.10 per additional agreeing
      // reviewer, capped at 1.0. Applied to the first-seen finding's confidence.
      const boosted = Math.min(1.0, entry.finding.confidence + 0.10)

      // Step 6: Conservative owner resolution — keep the more restrictive owner.
      const currentRank = OWNER_RANK[entry.finding.owner] ?? 0
      const incomingRank = OWNER_RANK[f.owner] ?? 0
      const resolvedOwner = incomingRank > currentRank ? f.owner : entry.finding.owner

      entry.finding = {
        ...entry.finding,
        confidence: boosted,
        owner: resolvedOwner,
      }
    }
  }

  const deduped = Array.from(byFp.values()).map(e => e.finding)

  // Step 5: Pre-existing separation.
  const residual = deduped.filter(f => !f.pre_existing)
  const pre_existing = deduped.filter(f => f.pre_existing)

  // Step 7: Severity-sorted presentation.
  // Primary key: severity (P0=0, P1=1, P2=2, P3=3) ascending.
  // Secondary key: confidence descending (higher confidence surfaces first within tier).
  const SEV_ORDER = { P0: 0, P1: 1, P2: 2, P3: 3 }
  residual.sort((a, b) =>
    (SEV_ORDER[a.severity] - SEV_ORDER[b.severity]) ||
    (b.confidence - a.confidence)
  )

  return { residual, pre_existing, dropped }
}
