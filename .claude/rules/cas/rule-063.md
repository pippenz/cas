---
id: rule-063
paths: "**/*.rs"
---

Use non-blocking mode for unix domain sockets in event loops on macOS. Socket read timeouts (set_read_timeout) on unix domain sockets may not work reliably, causing blocking reads that freeze single-threaded main loops. Use set_nonblocking(true) instead.