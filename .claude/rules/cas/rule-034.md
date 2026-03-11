---
id: rule-034
paths: "src/**/*.rs"
---

Handle dead/unused code appropriately:
- Use #[allow(dead_code)] with doc comments explaining why code exists (for scaffolding or future features)
- Do not add unused parameters or fields with 'reserved for future use' comments - either implement or omit
- Prefer integrating scaffolding into active functionality rather than leaving it unused long-term