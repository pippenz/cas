# Claude Code Changelog Diary — CAS Response Ledger

A living, **newest-first** ledger of Claude Code releases and how CAS responded to
each. The question this doc answers in one place: *"Does this Claude Code change
require a CAS change, and if so, where's the proof?"*

This is the index layer. Deep per-EPIC working notes live in their own dated files
(e.g. `2026-06-02-cc160-hook-surface.md`); this diary links out to them.

Sibling ledger for the other supported harness: `codex-changelog-diary.md`.

## How to update

When a new Claude Code version ships:

1. Pull the changelog: `curl -s https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md`
2. For each user-facing item, decide a verdict against CAS (table legend below).
   Most items are `✅ no action` — CAS rides the harness, it doesn't fork it.
3. Add a new `## <version>` section at the TOP of the Entries list with the relevant
   items + verdict + reasoning. Only record items with a real CAS angle; skip pure
   harness UI/perf fixes unless they touch a surface CAS depends on.
4. Update the Index table.
5. If an item triggers actual work, file a CAS task/EPIC and link it here. If it's a
   "reviewed, no change needed" conclusion worth not re-litigating, also drop a
   `reference`-type CAS memory.

**Verdict legend**

| Verdict | Meaning |
|---------|---------|
| ✅ no action | Reviewed; CAS unaffected or already insulated |
| 🟢 already covered | CAS already does the equivalent on its own surface |
| 👀 opportunity | Not required, but a cleaner path CAS could adopt — tracked, not urgent |
| 🔧 fix shipped | Required a CAS change; landed (link commit/version) |
| 🏗 EPIC | Large enough to warrant a factory EPIC (link epic note) |
| ⏭ n/a | Internal / no user-facing surface |

## Index

| CC version | Headline | CAS verdict | Pointer |
|------------|----------|-------------|---------|
| 2.1.170 | **Claude Fable 5** (Mythos-class model) GA + VS Code / inherited-env transcript-save fix | 👀 evaluate | this doc |
| 2.1.169 | **`--safe-mode`**, `/cd`, **`disableBundledSkills`**, `agents --json` state/id, managed-MCP enforcement fixes | ✅ no action / 👀 noted | this doc |
| 2.1.168 | Bug-fix rollup | ⏭ n/a | — |
| 2.1.167 | Bug-fix rollup | ⏭ n/a | — |
| 2.1.166 | `fallbackModel`, deny-rule globs, **SendMessage authority hardening** | ✅ no action | this doc |
| 2.1.165 | Bug-fix rollup | ⏭ n/a | — |
| 2.1.163 | Managed version pinning, `/plugin list`, **Stop/SubagentStop `additionalContext`** | 👀 opportunity | this doc |
| 2.1.162 | `claude agents` polish, perm-rule correctness, **Ctrl+V image paste** | 🟢 already covered | this doc |
| 2.1.161 | Telemetry labels, MCP secret redaction, parallel-tool isolation | ✅ no action | this doc |
| 2.1.160 | Sensitive-file write prompts, **"workflow"→"ultracode" rename** | ✅ no action | this doc |
| 2.1.159 | Internal infra | ⏭ n/a | — |
| 2.1.152–160 | Hook surface (reloadSkills, sessionTitle, disallowed-tools, MessageDisplay) | 🏗 EPIC → shipped v2.18.0 | [cc160 note](2026-06-02-cc160-hook-surface.md) |

---

## Entries

### 2.1.170 — Claude Fable 5 (Mythos-class model) GA · VS Code transcript-save fix

Reviewed 2026-06-10 (calm-crane-32 / supervisor).

