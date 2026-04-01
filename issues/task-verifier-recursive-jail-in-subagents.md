# Task Verifier Subagent Gets Trapped in Its Own Verification Jail

## Summary

When a task-verifier subagent tries to close a task after verifying it, CAS triggers verification jail on the subagent itself — requiring it to spawn another task-verifier. This creates a recursive loop. The subagent can verify the code but cannot record the verification or close the task.

## Reproduction

1. Supervisor spawns task-verifier subagent: `Agent(subagent_type="task-verifier", prompt="Verify task X")`
2. Subagent reviews code, approves the work
3. Subagent tries `mcp__cas__task action=close id=X reason="..."`
4. CAS returns: "VERIFICATION REQUIRED — spawn a task-verifier subagent"
5. Subagent is itself a task-verifier — it can't spawn another one inside itself
6. Subagent reports back: "Verification approved but I cannot close the task"

## Root Cause

The verification system treats the task-verifier subagent as a primary/supervisor agent (inherits the parent's agent type). It doesn't recognize that it IS the verifier and should be allowed to record its own verification.

## Proposed Fix

Task-verifier subagents should register as a special agent type that is authorized to record verifications and close tasks. Or: the `mcp__cas__task action=close` call should detect that it's being called from within a task-verifier context and skip the "spawn a verifier" requirement.
