---
id: rule-057
paths: "cas-cli/src/mcp/**/*.rs,crates/cas-mcp/src/types.rs"
---

Implement all fields defined in API request types. Do not define parameters in MCP request structs (SearchContextRequest, TaskRequest, etc.) without implementing their functionality in the corresponding handler. Dead fields in API schemas indicate incomplete work.