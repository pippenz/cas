# Slack draft — v2.28.4 tmpfs guardrail + close-lint accuracy (#cas-internal C0B44GUKDK2)

## Post 1 — User thread

**Top-level:**
Live on production — **User** (v2.28.4)
Your machine is now protected from agents quietly filling RAM-backed temp storage with huge files. Last week that exact failure ate all 16GB of swap on a dev box, got the operator's apps OOM-killed for days, and left 17GB of sole-copy audio one reboot from vanishing — from now on the agent gets warned in the moment and told where to put big files instead.

**Threaded reply:**
Was → Now
- Was: nothing watched writes into memory-backed locations like /tmp. Big files accumulated silently for weeks, machine performance degraded with no visible cause, and the files themselves would not survive a reboot. Lessons like "this machine's /tmp lives in RAM" were remembered only inside the project where they were learned — other projects on the same machine repeated the mistake.
- Now: the moment a session's writes to a RAM-backed location grow past a threshold (1GB by default), the agent gets a clear warning naming the durable staging directory to use instead. You can set that directory once per machine, and every agent on every project sees it at session start. Machine-level facts can now be saved so they follow the machine, not the project.
- Also fixed: bogus task-completion complaints. The pre-completion code check used to point at the wrong line, flag legitimate documentation comments as dead code, and keep complaining even after the problem was fixed. Its findings now name the right file and line, respect documentation, and clear as soon as a fix lands.

## Post 2 — Dev thread

**Top-level:**
Live on production — **Dev** (v2.28.4)
New warning-only tmpfs/ramfs write guardrail on PostToolUse (dual accounting: written-bytes + usage-growth per mount, flocked state, zero hot-path cost for non-write tools), `[staging] large_artifact_dir` host config, `host:<hostname>`-tagged memory injection — plus the close-gate structural lint now reports per-file line numbers and evaluates the branch tip.

**Threaded reply:**
Was → Now
- Was: no guardrail on writes to memory-backed mounts; host-level constraints trapped in project-scoped memory; the close-gate lint indexed lines across the whole flattened diff (wrong line numbers in multi-file diffs), merged comment runs across file/hunk boundaries (false violations), flagged XML block doc-headers as commented-out code, and linted the task-tagged commit so follow-up fixes never cleared findings.
- Now, guardrail: per-session state per mount tracks Write/Edit byte deltas and Bash-sampled usage growth separately (either crossing the threshold warns, once per multiple); single-shot fills are caught on the first sample; all tmpfs/ramfs mounts are enumerated; state is advisory-flocked against concurrent hook processes and pruned by retention. Gating happens before any config/mounts/state I/O, so Read/Grep/etc. pay nothing. Warning-only — no deny path exists.
- Now, config: `[staging] large_artifact_dir` resolves project-first with a host-level fallback that merges ONLY the staging section — operator-level `~/.cas/config.toml` sections (hooks, telemetry, llm, factory) can never leak into project config, and all hook events see one consistent view. Settable via `cas config set`.
- Now, memory: global entries tagged `host:<hostname>` inject into SessionStart context for any project on the machine, filtered at the SQL layer (no full-table scan) and capped under the SessionStart size budgets.
- Now, lint: findings carry file + per-file line numbers, comment runs flush at file and hunk boundaries, XML/HTML block comments pass, and the lint evaluates the cumulative worker range at branch tip — a follow-up fix commit clears its finding.
- The epic-level multi-persona review caught three P1s pre-merge (hot-path config loads on every tool call, section-wide host-config leakage, first-sample baseline miss) — all fixed before this release.
