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
 * @returns {{ residual: Finding[], pre_existing: Finding[] }}
 */
export function mergeFindings(reviewerOutputs) {
  // Step 1: Collect all findings from valid (non-null) envelopes.
  // Schema validation is pre-assumed (done by Workflow schema option).
  const allFindings = reviewerOutputs
    .filter(Boolean)
    .flatMap(r => r.findings || [])

  // Step 2: Confidence gate.
  //   P0:  confidence ≥ 0.50 (stakes too high to suppress at 0.60)
  //   P1–P3: confidence ≥ 0.60
  const gated = allFindings.filter(f =>
    f.severity === 'P0' ? f.confidence >= 0.50 : f.confidence >= 0.60
  )

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

  return { residual, pre_existing }
}
