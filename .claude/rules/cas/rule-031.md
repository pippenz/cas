---
id: rule-031
paths: "src/cli/*.rs"
---

Use subcommand pattern for multi-command CLI features. Name dispatcher functions execute_subcommand, parameters 'cmd' for subcommands and 'args' for flat arguments.