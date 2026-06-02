// cas-code-review.test.js — structural validation for the production Workflow script
// Run with: node --test cas-code-review.test.js
//
// Written test-first (cas-0f13 + cas-7c64, test-first posture).
// The production script under test: .claude/workflows/cas-code-review.js
//
// Tests validate:
//   1. The meta block has required fields and correct phase titles
//   2. PERSONA_PROMPTS has all 7 canonical reviewers (verbatim from personas/*.md)
//   3. The REVIEWER_OUTPUT_SCHEMA matches the ReviewerOutput contract
//   4. mergeFindings is imported from merge-findings.js (Phase A module)
//   5. ALWAYS_ON_PERSONAS contains exactly the 4 required personas
//   6. CONDITIONAL_PERSONAS contains exactly the 3 conditional personas
//   7. [Phase C] SETUP_SCHEMA exists and validates the combined setup agent output
//   8. [Phase C] Skill-facing args no longer require intent_summary or activated_personas
//      (the Workflow now handles Steps 1-2 internally)

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

// Import the production script's exported symbols.
// The script itself is NOT a standard module (it's a Workflow script).
// We test the exported constants via a thin barrel-export pattern:
// the production script exports its testable parts at the bottom.
// Import testable constants from the constants module.
// (The production Workflow script cas-code-review.js cannot be imported as a
// standard ES module because Workflow scripts use top-level `return` — a
// non-standard extension of the Workflow runtime. Constants are in a
// separate importable module.)
import {
  PERSONA_PROMPTS,
  ALWAYS_ON_PERSONAS,
  CONDITIONAL_PERSONAS,
  REVIEWER_OUTPUT_SCHEMA,
  WORKFLOW_META,
} from './cas-code-review-constants.js'

import { mergeFindings } from './merge-findings.js'

const CANONICAL_ALWAYS_ON = ['correctness', 'testing', 'maintainability', 'project-standards']
const CANONICAL_CONDITIONAL = ['security', 'performance', 'adversarial']
const CANONICAL_ALL = [...CANONICAL_ALWAYS_ON, ...CANONICAL_CONDITIONAL, 'fallow']

// ─────────────────────────────────────────────────────────────────────────────
// META BLOCK
// ─────────────────────────────────────────────────────────────────────────────

