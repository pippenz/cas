# 2026-07-21 — Supervisor model-routing rubric (Codex-first) — #cas-internal posts

## Post 1 — User

Live on `main` — **User**

Your factory used to route work Grok-first, so quality and cost swung with one vendor's credits and health. Now every job lands on the strongest default lane at the right effort for its size — automatically.

- **Was:** most tasks went to Grok by default, with Claude held back as an escalation hatch. When Grok credits or auth wobbled, routing wobbled with them.
- **Now:** every task is sized (light / standard / heavy / frontier) and lands on the GPT-5.6 Sol lane at matching effort — chores stay cheap, hard problems get real reasoning.
- Taste-sensitive output — docs, naming, release notes, anything user-facing — gets its own judgment lane instead of being treated as "small diff, cheap model."
- Claude Opus is reserved for the truly exceptional calls: architecture, safety, rescuing a stuck problem, or an independent second opinion.
- Grok still helps out as extra capacity, but only while it's healthy — if it isn't, work silently takes the equivalent default lane instead of failing.

## Post 2 — Dev

Live on `main` — **Dev**

Supervisor model routing is now Codex-first and two-stage: tier the task, then pick the lane.

- **Was:** Grok-first matrix (Composer Fast for light, grok-4.5 medium/high for standard/heavy) with Claude Sonnet as the taste/heavy escalation and Codex as quota backup.
- **Now:** all four tiers resolve to `cli=codex model=gpt-5.6-sol` at `effort=low|medium|high`; effort is the primary escalation lever before any lane change.
- Routine taste / public-surface / general-judgment work routes to `gpt-5.6-sol effort=medium` — Sonnet is no longer a normal worker lane.
- Claude Opus (`effort=high`) is exceptional-only: architecture, safety-critical changes, rescue, independent challenge.
- Grok is a health-gated capacity overlay (Composer Fast at `effort=low`, grok-4.5 medium/high) with a same-tier Codex fallback; the health check (credits, auth, `grok models` responding) gates routing to it.
- Applied identically across all three harness builtin twins; enforced by tests: normalized tri-body consistency, explicit `cli`/`model`/`effort` on every spawn recipe, a bare-`gpt-5.6`-slug guard (exact slug is `gpt-5.6-sol`), and an 8000-byte body soft cap under the 8192B SessionStart ceiling.
- Evidence rule added: routing guidance changes require cross-vendor evidence (leaderboards + live harness check), never a single failed spawn.
