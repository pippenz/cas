---
id: rule-055
---

Claude Code instances cannot pull or poll for data - they only respond to prompts. All inter-agent communication in factory mode must be push-based through the Director/TUI:

1. **State changes go to CAS** - Agents update tasks, memories, etc. in CAS
2. **Notifications go through prompt queue** - Use `mcp__cas__agent action=prompt target=X prompt="..."` 
3. **Director processes the queue** - TUI polls prompt queue and injects into target PTY

Example flows:

**Supervisor assigns task to worker:**
```
1. Supervisor: mcp__cas__task action=create ... assignee=swift-fox
2. Supervisor: mcp__cas__agent action=prompt target=swift-fox prompt="Check your assigned tasks"
3. Director injects prompt → Worker sees it → Worker checks CAS
```

**Worker completes task:**
```
1. Worker: mcp__cas__task action=close id=cas-123
2. Worker: mcp__cas__agent action=prompt target=supervisor prompt="Task cas-123 complete"
3. Director injects prompt → Supervisor sees it → Supervisor coordinates next steps
```

Never expect Claude instances to poll for updates - always push via prompts.