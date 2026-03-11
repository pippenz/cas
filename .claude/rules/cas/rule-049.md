---
id: rule-049
paths: "cas-cli/src/hooks/**/*.rs,**/*handler*.rs,**/cleanup*.rs"
---

In cleanup/stop handlers, perform all cleanup operations (releasing resources, clearing state, unregistering, cleanup_agent_leases, graceful_shutdown) BEFORE any early return checks or conditional logic. This prevents resource leaks when early return conditions skip cleanup code.

Example in Stop hook: cleanup must happen BEFORE block_stop_with_context() calls, otherwise the agent gets marked as shutdown while still active.

**SubagentStop Exception:** SubagentStop hook must NOT perform agent cleanup (graceful_shutdown, cleanup_pid_mapping, etc.). The session_id in SubagentStop belongs to the PARENT agent, not the subagent. Only clean up verifier marker files in SubagentStop.