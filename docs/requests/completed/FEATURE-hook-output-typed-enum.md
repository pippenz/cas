---
from: Petra Stella Cloud team
date: 2026-04-13
priority: P2
---

# Feature: Make invalid hook output unrepresentable via per-event enum

## Problem

`HookSpecificOutput` in `crates/cas-core/src/hooks/types.rs:103-126` is a flat struct with all-optional fields covering every event type:

```rust
pub struct HookSpecificOutput {
    pub hook_event_name: String,
    pub additional_context: Option<String>,        // legal only for UserPromptSubmit, PostToolUse
    pub permission_decision: Option<String>,       // legal only for PreToolUse
    pub permission_decision_reason: Option<String>,
    pub updated_input: Option<serde_json::Value>,  // legal only for PreToolUse
}
```

The compiler will happily let you build `{ hook_event_name: "Stop", additional_context: Some(...) }` — which Claude Code rejects with `JSON validation failed: Invalid input`, throwing away the entire hook output.

We just shipped fixes for two such bugs in commit `baa540b` (Stop-hook codemap reminder + loop iteration prompt). The constructor helpers (`with_context`, `with_system_context`) are guarded only by docstrings — no compile-time enforcement.

## Why this matters

This bug class has cost real user-visible context twice now:

1. **stop_flow.rs codemap reminder** — invalid `additionalContext` on Stop dropped the CODEMAP staleness notice for an unknown duration.
2. **session_stop/mod.rs `handle_loop_iteration`** — same shape, would have dropped the loop iteration prompt the next time `/loop` ran.

The second one was discovered only because we audited after the first. There's no reason a third instance won't appear: the type permits it, no test catches it.

## Proposed solution

Replace the flat struct with an enum keyed by event:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "hookEventName")]
pub enum HookSpecificOutput {
    PreToolUse {
        #[serde(skip_serializing_if = "Option::is_none", rename = "permissionDecision")]
        permission_decision: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "permissionDecisionReason")]
        permission_decision_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "updatedInput")]
        updated_input: Option<serde_json::Value>,
    },
    UserPromptSubmit {
        #[serde(rename = "additionalContext")]
        additional_context: String,
    },
    PostToolUse {
        #[serde(skip_serializing_if = "Option::is_none", rename = "additionalContext")]
        additional_context: Option<String>,
    },
    // No variants for Stop, SubagentStop, PreCompact, SessionEnd —
    // those events MUST use systemMessage, never hookSpecificOutput.
}
```

Then `with_context(event_name: &str, ...)` becomes `with_pretooluse_context(...)`, `with_userpromptsubmit_context(...)`, etc. — or better, drop the string-keyed helpers entirely and let callers construct the variant directly.

## Acceptance criteria

- [ ] `HookSpecificOutput` is an enum with exactly the variants for events Claude Code's schema allows
- [ ] Constructing `HookSpecificOutput` for Stop/SubagentStop/PreCompact/SessionEnd is a compile error (or simply impossible — no variant exists)
- [ ] All existing call sites updated; `cargo build --release -p cas` passes
- [ ] All hook tests pass (`cargo test -p cas-core --lib hooks`)
- [ ] `with_context` / `with_system_context` helpers either deleted or refactored to take typed event variants

## Related

- Companion request: `FEATURE-golden-json-hook-tests.md` (runtime regression net while the type-level fix lands)
- Bug-class history: commit `baa540b` (stop_flow.rs + handle_loop_iteration), commit `dcd99bc` (regression test)

---
completed: 2026-04-13
completed_by: cas-e55b (under EPIC cas-37b0)
commit: df992a5
resolution: |
  HookSpecificOutput converted to #[serde(tag = "hookEventName")] enum with
  five variants: PreToolUse, UserPromptSubmit, PostToolUse, SessionStart,
  PermissionRequest. Stop/SubagentStop/PreCompact/SessionEnd construction is
  now unrepresentable at the type level (no variant exists). String-keyed
  builders removed; replaced with typed: with_pre_tool_permission,
  with_user_prompt_context, with_post_tool_context, with_session_start_context,
  with_permission_request. 14 call sites migrated across 8 files. Byte-identical
  wire shape preserved via skip_serializing_if. 4 compile_fail doctests
  guarantee invalid events stay invalid. 5-persona code review caught a real
  P1 (wildcard match contradicting type-system claim) applied in-commit.
  Scope adjusted mid-task: spec said "exactly three variants" but production
  used five valid events; worker audited and expanded scope with supervisor
  approval. cas-82a2 follow-up folded in naturally.
---
