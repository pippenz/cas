# Codex CLI Changelog Diary — CAS Response Ledger

A living, **newest-first** ledger of OpenAI Codex CLI releases and how CAS responded to
each. Sibling to `claude-code-changelog-diary.md` — CAS supports both harnesses
(`--worker-cli codex` / `--supervisor-cli codex`), so we track Codex drift too.

Codex has no `CHANGELOG.md`; release notes live on the GitHub **releases page**.

## How to update

When a new Codex stable ships:

1. Pull releases: `gh release list --repo openai/codex --limit 15` (stable tags look like
   `rust-v0.138.0`; `-alpha.N` tags are pre-releases — track stables, skim alphas).
2. Read notes: `gh release view rust-v<X.Y.Z> --repo openai/codex --json body --jq '.body'`
3. Verdict each item against CAS, focusing on the **touchpoints** below — that's the whole
   surface a Codex change can break for us. Most items (TUI, plugins, ChatGPT auth, Python
   SDK, Bedrock) are orthogonal → `⏭ n/a`.
4. Add a newest-first entry + index row. File a CAS task only when work is actually required.
5. **Version gap matters:** track the version CAS is validated against vs latest (below).

**Verdict legend:** ✅ no action · 🟢 already covered · 👀 watch (touches a CAS dependency,
verify on upgrade) · 🔧 fix shipped · 🏗 EPIC · ⏭ n/a

## Version status

- **CAS validated against:** Codex CLI **0.128.0** (local install; `crates/cas-pty/src/pty.rs:317`
  comment pins the effort approach to 0.128).
- **Latest stable:** **0.138.0** (2026-06-08). **0.139.0** in alpha.
- **Gap:** ~10 minor versions. The seed entries below are a *triage pass* against the touchpoints,
  not a per-item code audit — upgrade-time re-verification is the trigger for promoting any 👀 to a
  task. (Contrast the Claude Code diary's .166/.162 entries, which were deep-verified for specific
  user questions.)

## CAS ↔ Codex touchpoints (what a release can break)

The load-bearing surface, all in `crates/cas-pty/src/pty.rs::PtyConfig::codex` unless noted:

- **`--yolo`** — approval + sandbox bypass for factory workers (Codex's analogue of Claude's
  `--dangerously-skip-permissions`). If renamed/removed/semantically narrowed, workers can't act.
  Watch any "sandbox / approval / deny-rule enforcement" changelog line.
- **`-c model_reasoning_effort=<e>`** — effort is set via a **TOML `-c` override, not a flag**
  (vocabulary: none/minimal/low/medium/high/xhigh). Mapped from `Effort::as_codex_config()`
  (`crates/cas-mux/src/spec.rs:45`). Any "reasoning effort" changelog line is a 👀.
- **`--config developer_instructions="..."`** — supervisor/worker role instructions are injected
  via this config key. A rename breaks role priming.
- **`--no-alt-screen`** — required for the factory mux to render Codex panes.
- **`--model <m>`** — model selection passthrough.
- **`.codex/config.toml`** — registers the CAS MCP server (exposed to Codex as **`cs`**, e.g.
  `mcp__cs__task`, `mcp__cs__coordination` — note: `cs`, not `cas`). MCP dep bumps (e.g. rmcp)
  are 👀.
- **`.codex/skills/` + `.codex/agents/`** — the Codex mirror synced from `.claude/` by
  `cas integrate` / `cas update`. Codex "skills plumbing / malformed skills field" changes are 👀.
- **`AGENTS.md`** — Codex's workspace-instruction file (its CLAUDE.md analogue). Workers run in
  worktrees, so "AGENTS.md loading / symlinked workspace" changes are 👀.
- **`CAS_AGENT_ROLE` / `CAS_FACTORY_MODE` env** — drive the same hook-local auto-approve + jail
  exemptions as Claude workers (note: Codex has no Claude-style hook system; CAS relies on `--yolo`
  + env, not PreToolUse, on the Codex path).

## Index

| Codex version | Headline | CAS verdict | Pointer |
|---------------|----------|-------------|---------|
| 0.139.0-alpha | (pre-release; not tracked until stable) | — | — |
| 0.138.0 | Effort-order-from-model, skills→extension bridge, AGENTS.md symlink fix, multi-agent v2 catalog | 👀 watch | this doc |
| 0.137.0 | Skills plumbing → dedicated crates, malformed-skills-as-warning, permission env identity, multi-agent v2 | 👀 watch | this doc |
| 0.136.0 | deny-read enforced in approval-bypass paths, rmcp 1.7.0, command-safety hardening | 👀 watch | this doc |

---

## Entries

### 0.138.0 — reasoning-effort order · skills→extension bridge · AGENTS.md symlink fix

Reviewed 2026-06-09 (calm-crane-32 / supervisor). Triage pass vs touchpoints.

- **"Reasoning effort selection is more flexible… model-defined effort levels now flow through in
  the order advertised by the model" (#25623, #26444, #26446).** → 👀 **touchpoint: effort.** CAS
  sets effort via `-c model_reasoning_effort=<e>` and the 0.128 comment notes Codex had no `--effort`
  flag. On the 0.128→0.138 upgrade, verify (a) the `model_reasoning_effort` TOML key still exists and
  (b) our fixed vocabulary (none…xhigh) still validates against model-advertised levels. If 0.138
  added a first-class `--effort` flag, consider switching to it.
