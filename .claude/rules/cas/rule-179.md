---
id: rule-179
---

Before creating or breaking down an EPIC, supervisors MUST search for learnings from related past work:

1. **Search for related EPICs**: `mcp__cas__task action=list task_type=epic status=closed` and find similar epics
2. **Review epic details**: Check design, notes, and subtask notes for discoveries and blockers
3. **Search memories**: `mcp__cas__search action=search query="<relevant keywords>" doc_type=entry`
4. **Apply learnings**: Add discovery notes to new tasks referencing what was learned

Example searches before an epic:
- "What issues did we hit last time we worked on X?"
- "Are there existing components/patterns we can reuse?"
- "What architectural decisions were made?"

This prevents re-learning the same lessons and ensures proven patterns are reused.