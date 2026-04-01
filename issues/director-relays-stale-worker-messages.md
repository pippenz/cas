# Director Relays Stale/Redundant Worker Messages After Shutdown

## Summary

After workers are shut down, the director agent continues relaying their messages (often identical "MCP tools unavailable" or "standing by" messages) as teammate notifications. These arrive interleaved with actual work, creating noise and confusion.

## Examples From Today

After `shutdown_workers count=0`:
- "Worker true-gopher-21: MCP CAS tools are NOT loading..."
- "Worker steady-cheetah-43: BLOCKED. Neither CAS MCP tools..."
- Multiple idle_notification JSON payloads from shut-down workers

## Impact

- Supervisor has to mentally filter stale messages from live ones
- Can trigger unnecessary responses ("let me redirect that worker" — worker is already dead)
- Clutters the conversation context

## Proposed Fix

1. Director should not relay messages from workers that have been shut down
2. Or: shutdown should be synchronous — wait for workers to actually stop before returning
3. Or: tag relayed messages with worker liveness status so supervisor can ignore dead workers
