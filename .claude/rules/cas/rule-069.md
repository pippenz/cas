---
id: rule-069
paths: "**/*.rs"
---

When removing fields from structs or deleting variables, search for and remove ALL references to those fields/variables. Use grep to find usages before confirming removal is complete. Partial removal causes compilation errors from undefined references.