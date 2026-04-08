# task-verifier close-reason regression fixture (cas-7c37)

Manual test cases for `cas-cli/src/builtins/agents/task-verifier.md` Step 0.
When the agent prompt changes, re-run these against a scratch verifier session
and confirm the expected verdict.

## Case 1 — ACCEPT (OpenClaw repro)

**Task AC**: "Daemon upgraded to 2026.4.8, Slack probe returns green, Signal
user removal verified."

**Close reason**:
> Daemon upgraded to 2026.4.8. Slack probe green. Signal confirmed user-removed
> pre-upgrade pending dedicated bot number — runbook updated to match.

**Expected**: APPROVE. Every AC item is described as satisfied. "pending
dedicated bot number" is a forward-looking roadmap note that does not belong
to this task's AC. Rejecting on keyword "pending" was the bug cas-7c37 fixes.

## Case 2 — ACCEPT (follow-up note)

**Task AC**: "Migrate 12 users from legacy auth to new SSO."

**Close reason**:
> Migrated 12 users. Follow-up: monitor for 24h and file any edge cases
> separately.

**Expected**: APPROVE. Migration complete; the monitoring follow-up is out of
this task's scope.

## Case 3 — REJECT (actual AC gap)

**Task AC**: "Implement login and logout flows for OAuth provider."

**Close reason**:
> Partially implemented auth; login works but logout still broken.

**Expected**: REJECT. The close reason explicitly says an AC item (logout) is
not done. Category: `incomplete_close_reason`.

## Case 4 — REJECT (stub admission)

**Task AC**: "Implement `compute_tax()` per state table."

**Close reason**:
> Stubbed `compute_tax()` to return 0 for all states; foundation for future
> work.

**Expected**: REJECT. "Stubbed" + "foundation for future work" directly admits
the AC is not implemented.

## How to run

Paste each close reason into a scratch `mcp__cas__task action=close` call on a
throwaway task whose AC matches the stated AC, observe the verifier subagent's
verdict, compare to expected. Or manually invoke the task-verifier agent with
the close reason and task context and read its summary.
