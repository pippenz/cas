# v2.27.0 — Default worker spawn → Codex GPT-5.5 (medium)

Channel: #cas-internal
Deploy: Live on production

## User thread

**Top-level**
Live on production — **User**: Your background AI workers now spawn on OpenAI's Codex GPT-5.5 by default — a different engine under the hood, no setup needed.

**Reply (Was → Now)**
- **Was:** New workers came up on Claude Sonnet 5 at the deepest reasoning setting.
- **Now:** New workers come up on Codex GPT-5.5 at a balanced medium reasoning setting — faster turnarounds by default, in every project. Anything you've set yourself is left exactly as-is.

## Dev thread

**Top-level**
Live on production — **Dev**: Stock worker default flipped to the `codex` harness running `gpt-5.5` @ `medium`; harness now has a proper worker-only stock floor, and clearing/resetting `llm.harness` works again.

**Reply (Was → Now)**
- **Was:** A worker with no `[llm.worker]` override resolved to `claude` / `claude-sonnet-5` / `xhigh`. The top-level `harness` was a plain string hard-defaulted to `claude`, so the worker had no harness stock floor — a model-only change would have spawned a worker under the Claude harness trying to run a Codex model string.
- **Now:** `harness` is optional with a 3-tier fallback (role override → top-level → worker stock floor `codex`), mirroring model/effort. Worker default resolves to `codex` / `gpt-5.5` / `medium`; supervisor stays `claude`. Projects with explicit `[llm.worker]` config are unaffected. Also fixed: `cas config reset llm.harness` (and the same latent case for `llm.model` / `llm.reasoning_effort`) no longer hard-errors on the `(default)` sentinel — reset/clear now returns the field to its unset stock-floor state.
