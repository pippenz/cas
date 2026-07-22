# Slack release notes — v2.28.5 (2026-07-22)

Channel: #cas-internal

## User thread

**Top-level:**
Live on production — **User**: Code review now shows you every finding it considered — nothing gets silently thrown away anymore.

**Reply:**
Was → Now:
- Was: the review pipeline quietly discarded reviewer findings it wasn't confident about — including one important real bug that was only recovered by digging through raw logs. Now: every set-aside finding is listed in the review result with who found it and exactly why it was set aside, so you can rescue anything that matters.
- Was: closing a finished epic printed a scary "orphaned task" warning even when everything was healthy. Now: a normal close says exactly what happened.
- Was: routine test runs triggered false "you're filling up memory" warnings, and the codemap kept claiming it was stale right after being updated. Now: both warnings only fire when something is actually wrong.
- Was: background workers sometimes asked for merges that had already happened, got nagged to take work they were told to hold off on, and could linger as zombie processes after shutdown. Now: workers check for fresh instructions before asking, hold-state is respected, and shutdown means shutdown.

## Dev thread

**Top-level:**
Live on production — **Dev**: The review merge is lossless (`dropped[]` with provenance), and a day of live factory friction — stale escalations, phantom task states, false guardrail/codemap warnings, idle-nudge races, a shutdown wedge — is fixed at the root.

**Reply:**
Was → Now:
- Was: `mergeFindings` filtered sub-threshold findings with no trace. Now: schema-invalid and confidence-gated findings land in `dropped[]` with reviewer, reason, and threshold; each drop is logged and counted in `stats.dropped_findings`; the codex adapter must emit schema-complete findings; byte- and behavior-parity tests lock the three copies of the merge logic together.
- Was: worker MERGE-REQUIRED guidance said to drain the inbox with `queue_poll` — which reads the supervisor queue and can never see supervisor→worker replies. Now: guidance reflects the real delivery mechanism (re-read just-delivered messages) and every escalation embeds the branch tip SHA + freshness qualifier, so stale requests are verifiable on sight.
- Was: `task action=release` deleted the lease but left status in-progress (invisible to the ready pool); supervisor-owned epic closes were labeled "orphaned". Now: release reopens the task with an audit note; epic owner-close has distinct response and audit wording.
- Was: `factory_mcp_ops_test` mutex-poison cascades under parallel `cargo test`, plus one test doing env-sensitive setup before its guard. Now: poison-tolerant process-wide env lock, guard-first ordering, and a canonical 8-var `env -u` recipe documented in the worker skill — proven with consecutive green parallel runs.
- Was: codemap staleness compared commit timestamps (`--since`), counting same-commit siblings as drift; the tmpfs guardrail counted transient test-tempdir churn as staged growth. Now: strict `commit..HEAD` range with positive-path coverage; guardrail requires growth to persist across two samples (persistent large artifacts still warn).
- Was: director WorkerIdle nudges re-fired against workers on deliberate standby and raced assignment writes; queued shutdowns wedged behind slow spawns leaving zombies. Now: supervisor-contact-aware suppression, delivery-time revalidation, and shutdown bypass with late-spawn discard.