**Verification note:** the "Claude Fable 5 / Mythos-class" headline tripped every skepticism wire
(naming doesn't match Opus/Sonnet/Haiku; hype phrasing; post-dates the Jan-2026 assistant cutoff),
so it was verified before recording. Multiple independent reputable sources — Anthropic news, CNBC,
NBC, TechCrunch, MacRumors, AWS/Amazon — corroborate a real launch on **2026-06-09**. (A WebFetch
summary labeled the page "fictional," but that is an April-2024-cutoff artifact of the summarizer
model, not a refutation — discounted.) **Conclusion: real model, genuinely launched.**

- **Claude Fable 5 — new top-tier model, GA 2026-06-09.** Public, safety-gated member of the
  "Mythos" class; positioned ABOVE Opus 4.8 (its safeguards fall back to Opus 4.8 on ~5% of sessions
  and block cyber/bio/chem topics). Pricing ~$10/Mtok in, ~$50/Mtok out. Heavy SWE claims
  (codebase-wide migrations in a day). → 👀 **opportunity, not yet actionable.** Relevant to CAS
  worker/supervisor model selection (`STOCK_WORKER_MODEL` in `cas-cli/src/config/settings.rs`;
  `--model` passthrough for both claude + codex paths). Blockers before wiring anything:
  1. **Need the exact API model ID** — the announcement doesn't publish it; can't set a config
     default without the literal id string.
  2. **Cost** — ~5–10× current default; at most a supervisor / hard-task option, never a stock
     worker default.
  3. **Safeguard fallback** — CAS is sometimes used for *authorized* security testing; Fable 5
     silently downgrades cyber/bio/chem prompts to Opus 4.8, so a security-focused worker may not
     actually run on Fable 5. Must be called out in any model-selection guidance.
  4. **Subscription window** — included in Pro/Max/Team/Enterprise only until **2026-06-22**, then
     usage-credits. Affects cost planning if adopted.
- **"Fixed sessions not saving transcripts (and not appearing in `--resume`) when launched from the
  VS Code integrated terminal or any shell that inherited Claude Code environment variables."** →
  👀 **watch — potential factory impact, highest-priority item in this release for us.** CAS spawns
  workers via PTY with INHERITED + augmented Claude Code env (`CAS_*`,
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`, etc. — `crates/cas-pty/src/pty.rs::PtyConfig::claude`).
  If "inherited Claude Code env vars" was the trigger, factory worker sessions on ≤2.1.169 may have
  silently failed to save transcripts — which would undercut session-log mining + blame attribution
  (the per-session JSONL we rely on; see `reference_claude_session_log_paths`,
  `feedback_cross_project_log_mining`). **Check:** do worker session JSONLs exist for recent ≤.169
  factory runs? If there are gaps that correlate with env-inherited spawns, 2.1.170 is the fix and
  the action is just the version bump. Candidate verify task.

### 2.1.169 — `--safe-mode` · `disableBundledSkills` · `/cd` · `agents --json` state/id

Reviewed 2026-06-09 (calm-crane-32 / supervisor). No item requires a CAS change; three are
worth knowing about.

- **`--safe-mode` flag + `CLAUDE_CODE_SAFE_MODE`** — boots Claude Code with ALL customizations
  disabled (CLAUDE.md, plugins, skills, hooks, MCP servers) for troubleshooting. → ✅ no action,
  **👀 useful CAS debugging lever.** When a CAS hook/skill/MCP misbehaves, `--safe-mode`
  cleanly isolates "is it CAS or the harness" in one flag — no manual settings surgery. Worth
  folding into a CAS troubleshooting/onboarding doc as the first triage step. Note: it disables
  the whole CAS surface, so factory mode won't run under it — it's a diagnosis tool, not a run mode.
- **`disableBundledSkills` setting + `CLAUDE_CODE_DISABLE_BUNDLED_SKILLS`** — hides Claude Code's
  *bundled* skills/workflows/built-in slash commands from the model. → ✅ no action, **👀 namespace
  hygiene option.** CAS ships its own skill set; a user drowning in bundled + CAS skills could set
  this to reduce menu clutter and slash-command collisions. Does NOT affect CAS skills (those are
  project/user skills, not bundled). Optional ops choice, not a code change.
- **`claude agents --json` gains `--all`, `id`, `state`; now includes blocked + just-dispatched
  sessions.** → ✅ no action. Re-confirms the standing finding (memory `project_claude_agents_json_session_scoped`,
  `project_cc_2_1_145_150_compat_results`): `agents --json` tracks *background sessions*, not the
  factory's tmux workers, so the new `state`/`id` fields still don't give CAS factory monitoring
  anything. Logged here so the next person doesn't re-test it expecting tmux visibility.
- **`/cd` (move cwd without breaking prompt cache); managed-MCP policy enforcement fixes (reconnect,
  IDE configs, `--mcp-config`, cold start); background agents now honor project `env` (e.g.
  `ANTHROPIC_MODEL`); untrusted-settings OTEL client-cert path now needs trust; `/workflows` opens
  mid-turn; native `TaskCreate` auto-repairs malformed input.** → ✅ no action. All host-side; CAS
  factory uses tmux workers (not background agents) and its own `mcp__cas__task`, so the
  background-agent + native-TaskCreate items don't touch CAS. Stricter MCP enforcement is strictly
  safer for the CAS MCP server registration.

### 2.1.166 — SendMessage authority hardening · `fallbackModel` · deny-rule globs

Reviewed 2026-06-08 (calm-crane-32 / supervisor).

- **Cross-session `SendMessage` no longer carries user authority** — receivers refuse
  relayed permission requests; auto mode blocks them. → **✅ no action.** Two
  independent reasons CAS is off this vector:
  1. In factory mode CAS *intercepts* native `SendMessage` in PreToolUse and reroutes
     it onto the CAS prompt queue, returning `deny` so the native relay never executes
     (`auto_route_send_message`, `cas-cli/src/hooks/handlers/handlers_events/pre_tool.rs:169`,
     impl `:1044`). The native relay subsystem is effectively dead code in factory mode.
  2. CAS grants worker permissions **hook-locally** via its own PreToolUse handler keyed
     on `CAS_AGENT_ROLE` (`FACTORY_AUTO_APPROVE_TOOLS` → `"allow"`, pre_tool.rs:113-119,
     :874, :1027) — never via a relayed cross-session message. That hook-local model is
     exactly what the hardening leaves intact. Informational coordination messages are
     unaffected (the change only restricts permission-request relays).
- **`fallbackModel` setting (up to 3 fallbacks); `--fallback-model` now interactive too.**
  → ✅ no action. CAS doesn't pin the harness model selection; workers inherit it. Could
  *optionally* document a recommended fallback chain for factory hosts, but not required.
- **Deny rules support glob in tool-name position (`"*"` denies all).** → ✅ no action.
  CAS enforces tool gating through its own PreToolUse hook + `disallowed-tools` skill
  frontmatter (cas-5be8), not through user deny rules. Glob deny rules are a host-config
  nicety orthogonal to CAS's enforcement.

### 2.1.163 — Stop/SubagentStop `additionalContext` · managed version pinning · `/plugin list`

Reviewed 2026-06-08.

- **Stop and SubagentStop hooks can return `hookSpecificOutput.additionalContext`** to
  feed Claude and keep the turn going without being flagged a hook error. → **👀
  opportunity.** This is a cleaner channel than what `session-learn`'s Stop-hook auto-trigger
  uses today. Worth a spike: route session-learn / supervisor guidance through
  `additionalContext` instead of the current Stop-hook output path. Not urgent — current
  path works — but flagged so we evaluate before the next hook-surface EPIC. No task filed
  yet.
- **`requiredMinimumVersion` / `requiredMaximumVersion` managed settings.** → ✅ no action.
  Could be useful to pin factory hosts to a known-good CC range, but that's an ops choice,
  not a code change. Note for onboarding docs if we ever standardize a CC floor.
- **`/plugin list`, skill `\$` escape, `CLAUDE_CODE_SESSION_ID` to stdio MCP on resume.**
  → ✅ no action.

### 2.1.162 — Ctrl+V image paste fix · permission-rule correctness · MCP timeout fix

Reviewed 2026-06-08.

- **Fixed `claude agents` Ctrl+V image paste doing nothing in the dispatch/reply boxes.**
  → **🟢 already covered (different surface).** That fix is in Claude Code's *own* `claude
  agents` UI, which CAS does not use — CAS has its own factory TUI
  (`cas-cli/src/ui/factory/client.rs`). CAS already handles image input there via bracketed
  paste: `Event::Paste(text)` → `contains_dropped_image_path()` → emits a `drop_image;…`
  control command (client.rs:202-214), with `file://` URI decode + tests (:464-472).
  **Precision:** CAS handles dropped image *file paths* (drag-drop / pasted path text,
  incl. `file://`), NOT binary-clipboard Ctrl+V of an actual image with no path — which is
  what 2.1.162 specifically addressed. In practice terminals deliver drag-dropped images as
  paths through bracketed paste, so CAS covers the realistic case. True binary-clipboard
  paste in the factory TUI would be net-new and is not currently justified.
- **WebFetch rules now apply to preapproved domains; Windows backslash/case path matching;
  Read deny rules hide files from Glob/Grep; sub-1000ms MCP `timeout` no longer floored to a
  1s watchdog; LSP `workspaceSymbol` returns results.** → ✅ no action (host-side correctness;
  CAS unaffected).

### 2.1.161 — MCP secret redaction · parallel-tool isolation · telemetry labels

Reviewed 2026-06-08.

- **`claude mcp` list/get/add no longer print secrets** (`${VAR}` not expanded; credential
  headers + URL secrets redacted). → ✅ no action; strictly safer for anyone inspecting the
  CAS MCP server registration.
- **A failed Bash in a parallel batch no longer cancels siblings** — each tool returns its
  own result. → ✅ no action; mildly beneficial for factory workers issuing parallel calls.
- **`isolation:"worktree"` Workflow agents in background sessions can now edit their own
  worktree.** → ✅ no action, but relevant to the #6 Workflow-migration direction (see cc160
  note) — removes a friction point if CAS skills author worktree-isolated Workflow agents.

### 2.1.160 — sensitive-file write prompts · "workflow" → "ultracode" rename

Reviewed 2026-06-08. (Note: the deep 2.1.152–160 hook-surface work is its own EPIC — see
[cc160 note](2026-06-02-cc160-hook-surface.md), shipped v2.18.0. Items below are the
remaining .160 changelog lines with a CAS angle.)

- **`acceptEdits` now prompts before writing exec-granting config; shell-startup-file write
  prompts.** → ✅ no action — already characterized in EPIC cas-2f29 task **cas-f97d**:
  factory workers spawn with `--dangerously-skip-permissions` (`bypassPermissions`), which
  short-circuits the .160 sensitive-file check. Non-factory acceptEdits users will see the
  intended Claude Code prompt; that's harness hardening, not a CAS bug.
- **Dynamic-workflow trigger keyword renamed `workflow` → `ultracode`.** → ✅ no action, but
  **terminology watch:** this renames the *trigger keyword* that fires a dynamic workflow,
  NOT the `Workflow` tool itself (still `Workflow`). CAS skill/docs references to the
  `Workflow` tool are unaffected. If any CAS prose tells users to "say workflow to trigger…",
  that guidance is now stale — none found at review time.
- **Removed `CLAUDE_CODE_OPUS_4_6_FAST_MODE_OVERRIDE` (now a no-op).** → ✅ no action; CAS
  doesn't set it.

---

## Backlog of opportunities (not required, tracked)

- **session-learn / guidance via Stop-hook `additionalContext`** (from 2.1.163) — evaluate
  before next hook-surface EPIC. See 2.1.163 entry.
- **Factory CC version floor** via `requiredMinimumVersion` (from 2.1.163) — ops/onboarding
  decision, not code.
