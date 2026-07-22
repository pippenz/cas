// merge-findings.test.js — unit tests for the 7-step mergeFindings() pipeline
// Run with: node --test merge-findings.test.js
// (Node ≥18 built-in test runner; no external deps)
//
// Written test-first (cas-e4d4, test-first posture).
// The module under test (.claude/workflows/merge-findings.js) is extracted
// from the prototype at commit 7464045 / 6280d02 / 5e0082b.

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'
import { mergeFindings, fingerprint, OWNER_RANK } from './merge-findings.js'

// ─────────────────────────────────────────────────────────────────────────────
// FIXTURES
// ─────────────────────────────────────────────────────────────────────────────

function finding(overrides = {}) {
  return {
    title: 'Default finding title',
    severity: 'P2',
    file: 'src/foo.rs',
    line: 42,
    why_it_matters: 'It matters.',
    autofix_class: 'manual',
    owner: 'downstream-resolver',
    confidence: 0.80,
    evidence: ['foo.rs:42 — evidence'],
    pre_existing: false,
    ...overrides,
  }
}

function reviewerOutput(reviewer, findings) {
  return { reviewer, findings }
}

// ─────────────────────────────────────────────────────────────────────────────
// UNIT TESTS — 7 steps
// ─────────────────────────────────────────────────────────────────────────────

