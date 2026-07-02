---
from: gabber-studio team (factory supervisor wild-spider-85)
date: 2026-06-10
priority: P2
---

# Factory liveness signals disagree: working agents shown stale/unregistered, dead workers leave phantom panes — supervisor can't trust `worker_status`

## Resolution (cas-c9f0, 2026-07-02)

Resolved and moved to completed after verifying each acceptance criterion. Most liveness plumbing had already shipped; cas-c9f0 closes the remaining missing affordance: one-command purge of stale/shutdown worker records.

- Busy agents during long tool calls: already fixed/proven. Agent heartbeat is sent by the daemon on a 30s interval independent of individual MCP handler execution in `cas-cli/src/mcp/daemon.rs:344`-`354` and `daemon.rs:451`-`453`, with retry/backoff in `daemon.rs:934`-`979`. `worker_status` also cross-checks recent worker I/O before marking a stale-heartbeat worker stale, reporting `[heartbeat stale, active I/O]` instead of pruning in `cas-cli/src/mcp/tools/service/factory_ops.rs:318`-`370` and `factory_ops.rs:426`-`437` (commit `aa500195`). Regression coverage for the active-I/O guard is in `factory_ops.rs:2353`-`2435`. Agent listing is scoped to the current factory roster and does not perform the worker-status stale prune in `cas-cli/src/mcp/tools/core/agent_coordination/agent_management.rs:96`-`177`.
- Consistent current-session views: already fixed/proven for the director/factory surfaces that caused this report. Director data only includes `Active`/`Idle` factory-relevant agents in `crates/cas-factory/src/director.rs:329`-`338`, then the TUI filters that data to current `worker_names` plus supervisor in `cas-cli/src/ui/factory/app/mod.rs:512`-`524` before prompt generation in `app/mod.rs:651`-`666`. `worker_status` lists active worker records only in `cas-cli/src/mcp/tools/service/factory_ops.rs:372`-`394` and labels the output as `Workers (N)` in `factory_ops.rs:419`-`423`.
- Dead worker reaping: already fixed/proven. Graceful shutdown marks the worker shutdown, kills the process tree, removes it from `worker_names`, refreshes director data, calls `event_detector.remove_worker`, and rebuilds the pane grid in `cas-cli/src/ui/factory/app/render_and_ops/epic_workers.rs:456`-`516`. Unexpected pane/process exits call `mark_worker_crashed`, which removes the worker from `worker_names`, suppresses future events, rebuilds the pane grid, and reclaims the worktree when tasks are closed and the tree is clean in `epic_workers.rs:591`-`614`. The daemon heartbeat liveness gate stops heartbeating dead/recycled client PIDs and marks the agent stale in `cas-cli/src/mcp/daemon.rs:862`-`930`, with tests in `cas-cli/src/mcp/daemon_tests/tests.rs:96`-`149` and `daemon_tests/tests.rs:295`-`350`.
- Single purge command for dead records: fixed by cas-c9f0. `mcp__cas__coordination action=gc_cleanup` / `mcp__cs__coordination action=gc_cleanup` now marks stale worker records and unregisters stale/shutdown non-supervisor records in `cas-cli/src/mcp/tools/service/factory_ops.rs:1076`-`1114`, reporting `Dead agent records purged` in `factory_ops.rs:1147`. Regression coverage is `test_gc_cleanup_purges_stale_and_shutdown_worker_records` and `test_gc_cleanup_preserves_stale_supervisors` in `cas-cli/tests/factory_mcp_ops_test.rs:761`-`809`.
- FACTORY active count ambiguity: already narrowed/proven by current code for worker count surfaces. The factory app's worker count is derived from `worker_names.len()` in `cas-cli/src/ui/factory/app/mod.rs:737`-`740`; spawn/respawn telemetry names it `workers_active` in `cas-cli/src/ui/factory/app/render_and_ops/epic_workers.rs:432`-`436` and `epic_workers.rs:704`-`708`; `worker_status` renders the count as `Workers (N)` with coverage in `cas-cli/tests/factory_mcp_ops_test.rs:481`-`505` and `factory_mcp_ops_test.rs:1139`-`1145`.

A factory supervisor has at least four ways to ask "which agents/workers are alive": the `worker_status` MCP action, the `agent_list` MCP action, the TUI **FACTORY** pane, and the actual OS process table. During a routine cleanup on 2026-06-10 (gabber-studio, `cas 2.20.0`) **all four disagreed with each other**, in both directions:

- agents that were **actively working** were reported **stale / not-registered**, and
- workers that were **dead** still appeared as live tmux panes (and the FACTORY pane showed a non-zero "active" count).

Net effect: the supervisor cannot trust any single liveness signal. The dangerous failure mode is acting on a false "stale / None active" reading — e.g. `shutdown_workers` or `unregister` against agents that are actually mid-task. (In this incident the workers were genuinely dead, so no live work was lost — but that was luck, not a guarantee the tooling provides.)

## Affected version

`cas 2.20.0 (3badbb9-dirty 2026-06-07)`, factory/teammate (tmux) mode.

## What I observed (evidence, same session)

