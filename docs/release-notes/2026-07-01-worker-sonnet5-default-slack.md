# v2.26.0 — Default worker model → Claude Sonnet 5 (xhigh)

Channel: #cas-internal
Deploy: Live on production

## User thread

**Top-level**
Live on production — **User**: Your background AI workers just got a brain upgrade — they now run on Claude Sonnet 5 with deeper reasoning by default, no setup needed.

**Reply (Was → Now)**
- **Was:** Workers used the previous-generation Sonnet model at standard thinking depth.
- **Now:** Workers default to the brand-new Claude Sonnet 5 with a higher reasoning depth — sharper, more autonomous task work automatically, in every project.

## Dev thread

**Top-level**
Live on production — **Dev**: Stock worker default bumped to `claude-sonnet-5` @ `xhigh`; `xhigh` and `max` effort tiers unlocked in config.

**Reply (Was → Now)**
- **Was:** Fallback for a worker with no `[llm.worker]` override resolved to `claude-sonnet-4-6` @ `high`; `reasoning_effort` config accepted only `low`/`medium`/`high`.
- **Now:** Fallback resolves to `claude-sonnet-5` @ `xhigh`; `llm.reasoning_effort`, `llm.supervisor.*`, and `llm.worker.*` now also accept `xhigh` and `max`. Projects with an explicit `[llm.worker]` block are unaffected.
