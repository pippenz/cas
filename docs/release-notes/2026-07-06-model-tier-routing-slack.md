# Slack draft — model-tier routing for factory jobs (2026-07-06)

Channel: #cas-internal (`C0B44GUKDK2`) — two distinct top-level posts.

---

## Post 1 — User

**Every factory job used to run on the same default model — a one-line doc fix got the same brain (and the same bill) as a hairy concurrency refactor. Now each job is matched to the model and thinking depth its difficulty actually calls for.**

- Easy work (docs, renames, config bumps) runs on a cheap, fast setup instead of burning premium reasoning budget.
- Hard work (cross-cutting refactors, tricky debugging, critical-path changes) gets a stronger model with deeper reasoning by default — no more starving the hard tasks to subsidize the easy ones.
- If a job keeps failing at one level, it automatically moves up to a stronger model instead of retrying the same way and hoping.
- When only easy work is left, expensive capacity winds down rather than idling on your budget.

---

## Post 2 — Dev

**The per-job model knobs (`cli` / `model` / `effort`) have existed in the spawn path for a while — but nothing said when to turn them, so everything ran at the session default. There's now a price/performance tier rubric wired into the planning flow.**

- New `model-selection` reference in the built-in orchestration guide defines four tiers: **light** (codex gpt-5.5, low effort), **standard** (session default floor), **heavy** (claude sonnet, high effort), **frontier** (claude opus, high effort — sparingly, mapped to named jobs).
- Tier is decided at breakdown time from task signals — depth, type, priority, blast radius (module count, shared traits/schemas, unwind/locking code) — and recorded as a `tier:*` label.
- Fleets spawn as a mix: one spawn call per tier (a call's overrides apply to all workers in it); routing follows the labels; two failed attempts escalate the job one tier up; light tails de-escalate.
- Budget note baked in: on metered subscriptions, effort is the dominant cost lever — the rubric prefers switching to sonnet-high over pinning codex at high/xhigh for long heavy runs.
- Guardrail test pins the Claude/Codex doc mirrors byte-identical, both registrations present, the tier vocabulary intact, and the guide body under its 8 KB session-injection cap.
