---
from: Petra Stella Cloud team
date: 2026-04-13
priority: P2
---

# Feature: Per-event golden-JSON tests for hook output

## Problem

The hook subsystem has 50+ unit tests but **zero schema-conformance tests** for the JSON Claude Code actually receives. Two recently-discovered bugs (commit `baa540b`) emitted JSON that Claude Code's runtime schema validator rejects wholesale:

```
JSON validation failed: Hook JSON output validation failed:
- : Invalid input
```

When this happens the entire hook output is discarded — no error surfaces to the user beyond a stderr line, and any context the hook tried to inject (CODEMAP reminder, loop iteration prompt) is silently lost.

The new test added in commit `dcd99bc` (`test_with_system_context_has_no_hook_specific_output`) covers one helper. We need broader coverage.

## Why this matters

The bug class is "valid Rust struct, invalid Claude Code JSON." Type checking won't catch it (until/unless `FEATURE-hook-output-typed-enum.md` lands). Without a schema test, the only way bugs surface is through user reports of missing context — and most users won't notice missing context.

We're flying blind on the contract with the harness.

## Proposed solution

One serialization test per hook event, asserting the emitted JSON conforms to Claude Code's documented schema. Schema reference (from a real rejection):

```
"hookSpecificOutput": {
  "for PreToolUse":      { hookEventName, permissionDecision?, permissionDecisionReason?, updatedInput? }
  "for UserPromptSubmit":{ hookEventName, additionalContext (required) }
  "for PostToolUse":     { hookEventName, additionalContext? }
}
```

Top-level fields like `continue`, `suppressOutput`, `stopReason`, `decision` ("approve"|"block"), `reason`, `systemMessage`, `permissionDecision` ("allow"|"deny"|"ask") are universal.

**Tests to add** (one per file, location: `crates/cas-core/src/hooks/types.rs` test module or a new `crates/cas-core/tests/hook_schema.rs`):

```rust
#[test]
fn pretooluse_output_schema() {
    let out = HookOutput::with_permission_decision("PreToolUse", "allow", "test");
    let json: serde_json::Value = serde_json::to_value(&out).unwrap();
    let hso = &json["hookSpecificOutput"];
    assert_eq!(hso["hookEventName"], "PreToolUse");
    assert!(hso.get("additionalContext").is_none(), "PreToolUse must not have additionalContext");
    // ... similar for permissionDecision allowed values
}

#[test]
fn stop_output_never_has_hook_specific_output() {
    // Cover all Stop-producing constructors:
    for out in [
        HookOutput::empty(),
        HookOutput::with_system_context("ctx".into()),
        HookOutput::block_stop("reason".into()),
        HookOutput::block_stop_with_context("Stop", "r".into(), "c".into()),
    ] {
        let json: serde_json::Value = serde_json::to_value(&out).unwrap();
        assert!(json.get("hookSpecificOutput").is_none(),
            "Stop output must not contain hookSpecificOutput, got: {json}");
    }
}

#[test]
fn userpromptsubmit_requires_additional_context() { /* ... */ }

#[test]
fn posttooluse_additional_context_optional() { /* ... */ }
```

Plus one **integration-style test** that pipes a fake `HookInput` through each `handle_*` handler and asserts the resulting JSON is schema-valid. This catches handler-level mistakes that unit tests on constructors won't.

## Acceptance criteria

- [ ] Schema test exists for every HookEvent variant (PreToolUse, PostToolUse, UserPromptSubmit, Notification, Stop, SubagentStop, PreCompact, SessionStart, SessionEnd, SubagentStart, PermissionRequest)
- [ ] Each test asserts both presence of required fields AND absence of forbidden fields
- [ ] Tests catch the two bugs that just shipped (regression coverage for `baa540b`)
- [ ] All tests pass: `cargo test -p cas-core --lib hooks`

## Related

- `FEATURE-hook-output-typed-enum.md` — type-level fix that makes most of these tests redundant. These tests are insurance for the period before that lands, and cover handler-level mistakes (wrong builder used) even after.
- Existing test added in `dcd99bc`: `test_with_system_context_has_no_hook_specific_output` — pattern to follow.

---
completed: 2026-04-13
completed_by: cas-40fb (under EPIC cas-37b0)
commit: 7ca15ac
resolution: |
  30 golden-JSON schema tests shipped across two files:
  - crates/cas-core/tests/hook_schema.rs — 18 constructor-level tests
    (PreToolUse/UserPromptSubmit/PostToolUse/SessionStart/SessionEnd/Stop/
    SubagentStop/PreCompact/Notification/SubagentStart/PermissionRequest).
  - cas-cli/tests/hook_schema.rs — 12 handler-level tests piping fake
    HookInput through all 11 handle_* handlers with fresh HookInput fixtures.
  Every test asserts both required-field presence and forbidden-field absence.
  Mutation-tested: temporarily reintroducing the baa540b bug (Stop with
  additionalContext) fails 3 tests loudly with "hookSpecificOutput must be
  absent". 4-persona code review applied 3 safe_auto tweaks in-commit.
  Runs under default cargo test — no feature gates. All 30 green on main.
---
