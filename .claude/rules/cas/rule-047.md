---
id: rule-047
paths: "**/*.rs,**/*.ts,**/*.tsx"
---

Before closing a task, verify all work is complete:

1. **Acceptance criteria**: All criteria are implemented and working - not just partially done
2. **Tests pass**: Run `cargo test` (Rust) or `npm test` (JS/TS) on modified packages
3. **Test coverage**: When acceptance criteria require test coverage, tests must exercise the actual functionality - not just trivial cases like constructors
4. **No stub code**: All defined interfaces, API fields, and function signatures must have real implementations

Do not close tasks when:
- Acceptance criteria remain unimplemented
- Tests are failing
- Required test coverage is missing
- Code contains unimplemented stubs or placeholder logic