describe('WORKFLOW_META', () => {
  test('has required name field', () => {
    assert.equal(typeof WORKFLOW_META.name, 'string')
    assert.ok(WORKFLOW_META.name.length > 0)
    assert.equal(WORKFLOW_META.name, 'cas-code-review')
  })

  test('has required description field', () => {
    assert.equal(typeof WORKFLOW_META.description, 'string')
    assert.ok(WORKFLOW_META.description.length > 0)
  })

  test('phases array covers Resolve, Review, Merge', () => {
    assert.ok(Array.isArray(WORKFLOW_META.phases))
    const titles = WORKFLOW_META.phases.map(p => p.title)
    assert.ok(titles.some(t => t.includes('Resolve') || t.includes('resolve')),
      `phases must include Resolve: ${titles}`)
    assert.ok(titles.some(t => t.includes('Review') || t.includes('review')),
      `phases must include Review: ${titles}`)
    assert.ok(titles.some(t => t.includes('Merge') || t.includes('merge')),
      `phases must include Merge: ${titles}`)
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// PERSONA PROMPTS
// ─────────────────────────────────────────────────────────────────────────────

describe('PERSONA_PROMPTS', () => {
  test('has all 8 canonical personas (7 + fallow)', () => {
    for (const name of CANONICAL_ALL) {
      assert.ok(name in PERSONA_PROMPTS, `Missing persona: ${name}`)
    }
  })

  test('each persona prompt is a non-empty string', () => {
    for (const [name, prompt] of Object.entries(PERSONA_PROMPTS)) {
      assert.equal(typeof prompt, 'string',
        `${name}: prompt must be a string`)
      assert.ok(prompt.length > 100,
        `${name}: prompt is suspiciously short (${prompt.length} chars)`)
    }
  })

  test('correctness prompt references its mandate and output contract', () => {
    const p = PERSONA_PROMPTS.correctness
    assert.ok(p.includes('ReviewerOutput') || p.includes('reviewer'),
      'correctness prompt must reference ReviewerOutput or reviewer field')
    assert.ok(p.includes('correctness'),
      'correctness prompt must self-identify as correctness reviewer')
  })

  test('fallow prompt references fallow audit CLI', () => {
    const p = PERSONA_PROMPTS.fallow
    assert.ok(p.includes('fallow audit'),
      'fallow prompt must reference fallow audit command')
    assert.ok(p.includes('JS/TS') || p.includes('TypeScript') || p.includes('JavaScript'),
      'fallow prompt must reference JS/TS scope')
  })

  test('security persona prompt references auth/input surfaces', () => {
    const p = PERSONA_PROMPTS.security
    assert.ok(p.includes('auth') || p.includes('session') || p.includes('input'),
      'security prompt must reference auth/session/input surfaces')
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// ALWAYS_ON_PERSONAS / CONDITIONAL_PERSONAS
// ─────────────────────────────────────────────────────────────────────────────

describe('ALWAYS_ON_PERSONAS', () => {
  test('contains exactly the 4 required always-on personas', () => {
    assert.deepEqual([...ALWAYS_ON_PERSONAS].sort(), [...CANONICAL_ALWAYS_ON].sort())
  })
})

describe('CONDITIONAL_PERSONAS', () => {
  test('contains exactly the 3 conditional personas', () => {
    assert.deepEqual([...CONDITIONAL_PERSONAS].sort(), [...CANONICAL_CONDITIONAL].sort())
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// REVIEWER_OUTPUT_SCHEMA
// ─────────────────────────────────────────────────────────────────────────────

describe('REVIEWER_OUTPUT_SCHEMA', () => {
  test('is a JSON Schema object', () => {
    assert.equal(REVIEWER_OUTPUT_SCHEMA.type, 'object')
  })

  test('requires reviewer and findings fields', () => {
    assert.ok(REVIEWER_OUTPUT_SCHEMA.required.includes('reviewer'))
    assert.ok(REVIEWER_OUTPUT_SCHEMA.required.includes('findings'))
  })

  test('has additionalProperties: false (strict schema)', () => {
    assert.equal(REVIEWER_OUTPUT_SCHEMA.additionalProperties, false)
  })

  test('findings items have correct severity enum', () => {
    const findingSchema = REVIEWER_OUTPUT_SCHEMA.properties.findings.items
    const severityEnum = findingSchema.properties.severity.enum
    assert.deepEqual(severityEnum.sort(), ['P0', 'P1', 'P2', 'P3'])
  })

  test('findings items have correct owner enum', () => {
    const findingSchema = REVIEWER_OUTPUT_SCHEMA.properties.findings.items
    const ownerEnum = findingSchema.properties.owner.enum
    assert.deepEqual(ownerEnum.sort(), ['downstream-resolver', 'human', 'review-fixer'])
  })

  test('findings items have correct autofix_class enum', () => {
    const findingSchema = REVIEWER_OUTPUT_SCHEMA.properties.findings.items
    const autofixEnum = findingSchema.properties.autofix_class.enum
    assert.deepEqual(autofixEnum.sort(), ['advisory', 'gated_auto', 'manual', 'safe_auto'])
  })

  test('confidence is a number bounded 0.0..1.0', () => {
    const findingSchema = REVIEWER_OUTPUT_SCHEMA.properties.findings.items
    const conf = findingSchema.properties.confidence
    assert.equal(conf.type, 'number')
    assert.equal(conf.minimum, 0.0)
    assert.equal(conf.maximum, 1.0)
  })

  test('evidence requires at least one item', () => {
    const findingSchema = REVIEWER_OUTPUT_SCHEMA.properties.findings.items
    const evid = findingSchema.properties.evidence
    assert.equal(evid.minItems, 1)
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// INTEGRATION: mergeFindings import still works through Phase A module
// ─────────────────────────────────────────────────────────────────────────────

describe('mergeFindings integration (Phase A)', () => {
  test('mergeFindings is a function', () => {
    assert.equal(typeof mergeFindings, 'function')
  })

  test('mergeFindings on empty input returns empty residual', () => {
    const { residual, pre_existing } = mergeFindings([])
    assert.deepEqual(residual, [])
    assert.deepEqual(pre_existing, [])
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// PHASE C: SETUP_SCHEMA (combined Steps 1-2 agent output)
// The single setup agent returns intent + activation decisions in one call,
// halving the Phase 1 round-trips vs the Phase B design.
// ─────────────────────────────────────────────────────────────────────────────

import {
  SETUP_SCHEMA,
} from './cas-code-review-constants.js'

describe('SETUP_SCHEMA (Phase C — combined setup agent)', () => {
  test('is a JSON Schema object', () => {
    assert.equal(SETUP_SCHEMA.type, 'object')
  })

  test('requires intent_summary field', () => {
    assert.ok(SETUP_SCHEMA.required.includes('intent_summary'),
      'SETUP_SCHEMA must require intent_summary')
  })

  test('requires activate_security field', () => {
    assert.ok(SETUP_SCHEMA.required.includes('activate_security'),
      'SETUP_SCHEMA must require activate_security')
  })

  test('requires activate_adversarial field', () => {
    assert.ok(SETUP_SCHEMA.required.includes('activate_adversarial'),
      'SETUP_SCHEMA must require activate_adversarial')
  })

  test('requires fallow_skip_reason field', () => {
    assert.ok(SETUP_SCHEMA.required.includes('fallow_skip_reason'),
      'SETUP_SCHEMA must require fallow_skip_reason')
  })

  test('activate_security is boolean', () => {
    const prop = SETUP_SCHEMA.properties.activate_security
    assert.equal(prop.type, 'boolean',
      'activate_security must be boolean for deterministic activation')
  })

  test('activate_adversarial is boolean', () => {
    const prop = SETUP_SCHEMA.properties.activate_adversarial
    assert.equal(prop.type, 'boolean',
      'activate_adversarial must be boolean for deterministic activation')
  })

  test('activate_performance is boolean (optional conditional)', () => {
    const prop = SETUP_SCHEMA.properties.activate_performance
    assert.ok(prop, 'activate_performance property must exist')
    assert.equal(prop.type, 'boolean')
  })

  test('fallow_skip_reason allows null (fallow should run)', () => {
    const prop = SETUP_SCHEMA.properties.fallow_skip_reason
    const types = Array.isArray(prop.type) ? prop.type : [prop.type]
    assert.ok(types.includes('null') || prop.nullable === true,
      'fallow_skip_reason must allow null to signal fallow should run')
  })

  test('intent_summary is a string', () => {
    const prop = SETUP_SCHEMA.properties.intent_summary
    assert.equal(prop.type, 'string')
  })

  test('has additionalProperties: false for strict validation', () => {
    assert.equal(SETUP_SCHEMA.additionalProperties, false)
  })
})