describe('mergeFindings()', () => {

  // ── Step 1: empty input ──────────────────────────────────────────────────
  test('empty input returns empty residual, pre_existing, and dropped', () => {
    const { residual, pre_existing, dropped } = mergeFindings([])
    assert.deepEqual(residual, [])
    assert.deepEqual(pre_existing, [])
    assert.deepEqual(dropped, [])
  })

  test('null entries in input are skipped', () => {
    const { residual } = mergeFindings([null, undefined, reviewerOutput('correctness', [])])
    assert.deepEqual(residual, [])
  })

  test('reviewerOutputs with no findings produce empty output', () => {
    const { residual } = mergeFindings([
      reviewerOutput('correctness', []),
      reviewerOutput('testing', []),
    ])
    assert.deepEqual(residual, [])
  })

  test('under-filled finding is surfaced in dropped instead of disappearing', () => {
    const underFilled = {
      title: 'Important independent finding',
      severity: 'P1',
      file: 'src/foo.rs',
      line: 42,
      why_it_matters: 'The wrapper omitted required merge metadata.',
    }

    const { residual, pre_existing, dropped } = mergeFindings([
      reviewerOutput('gpt-5.5:independent', [underFilled]),
    ])

    assert.deepEqual(residual, [])
    assert.deepEqual(pre_existing, [])
    assert.equal(dropped.length, 1)
    assert.equal(dropped[0].reviewer, 'gpt-5.5:independent')
    assert.equal(dropped[0].reason, 'schema_validation_failed')
    assert.deepEqual(dropped[0].finding, underFilled)
    assert.ok(dropped[0].validation_errors.includes('missing required field: confidence'))
    assert.ok(dropped[0].validation_errors.includes('missing required field: evidence'))
  })

  // ── Step 2: confidence gate ──────────────────────────────────────────────
  test('P2/P3 finding below 0.60 confidence is suppressed', () => {
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [finding({ confidence: 0.59 })])
    ])
    assert.equal(residual.length, 0)
  })

  test('P2/P3 finding at exactly 0.60 confidence passes the gate', () => {
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [finding({ confidence: 0.60 })])
    ])
    assert.equal(residual.length, 1)
  })

  test('P0 finding at 0.50 confidence passes the gate', () => {
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [finding({ severity: 'P0', confidence: 0.50 })])
    ])
    assert.equal(residual.length, 1)
  })

  test('P0 finding below 0.50 confidence is suppressed', () => {
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [finding({ severity: 'P0', confidence: 0.49 })])
    ])
    assert.equal(residual.length, 0)
  })

  test('P1 finding at exactly 0.60 passes, below 0.60 suppressed', () => {
    const { residual: pass } = mergeFindings([
      reviewerOutput('correctness', [finding({ severity: 'P1', confidence: 0.60 })])
    ])
    assert.equal(pass.length, 1)

    const { residual: fail } = mergeFindings([
      reviewerOutput('correctness', [finding({ severity: 'P1', confidence: 0.59 })])
    ])
    assert.equal(fail.length, 0)
  })

  test('confidence-gated finding is surfaced in dropped with its threshold', () => {
    const lowConfidence = finding({
      title: 'Factory hook matcher omits AskUserQuestion',
      severity: 'P1',
      confidence: 0.55,
    })
    const { residual, dropped } = mergeFindings([
      reviewerOutput('gpt-5.5:independent', [lowConfidence]),
    ])

    assert.deepEqual(residual, [])
    assert.equal(dropped.length, 1)
    assert.equal(dropped[0].reviewer, 'gpt-5.5:independent')
    assert.equal(dropped[0].reason, 'confidence_below_threshold')
    assert.equal(dropped[0].threshold, 0.60)
    assert.deepEqual(dropped[0].finding, lowConfidence)
  })

  // ── Step 3: fingerprint deduplication ────────────────────────────────────
  test('exact duplicate across two personas deduplicates to one finding', () => {
    const f1 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10, confidence: 0.80 })
    const f2 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10, confidence: 0.75 })
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [f1]),
      reviewerOutput('testing', [f2]),
    ])
    assert.equal(residual.length, 1)
  })

  test('findings at different files are NOT deduped', () => {
    const f1 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10 })
    const f2 = finding({ title: 'Same bug', file: 'src/b.rs', line: 10 })
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [f1]),
      reviewerOutput('testing', [f2]),
    ])
    assert.equal(residual.length, 2)
  })

  test('line bucket ±3 collapses nearby lines to same fingerprint', () => {
    // line 10 and line 11 both floor(n/3) = 3 → same bucket → same fingerprint
    const f1 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10 })
    const f2 = finding({ title: 'Same bug', file: 'src/a.rs', line: 11 })
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [f1]),
      reviewerOutput('testing', [f2]),
    ])
    assert.equal(residual.length, 1)
  })

  test('fingerprint normalization: title case and punctuation are stripped', () => {
    const f1 = finding({ title: 'Unwrap() can panic!', file: 'src/a.rs', line: 10 })
    const f2 = finding({ title: 'unwrap can panic', file: 'src/a.rs', line: 10 })
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [f1]),
      reviewerOutput('testing', [f2]),
    ])
    assert.equal(residual.length, 1, 'normalised titles should match')
  })

  // ── Step 4: cross-reviewer confidence boost ───────────────────────────────
  test('second agreeing reviewer boosts confidence by +0.10', () => {
    const f1 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10, confidence: 0.75 })
    const f2 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10, confidence: 0.70 })
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [f1]),
      reviewerOutput('testing', [f2]),
    ])
    assert.equal(residual.length, 1)
    assert.equal(residual[0].confidence, 0.75 + 0.10, 'confidence boosted by +0.10')
  })

  test('confidence boost is capped at 1.0', () => {
    const f1 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10, confidence: 0.95 })
    const f2 = finding({ title: 'Same bug', file: 'src/a.rs', line: 10, confidence: 0.90 })
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [f1]),
      reviewerOutput('testing', [f2]),
    ])
    assert.equal(residual[0].confidence, 1.0, 'confidence capped at 1.0')
  })

  // ── Step 5: pre-existing separation ──────────────────────────────────────
  test('pre_existing:true findings go to pre_existing bucket', () => {
    const { residual, pre_existing } = mergeFindings([
      reviewerOutput('correctness', [
        finding({ title: 'New bug', pre_existing: false }),
        finding({ title: 'Old bug', pre_existing: true }),
      ])
    ])
    assert.equal(residual.length, 1)
    assert.equal(pre_existing.length, 1)
    assert.equal(residual[0].title, 'New bug')
    assert.equal(pre_existing[0].title, 'Old bug')
  })

  // ── Step 6: conservative owner resolution ────────────────────────────────
  test('owner resolves to human > downstream-resolver > review-fixer', () => {
    // human beats downstream-resolver
    const fRF = finding({ title: 'Same', file: 'a.rs', line: 1, owner: 'review-fixer' })
    const fDR = finding({ title: 'Same', file: 'a.rs', line: 1, owner: 'downstream-resolver' })
    const fHU = finding({ title: 'Same', file: 'a.rs', line: 1, owner: 'human' })

    const { residual: r1 } = mergeFindings([
      reviewerOutput('correctness', [fRF]),
      reviewerOutput('testing', [fDR]),
    ])
    assert.equal(r1[0].owner, 'downstream-resolver', 'downstream-resolver beats review-fixer')

    const { residual: r2 } = mergeFindings([
      reviewerOutput('correctness', [fDR]),
      reviewerOutput('testing', [fHU]),
    ])
    assert.equal(r2[0].owner, 'human', 'human beats downstream-resolver')

    const { residual: r3 } = mergeFindings([
      reviewerOutput('correctness', [fRF]),
      reviewerOutput('testing', [fHU]),
    ])
    assert.equal(r3[0].owner, 'human', 'human beats review-fixer')
  })

  // ── Step 7: severity sort ─────────────────────────────────────────────────
  test('residual findings are sorted P0 > P1 > P2 > P3', () => {
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [
        finding({ title: 'Low', severity: 'P3', confidence: 0.90 }),
        finding({ title: 'Critical', severity: 'P0', confidence: 0.90 }),
        finding({ title: 'Medium', severity: 'P2', confidence: 0.90 }),
        finding({ title: 'High', severity: 'P1', confidence: 0.90 }),
      ])
    ])
    assert.equal(residual[0].title, 'Critical')
    assert.equal(residual[1].title, 'High')
    assert.equal(residual[2].title, 'Medium')
    assert.equal(residual[3].title, 'Low')
  })

  test('within same severity, higher confidence ranks first', () => {
    const { residual } = mergeFindings([
      reviewerOutput('correctness', [
        finding({ title: 'A', severity: 'P1', confidence: 0.70 }),
        finding({ title: 'B', severity: 'P1', confidence: 0.90 }),
        finding({ title: 'C', severity: 'P1', confidence: 0.80 }),
      ])
    ])
    assert.equal(residual[0].title, 'B')
    assert.equal(residual[1].title, 'C')
    assert.equal(residual[2].title, 'A')
  })

})

