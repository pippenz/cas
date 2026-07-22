# Grok Build Changelog Diary — CAS Response Ledger

A living, **newest-first** ledger of xAI Grok Build CLI releases and how CAS
responded to each. Sibling to `claude-code-changelog-diary.md` and
`codex-changelog-diary.md` — CAS supports three harnesses (`cli=claude` /
`cli=codex` / `cli=grok`, EPIC cas-8888), so we track Grok drift too.

Grok ships a local changelog at `~/.grok/CHANGELOG.md` (and a flat item list at
`~/.grok/CHANGELOG.json` for the current install). Unlike Claude Code (upstream
GitHub CHANGELOG) or Codex (GitHub releases), the installable history on a host
may only cover the versions present on that binary's changelog surface.

## How to update

When a new Grok Build version ships (or after `grok` upgrades on the host):

1. Confirm the binary: `grok --version` (example: `grok 0.2.101 (… ) [stable]`).
2. Read the local changelog: `~/.grok/CHANGELOG.md` (version sections) and, if
   useful, `~/.grok/CHANGELOG.json` (flat feature/fix list for the current install).
3. Verdict each user-facing item against the **CAS ↔ Grok touchpoints** below.
   Most TUI polish is `⏭ n/a`. Prefer items that touch permissions, session IDs,
   rules/system prompt, MCP discovery, env inheritance, transcripts, or hooks.
4. Add a newest-first entry + index row. File a CAS task only when work is required.
5. **Version gap matters:** keep **validated pin** (pty.rs comment) vs **locally
   installed** vs **latest in changelog** honest. Do not invent older releases —
   if the local changelog only has N versions, seed those N and mark the **seed floor**.

**Verdict legend:** ✅ no action · 🟢 already covered · 👀 watch (touches a CAS
dependency, verify on upgrade) · 🔧 fix shipped · 🏗 EPIC · ⏭ n/a

## Version status

- **CAS validated against:** Grok Build **0.2.93** (comment on
  `crates/cas-pty/src/pty.rs` `PtyConfig::grok`, verified live 2026-07-09 via
  `grok --help` / `grok inspect` / `grok mcp doctor`).
- **Locally installed:** **0.2.101** (`grok 0.2.101`, checked 2026-07-14).
- **Latest in local changelog:** **0.2.101** (2026-07-13). The installable
  `~/.grok/CHANGELOG.md` currently documents only **0.2.100** and **0.2.101** —
  that is the **seed floor** for this diary (do not invent pre-0.2.100 history).
- **Gap:** ~8 patch versions between the validated pin (0.2.93) and installed/latest
  (0.2.101). Entries below are a *triage pass* against touchpoints for the two
  changelog-visible releases, not a re-run of the 0.2.93 live verification.
  Upgrade-time re-verification of the full touchpoint checklist is the trigger for
  promoting any 👀 to a task.

## CAS ↔ Grok touchpoints (what a release can break)

The load-bearing surface is `crates/cas-pty/src/pty.rs::PtyConfig::grok` (approx.
lines 423–580; instructions constants near top of file). Ground truth is that
code + its "Verified against … 0.2.93" block — re-read it on upgrade rather than
trusting this diary alone.

### CLI flags (spawn args)

