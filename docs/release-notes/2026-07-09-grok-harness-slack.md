# Release — First-class Grok Build harness (2026-07-09, main f4a3e4b)

Channel: #cas-internal (C0B44GUKDK2). Two top-level posts.

---

## Post 1 — User

🚀 **Live on production** — your coding factory can now run on xAI's Grok, right alongside Claude and Codex.

**Was → Now:**
- **Model choice.** Was: your agents ran on Claude or Codex. Now: you can also run them on Grok (Grok 4.5) — pick the right one per job, or mix all three in the same run.
- **Zero setup.** Was: adding a new AI backend meant wiring it up by hand. Now: Grok picks up your existing project instructions, skills, and tools automatically — if you've got Grok installed, it just works.
- **First-turn correctness.** Was: n/a. Now: start a Grok-powered session with `cas grok`, and your agents already know how to use your project's tools correctly from the very first message.

---

## Post 2 — Dev

🚀 **Live on production** — Grok Build is now a first-class harness alongside Claude and Codex.

**Was → Now:**
- **Harness layer.** Was: two supported CLIs (Claude, Codex). Now: a third — a full Grok launch driver with its own `cas__` MCP tool namespace and capability profile (hooks + subagents), routed correctly through every dispatch site. Entry points: `cas grok` (supervisor), `cas default grok` (persist), `cli=grok` (workers).
- **Liveness / transcripts.** Was: liveness and transcript resolution assumed Claude's on-disk layout. Now: harness-aware — a persisted per-agent CLI field routes each agent to the right transcript resolver (Grok's `~/.grok/sessions/…` layout), with activity classification tuned per harness. (This persisted-CLI field is the general foundation harness-aware liveness needed, not just Grok's.)
- **Tool-name aliasing.** Was: hook reminders and skill guidance hardcoded Claude's `mcp__cas__` prefix. Now: aliasing is keyed off the recipient's own harness (three-way), and a complete Grok skill set ships with the correct `cas__` namespace. Fixed several pre-existing Codex-prefix bugs surfaced in the process.
- **Verification.** A real Grok worker was driven end-to-end against the live datastore and confirmed to register and answer a tool call; the integration is proven, not just unit-tested.
