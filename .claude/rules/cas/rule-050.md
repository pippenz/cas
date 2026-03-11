---
id: rule-050
paths: "cas-cli/src/mcp/**/*.rs"
---

In MCP tool handlers, use self.inner.get_agent_id() instead of self.inner.agent_id.get(). The get_agent_id() method performs auto-registration from the session file; direct OnceLock access only works after explicit MCP registration.