1. **`worker_status` → "Workers: None active"** (returned twice during the session).
2. **`agent_list` → 17 agents**, of which the gabber factory workers (`wise-jaguar-67`, `quick-jaguar-64`, `zen-spider-44`, `fair-condor-40`, …) were all `[stale]` or `[shutdown]`; the two supervisors were the only `[active]`/recent entries.
3. **My own supervisor agent (`wild-spider-85`, `189c8855-…`) was listed `[stale]`** in `agent_list` *while actively executing tool calls* — its heartbeat had lapsed during a sequence of long-running `Bash` polls (each 40–120 s). It is the live, driving session; it should never read as stale.
4. **TUI FACTORY pane disagreed with the registry:** it rendered `EPIC: cas-39ba — 2 active, 0 queued` **and** `wild-spider-85: not registered` — i.e. the same supervisor that `agent_list` *does* list (as stale) is reported by the FACTORY pane as **not registered at all**. Two views, opposite answers, same agent.
5. **tmux panes for dead workers persisted**, showing frozen final output (`wise-jaguar-67`, `zen-spider-44`, `quick-jaguar-64`) — which reads, at a glance, as "these workers are alive and mid-task."
6. **Ground truth from `ps`:** there were **no live processes** for any of those worker names. The named gabber workers were absent from the process table entirely; the table was full of `[claude] <defunct>` zombies. The only live `claude` sessions were two supervisors (one for `ozer-brave-puma-36`, one for `gabber-studio-subtle-dragon-36`) and two unrelated general-purpose teammates. The workers' worktrees (`.cas/worktrees/*`) had already been removed and the dir was empty.

So `worker_status: None active` was, in this case, **correct** — but `agent_list` (stale registrations lingering), the FACTORY pane (`2 active` + supervisor `not registered`), and the visible tmux panes all painted a contradictory picture, and none of them reconciled against `ps`.

## Two distinct defects, likely a shared root

### A. Active agents misclassified as stale / not-registered
- Heartbeat-based liveness appears to use an **over-aggressive staleness threshold** (the summary of this factory's behavior cites a ~30 s `worker_status` filter) **and the heartbeat is not refreshed during long synchronous tool calls.** A supervisor or worker running a single 60–120 s `Bash`/agent step stops heartbeating for the duration and is then filtered out as stale / counted as not-registered — even though it is the live, working session.
- The FACTORY pane's `not registered` vs `agent_list`'s `[stale]` for the *same* agent shows the FACTORY monitor and the registry derive "registered/live" from **different sources that aren't reconciled.**

### B. Dead workers are not reaped from any view
- Exited/`<defunct>` worker processes leave behind: (i) lingering `agent_list` registrations (`[stale]`/`[shutdown]` rather than removed), (ii) **stale tmux panes** with frozen output, and (iii) — pending confirmation — a non-zero "active" count in the FACTORY pane. (Caveat: `2 active` on `cas-39ba` *might* be counting the epic's 2 still-open child **tasks** — `cas-18be`, `cas-ed69` — rather than workers; if so, the label "active" is ambiguous between tasks and workers and should be disambiguated. The `wild-spider-85: not registered` line is unambiguous, however.)
- `agent_cleanup` (run with `stale_threshold_secs=900`) reported `Stale agents marked: 0, Expired leases reclaimed: 0` and did **not** remove the already-`[stale]`/`[shutdown]` records — so there was no one-shot "purge dead agents" affordance; I had to `unregister` 15 agents by UUID individually.

## Impact

- **Supervisor decision-making is unsafe.** A supervisor that trusts `worker_status`/`agent_list`/FACTORY can either (a) `shutdown_workers`/`unregister` an agent that is actually mid-task (false-stale → killed live work — would be P1), or (b) believe phantom workers are running and wait on them forever.
- **User-visible confusion.** The operator saw worker panes + a `2 active` FACTORY count and reasonably concluded the supervisor was "failing to see workers," when in fact the workers were dead and the displays were stale.

## Suggested fixes

1. **Refresh the agent heartbeat around long tool calls** (or heartbeat on a background timer independent of tool-call boundaries) so a busy agent never reads as stale purely because it's busy.
2. **Raise / make-configurable the `worker_status` staleness threshold**, and/or distinguish "stale-heartbeat" from "process-dead" instead of collapsing both into "not active."
3. **Reconcile the FACTORY pane, `agent_list`, and `worker_status` against a single source of truth** for registration + liveness. The same agent must not be simultaneously `[stale]` (registry) and `not registered` (FACTORY).
4. **Reap dead workers:** on process exit / worktree removal, drop the agent registration, close/mark the tmux pane, and decrement any factory "active" tally. If a worker's worktree no longer exists, it cannot be active.
5. **Disambiguate the FACTORY "N active" label** — is it active *tasks* or active *workers*? Show both, or label clearly.
6. **Give `agent_cleanup` (or a new `agent_purge`) a mode that actually removes `[stale]`/`[shutdown]` records**, not just marks freshly-stale ones — so cleanup isn't 15 manual `unregister` calls by UUID.

## Repro sketch

1. Spawn N factory workers in tmux mode; let them complete and exit (or remove their worktrees).
2. From the supervisor, run a single long (>30 s) `Bash` step, then call `worker_status` and `agent_list`.
3. Observe: the supervisor itself may show `[stale]`; the dead workers linger as `[stale]`/`[shutdown]` with live-looking tmux panes; the FACTORY pane shows a non-zero active count and/or `<supervisor>: not registered`; `ps` shows no matching live processes.

## Acceptance criteria

1. An agent actively executing a long (>60 s) tool call is **never** reported stale / not-registered by `worker_status`, `agent_list`, or the FACTORY pane.
2. `worker_status`, `agent_list`, and the FACTORY pane return **mutually consistent** registration + liveness for every agent (no `[stale]`-here / `not-registered`-there for the same UUID).
3. When a worker process exits or its worktree is removed, its registration, tmux pane, and any factory "active" count are reaped within one monitor tick.
4. A single command purges all dead (`[stale]`/`[shutdown]`) agent records.
5. The FACTORY "N active" figure is unambiguously labeled (tasks vs workers) and matches `ps` for workers.
