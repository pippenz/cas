---
id: rule-001
paths: "src/store/**"
---

All storage operations should go through Store traits, not direct database access. This ensures consistent behavior across SqliteStore and MarkdownStore implementations.