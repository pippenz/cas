---
id: rule-060
paths: "**/*.rs"
---

When processing paths from git status output, always check is_dir() before file operations. Git reports untracked directories (e.g., '?? path/to/dir/') and attempting to read these as files can cause hangs on macOS.