// ─────────────────────────────────────────────────────────────────────────────
// FINGERPRINT UNIT TESTS
// ─────────────────────────────────────────────────────────────────────────────

describe('fingerprint()', () => {
  test('same file + same line bucket + normalized title → same fingerprint', () => {
    const f1 = finding({ title: 'Unwrap panic!', file: 'src/a.rs', line: 10 })
    const f2 = finding({ title: 'unwrap panic', file: 'src/a.rs', line: 11 })
    assert.equal(fingerprint(f1), fingerprint(f2))
  })

  test('different files → different fingerprint', () => {
    const f1 = finding({ title: 'Same', file: 'src/a.rs', line: 10 })
    const f2 = finding({ title: 'Same', file: 'src/b.rs', line: 10 })
    assert.notEqual(fingerprint(f1), fingerprint(f2))
  })

  test('far-apart lines → different fingerprint', () => {
    const f1 = finding({ title: 'Same', file: 'src/a.rs', line: 10 })   // bucket 3
    const f2 = finding({ title: 'Same', file: 'src/a.rs', line: 100 })  // bucket 33
    assert.notEqual(fingerprint(f1), fingerprint(f2))
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// OWNER_RANK STRUCTURE TEST
// ─────────────────────────────────────────────────────────────────────────────

describe('OWNER_RANK', () => {
  test('human ranks highest (2)', () => {
    assert.equal(OWNER_RANK['human'], 2)
  })
  test('downstream-resolver ranks middle (1)', () => {
    assert.equal(OWNER_RANK['downstream-resolver'], 1)
  })
  test('review-fixer ranks lowest (0)', () => {
    assert.equal(OWNER_RANK['review-fixer'], 0)
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// REAL FIXTURE: cas-6a84 raw persona findings vs known merged result
//
// The Workflow run wf_0a04a6b2-2d2 on 9b81e68..e6f1e84 produced:
//   - 5 persona outputs (adversarial:5, testing:6, project-standards:4,
//     maintainability:6, correctness:5) = 26 raw findings
//   - Merged result: 24 new findings (P1:2, P2:9, P3:13) + 2 pre-existing
//
// This test verifies mergeFindings() applied to those exact raw inputs
// reproduces the same result (determinism gate).
// ─────────────────────────────────────────────────────────────────────────────

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const fixtureFile = path.join(__dirname, 'fixtures', 'cas-e603-raw-persona-findings.json')

let rawFixture = null
try {
  rawFixture = JSON.parse(readFileSync(fixtureFile, 'utf8'))
} catch {
  // fixture file not yet present — skip live-data tests
}

describe('real fixture: cas-e603 (wf_0a04a6b2-2d2)', { skip: rawFixture === null }, () => {

  test('mergeFindings() on cas-e603 raw inputs produces 24 residual + 2 pre-existing', () => {
    const inputs = Object.values(rawFixture)
    const { residual, pre_existing } = mergeFindings(inputs)
    assert.equal(residual.length, 24, `expected 24 residual, got ${residual.length}`)
    assert.equal(pre_existing.length, 2, `expected 2 pre-existing, got ${pre_existing.length}`)
  })

  test('merged output has P0:0, P1:2 — no false P0s, 2 P1s confirmed', () => {
    const inputs = Object.values(rawFixture)
    const { residual } = mergeFindings(inputs)
    const p0 = residual.filter(f => f.severity === 'P0')
    const p1 = residual.filter(f => f.severity === 'P1')
    assert.equal(p0.length, 0, 'no P0 findings expected')
    assert.equal(p1.length, 2, '2 P1 findings expected')
  })

  test('P1 finding: supervisor_guidance ceiling bug is present (conf ≥ 0.95)', () => {
    const inputs = Object.values(rawFixture)
    const { residual } = mergeFindings(inputs)
    const found = residual.find(f =>
      f.severity === 'P1' &&
      f.title.toLowerCase().includes('supervisor_guidance') &&
      f.title.toLowerCase().includes('12')
    )
    assert.ok(found, 'supervisor_guidance P1 finding present')
    assert.ok(found.confidence >= 0.95, `confidence should be ≥ 0.95, got ${found.confidence}`)
  })

  test('26 raw findings split into 24 residual + 2 pre-existing (no fingerprint dedup in this diff)', () => {
    // All 26 raw findings from cas-e603 have unique fingerprints — no cross-persona
    // duplicates occurred. The 26→24 "reduction" is entirely the pre-existing separation
    // (2 findings have pre_existing:true). This verifies the separation step correctly.
    const inputs = Object.values(rawFixture)
    const allRaw = inputs.flatMap(r => r.findings || [])
    assert.equal(allRaw.length, 26, `expected 26 raw findings, got ${allRaw.length}`)
    const { residual, pre_existing } = mergeFindings(inputs)
    assert.equal(residual.length + pre_existing.length, 26,
      `all 26 raw findings should be preserved (residual ${residual.length} + pre-existing ${pre_existing.length})`)
  })

  test('first finding in output is the highest-severity highest-confidence item', () => {
    const inputs = Object.values(rawFixture)
    const { residual } = mergeFindings(inputs)
    assert.ok(residual.length > 0)
    const SEV = { P0: 0, P1: 1, P2: 2, P3: 3 }
    // Sort invariant: no finding later in the list should outrank the first
    for (let i = 1; i < residual.length; i++) {
      const prev = residual[i - 1], curr = residual[i]
      const prevRank = SEV[prev.severity] * 10 - prev.confidence
      const currRank = SEV[curr.severity] * 10 - curr.confidence
      assert.ok(prevRank <= currRank,
        `sort violation at index ${i}: ${prev.severity}/${prev.confidence.toFixed(2)} before ${curr.severity}/${curr.confidence.toFixed(2)}`)
    }
  })

  test('all residual findings pass the confidence gate', () => {
    const inputs = Object.values(rawFixture)
    const { residual } = mergeFindings(inputs)
    for (const f of residual) {
      const threshold = f.severity === 'P0' ? 0.50 : 0.60
      assert.ok(f.confidence >= threshold,
        `${f.severity} finding "${f.title.slice(0,40)}" has confidence ${f.confidence} < threshold ${threshold}`)
    }
  })

})
