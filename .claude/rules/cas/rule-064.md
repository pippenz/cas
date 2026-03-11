---
id: rule-064
paths: "**/*daemon*.rs,**/*server*.rs"
---

In daemon accept/read loops, use `.is_err()` checks with `continue` instead of `?` error propagation. Transient client errors (bad socket, timeout) should skip the client, not crash the daemon. Example: `if stream.set_nonblocking(false).is_err() { continue; }`