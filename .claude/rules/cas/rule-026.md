---
id: rule-026
paths: "cas-cli/src/search/hybrid.rs"
---

Filter semantic search results to exclude invalid entries:
- Exclude archived entries from results in hybrid search implementation
- Only include IDs that exist in the active entries list
- Validate entry existence before returning results