- **"Bridge host-loaded skills into the skills extension" (#26172).** → 👀 **touchpoint: `.codex/skills/`.**
  Codex is moving skills into an extension subsystem. Verify our synced `.codex/skills/*.md` still load
  on upgrade.
- **"Workspace instruction loading is more accurate for remote and symlinked workspaces, so the right
  `AGENTS.md` files are picked up" (#26205, #26465).** → 👀 **touchpoint: AGENTS.md.** Factory workers
  run in worktrees; this likely *helps* (more reliable pickup) but verify worker priming still lands.
- **"catalog multi-agent v2 config" (#26254) + multi-agent v2 work.** → 👀 **strategic.** Codex is
  building its own multi-agent orchestration — the same "cede the mechanism, own knowledge + quality"
  fork tracked for Claude Code (Workflow / Agent Teams). Same posture applies: CAS should feed Codex
  multi-agent, not compete with it. No action; flagged for the next strategy pass.
- **Startup resilience: `/usr/bin/bash` support (#26538), OAuth-backed MCP pre-refresh (#26482).** →
  ✅ no action (strictly helpful; the bash one echoes our shell-form vs exec-form lineage).
- **`/app` desktop handoff, local-image paths to model, plugin `--json`, Bazel worktree settings,
  forked-thread titles, TUI streaming whitespace.** → ⏭ n/a (orthogonal to the CAS surface).

### 0.137.0 — skills plumbing → dedicated crates · permission env identity · multi-agent v2

Reviewed 2026-06-09. Triage pass.

- **"Shared prompts, context fragments, and skills plumbing moved into dedicated crates/extension
  paths to reduce `codex-core` coupling" (#25151, #25953, #25959, #26106, #26122, #26167).** → 👀
  **touchpoint: `.codex/skills/`.** Internal refactor of how skills load; watch for format/location
  drift in our synced mirror across the upgrade.
- **"Plugin loading… treats malformed `skills` fields as warnings" (#25782).** → 👀 **touchpoint:
  skills.** If our generated `.codex/skills` frontmatter has a field Codex now scrutinizes, it
  degrades to a warning rather than hard-failing — safer, but verify nothing silently drops.
- **"Permission requests and approvals now carry environment identity" (#25850, #25858, #25862).** →
  👀 **touchpoint: `--yolo`/approval.** CAS workers bypass approvals via `--yolo`; confirm the new
  env-identity carrying doesn't reintroduce a prompt on the bypass path.
- **"Multi-agent v2 keeps runtime choice with each thread… cleaner follow-up and metadata defaults
  for spawned agents" (#25266, #25636, …).** → 👀 strategic (see 0.138 note).
- **"Moved repo review rules and contributor conventions into `AGENTS.md`" (#25682).** → ✅ no action
  (Codex repo's own convention; informs that AGENTS.md is the live instruction surface).
- **F13–F24 keybindings, enterprise credit limits, remote-control pairing, ChatGPT-auth, SQLite
  startup, Python SDK.** → ⏭ n/a.

### 0.136.0 — deny-read enforced in approval-bypass paths · rmcp 1.7.0 · command-safety hardening

Reviewed 2026-06-09. Triage pass.

- **"`deny` read rules stay enforced for safe-command and approval-bypass paths" (#22729, #19880,
  #23943).** → 👀 **touchpoint: `--yolo`.** Most relevant item in the seed: deny-read rules now hold
  even on approval-bypass paths. Verify our `--yolo` workers can still read everything they need (no
  default deny-read that blocks worktree/CAS-root access). Low risk but directly on the bypass path.
- **"Updated MCP dependencies to `rmcp` 1.7.0" (#24763).** → 👀 **touchpoint: MCP (`cs`).** Protocol
  is stable, but a Codex-side MCP client bump is worth a smoke test of `mcp__cs__*` tool calls on
  upgrade.
- **"Command-safety hardening: `/diff` won't run repo Git helpers/hooks; reject browser-origin
  exec-server websocket; no PowerShell parser exec on non-Windows" (#24954, #24946, #24947).** → ✅
  no action (security hardening; doesn't touch our launch surface).
- **"Move memories root setup out of core config" (#24758).** → 👀 minor — Codex has its own
  "memories" concept; confirm no collision with how CAS presents memory via MCP. Likely orthogonal.
- **`/archive` + `codex archive`, OSC 8 TUI links, Windows sandbox elevated setup, Bedrock region
  fallback, image-gen extension.** → ⏭ n/a.

---

## Backlog of opportunities (not required, tracked)

- **Effort flag migration:** if a stable Codex ships a first-class `--effort`, replace the
  `-c model_reasoning_effort=` TOML override (cleaner, version-stable). See 0.138 entry.
- **Multi-agent v2 strategic posture:** decide CAS's stance toward Codex's native multi-agent
  orchestration (mirror of the Claude Code Workflow/Agent-Teams fork). See 0.137/0.138 entries.
- **0.128 → 0.138 upgrade validation:** when bumping the local/factory Codex, run the touchpoint
  checklist above (effort key, skills load, AGENTS.md pickup, `--yolo` deny-read, `cs` MCP smoke).
