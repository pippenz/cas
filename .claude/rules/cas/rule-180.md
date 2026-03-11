---
id: rule-180
paths: "**/*.rs,**/*.ts,**/*.py"
---

Remove all placeholder implementations that admit incomplete work. Comments like "In a full implementation...", "For now, we just...", or "Note: ..." followed by descriptions of missing functionality indicate work that is not complete. All protocol handshakes, state recovery, and error handling must be fully implemented - not deferred or stubbed.

Examples to reject:
- "In a full implementation, we'd need to..." - implement it fully
- "For now, we just..." - complete the work, don't defer
- Comments describing what SHOULD happen but doesn't - implement what should happen
- Dropped handles or broken message loops - fix the loop properly

This applies especially to:
- Protocol handshakes (Connect/Connected messages)
- Reconnection and state recovery
- Message loop state management
- Error handling and cleanup