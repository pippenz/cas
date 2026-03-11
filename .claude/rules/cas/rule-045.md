---
id: rule-045
paths: "**/*.rs,**/*.ts,**/*.py"
---

Avoid temporal language in code comments: 'for now', 'temporarily', 'later', 'eventually', 'in the future', 'reserved for future'. These indicate shortcuts that bypass proper implementation. Either implement the feature completely or explicitly document it as out of scope.