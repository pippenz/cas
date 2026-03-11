---
id: rule-059
---

Always set busy_timeout on new SQLite connections in multi-agent scenarios. Use the SQLITE_BUSY_TIMEOUT constant (30 seconds) via conn.busy_timeout() to make SQLite wait before returning SQLITE_BUSY errors when multiple agents contend for the write lock.