- **`--permission-mode bypassPermissions`** — factory workers skip interactive
  approval (Grok's analogue of Claude's bypass / Codex's `--yolo`). Any rename,
  removal, or semantic narrowing of `bypassPermissions` breaks unattended workers.
- **`--session-id <uuid>`** — fresh UUID per *new* conversation (anti-overwrite
  model, same family as Claude; not Codex's `codex-<name>-<uuid>` prefix). Phase 4
  transcript resolution keys on this exact value. Doc comment also notes short form
  `-s/--session-id` on the 0.2.93 binary.
- **`-m` / `--model <MODEL>`** — optional model pin when the factory requests one.
- **`--reasoning-effort <EFFORT>`** (alias `--effort` on the verified binary) —
  vocabulary minimal/low/medium/high/xhigh via `Effort::as_claude_arg()` (no
  separate `as_grok_arg`). Any "reasoning effort" changelog line is a 👀.
- **`--cwd <path>`** — worktree/working directory for the worker process.
- **`--rules <text>`** — **"Extra rules to append to the system prompt."** This is
  the load-bearing context path for factory role priming. CAS injects
  `GROK_WORKER_INSTRUCTIONS` or `GROK_SUPERVISOR_INSTRUCTIONS` (same file). Grok's
  **SessionStart hook fires but its stdout is ignored** (delta #2) — do not assume
  Claude-style SessionStart `additionalContext` delivery on Grok.

### MCP discovery (no per-spawn `-c` override)

- Grok has **no** ephemeral per-launch MCP config flag analogous to Codex
  `-c mcp_servers.*`. Servers come from persistent discovery:
  project `.mcp.json`, `~/.claude.json`, and/or `~/.grok/config.toml`
  (`grok mcp add` writes the latter).
- Tools are namespaced **`cas__*`** on Grok (e.g. `cas__task`, `cas__coordination`)
  — **not** `mcp__cas__*` and **not** Codex's `mcp__cs__*` / `cs` prefix. Worker and
  supervisor `--rules` text must keep that prefix honest.
- Identity for `cas serve` rides ordinary **child-process env inheritance** from the
  grok process (same pattern as Claude; no `mcp_servers.*.env` TOML block).

### Process env (set on the grok child)

At minimum, `PtyConfig::grok` sets:

- `CAS_AGENT_NAME`, `CAS_AGENT_ROLE`
- `CAS_FACTORY_MODE=1` (verification-jail / factory exemptions)
- `CAS_SESSION_ID` — same UUID as `--session-id`; load-bearing identity when hooks
  cannot deliver SessionStart context
- `CAS_CLONE_PATH`, optional `CAS_ROOT`, optional `CAS_SUPERVISOR_NAME`
- `CAS_FACTORY_WORKER_CLI=grok` — **unconditional** on a grok process (cas-921f);
  required so harness-aware liveness looks under Grok paths, not Claude's
- Plus shared factory metadata / cargo / zig env helpers used by other CLIs

### Transcripts / liveness

- Grok session transcripts live under **`~/.grok/sessions/*`**, not
  `~/.claude/projects/*`. If `CAS_FACTORY_WORKER_CLI` is wrong, is-wedged/liveness
  globs the Claude tree and always resolves `None` for a real grok worker.

### Hooks posture (contrast Claude)

- Claude path: SessionStart / PreToolUse are load-bearing.
- Grok path: SessionStart stdout ignored → **`--rules` + env** carry factory
  identity and role text. Changelog lines about "hooks disabled at session start"
  or hook config are still 👀 (config surface), but do not restore Claude-style
  stdout injection unless Grok documents a behavior change.

## Index

| Grok version | Headline | CAS verdict | Pointer |
|--------------|----------|-------------|---------|
| 0.2.101 | **grok inspect** multi-harness compatibility settings · TUI refresh cadence · queue/status/subagent polish · rate-limit copy | 👀 / ✅ / ⏭ | this doc |
| 0.2.100 | **Session picker + welcome resume** across Claude/Codex/Cursor · web-fetch artifacts · queue/multiline Enter · pane-closed resume crash · hooks honor disabled-at-start · long-turn status markers | 👀 / ✅ / ⏭ | this doc |
| *(seed floor)* | No older versions in host `~/.grok/CHANGELOG.md` as of 2026-07-14 | — | — |

---

## Entries

### 0.2.101 — inspect multi-harness settings · queue/status polish · refresh rate

Reviewed 2026-07-14 (w-grok-diary / cas-5828). Host install is **0.2.101**.
Source: `~/.grok/CHANGELOG.md` (2026-07-13).

- **"grok inspect now shows effective compatibility settings for Cursor, Claude, and
  Codex sessions."** → 👀 **opportunity / ops win, no CAS code required.** Multi-harness
  inspect is exactly the debugging surface factory hosts need when mixing CLIs. Does not
  change spawn flags; useful when validating MCP discovery and compat layers after
  upgrades. No task.
- **"New setting: Match display refresh rate" (native high-refresh TUI cadence).** →
  ⏭ n/a (host TUI preference; orthogonal to `PtyConfig::grok`).
- **"Parked subagent status no longer duplicates or interleaves incorrectly in
  scrollback."** → ✅ no action — render fix. Factory may spawn Grok-side subagents;
  cleaner scrollback only. Not a spawn/MCP/rules break.
- **"Status line during waits shows elapsed time before the queued-message hint."** →
  ⏭ n/a (TUI chrome).
- **"Queued messages sent with Enter now appear immediately instead of vanishing
  briefly."** + related queue reliability in 0.2.100 → 👀 **watch (factory messaging
  UX).** Supervisor→worker delivery often lands as injected/queued turns. Appearance
  glitches can look like "message lost" during ops; this is a harness fix, not a CAS
  change. Verify subjectively on upgrade if operators still report vanished queue items.
- **"Resume hint after quitting minimal mode prints the correct `grok --minimal
  --resume` command."** → ⏭ n/a (minimal-mode UX; factory workers are not launched in
  that interactive path).
- **"Rate-limit messages correctly direct API-key users to team plans."** → ⏭ n/a
  (billing/copy).

### 0.2.100 — cross-harness session picker · queue Enter · hooks disabled-at-start · pane-closed crash

Reviewed 2026-07-14 (w-grok-diary / cas-5828). Source: `~/.grok/CHANGELOG.md`
(2026-07-13). **Seed-floor version** — oldest section currently present in the local
changelog; no pre-0.2.100 entries inventable from this host.

- **"Session picker discovers and resumes recent Claude Code, Codex, and Cursor
  sessions"** + **"Welcome screen one-click resume nudge for recent Claude, Codex, or
  Cursor sessions."** → 👀 **strategic / host UX, not factory spawn.** Interesting for
  multi-harness hosts running CAS, but factory panes use fresh `--session-id` UUIDs and
  do not resume foreign harness sessions via this picker. No code action; note for
  onboarding docs only.
- **"Web fetch tool preserves full truncated page content as readable artifacts."** →
  ✅ no action (agent tool quality; not a launch touchpoint).
- **"Multiline mode correctly sends the top queued message on empty Enter when a turn
  is running"** + **"Queued commands no longer disappear or delay when pressing Enter
  twice quickly during a running turn."** → 👀 **watch — input/queue path.** Same class
  as 0.2.101 queue-visibility fixes: factory coordination depends on messages actually
  enqueueing during long turns. Harness-side reliability win; smoke "message during
  running turn" after big Grok bumps.
- **"Minimal mode text readable on dark terminals."** → ⏭ n/a.
- **"Grok no longer crashes when printing resume hints after the terminal pane has
  closed."** → 👀 **watch — factory mux / pane lifecycle.** Factory workers run inside
  CAS-managed panes; a crash on post-close resume-hint printing could have looked like
  a worker death. Fix is pure harness; confirm no residual panic on worker shutdown
  after upgrade. No CAS change expected.
- **"Long-running turns with multiple waits show updated status markers instead of
  appearing stuck."** → ✅ no action — direct win for long factory tasks (stall
  false-positives from "stuck" UI). Complements CAS is-wedged logic; does not replace
  transcript-path correctness (`~/.grok/sessions/*` + `CAS_FACTORY_WORKER_CLI=grok`).
- **"Claude and Cursor hooks are now correctly disabled at session start when disabled
  in config."** → 👀 **touchpoint: hooks/config posture.** Grok already ignores
  SessionStart *stdout* for CAS context injection (we use `--rules` + env). This line is
  about honoring "disabled in config" for Claude/Cursor-compat hooks — verify that
  disabling hooks in config does not also strip something CAS still relies on (unlikely
  for factory spawn, since we do not depend on SessionStart stdout). On upgrade, re-check
  that `CAS_SESSION_ID` registration and `--rules` role text still land with hooks
  disabled.

---

## Backlog of opportunities (not required, tracked)

- **0.2.93 → 0.2.101 upgrade validation:** re-run the live checklist that pinned 0.2.93
  (`grok --help` flags, `grok inspect`, `grok mcp doctor`, factory spawn smoke):
  `--permission-mode bypassPermissions`, `--session-id`, `-m` / `--reasoning-effort`,
  `--cwd`, `--rules`, MCP discovery of `cas` without per-spawn `-c`, env inheritance
  into `cas serve`, tools as `cas__*`, transcripts under `~/.grok/sessions/*`.
- **Validated-pin comment bump:** after a successful factory smoke on 0.2.101, update
  the `PtyConfig::grok` "Verified against … 0.2.93" comment (and this diary's Version
  status) so the pin tracks reality.
- **Changelog history depth:** when Grok exposes older releases beyond the two-section
  local CHANGELOG, backfill entries without inventing notes; until then keep the seed
  floor explicit.
- **SessionStart stdout:** if a future Grok release starts delivering SessionStart
  stdout like Claude, re-evaluate whether `--rules` remains the sole context path or
  becomes defense-in-depth (would be a deliberate EPIC, not a silent drop of `--rules`).
- **Queue/input reliability:** 0.2.100–0.2.101 cluster of queue/Enter fixes — if factory
  operators still report lost mid-turn messages on Grok workers, capture repro before
  assuming CAS delivery is at fault.
