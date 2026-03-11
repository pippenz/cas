---
name: cas-e2e-testing
description: Run and create E2E tests for CAS. Use when running tests, writing new tests, or debugging test failures.
argument-hint: [test-name or category]
---

# e2e-testing

## Running Tests

```bash
# Unit tests (no API)
cargo test -p cas-cli
cargo test -p cas-cli agent_id::tests -- --nocapture

# Integration tests
cargo test -p cas-cli --test cli_test

# E2E tests with real Claude API (uses Claude subscription)
cargo test -p cas-cli e2e --ignored -- --nocapture
```

## Writing E2E Tests with claude_rs

```rust
use claude_rs::{ClaudeCodeSession, query::Query};

#[tokio::test]
#[ignore] // E2E tests are slow, run explicitly with --ignored
async fn test_my_feature() {
    let session = ClaudeCodeSession::new()
        .print(true)
        .allowed_tools(vec!["mcp__cas__*".to_string()])
        .build()
        .await
        .unwrap();

    let result = session.query(Query::user("Your prompt here")).await.unwrap();
    assert!(result.response.contains("expected"));
}
```

**Key patterns:**
- `.print(true)` - See Claude's output
- `.allowed_tools(vec![...])` - Whitelist specific tools
- `#[ignore]` - Required for E2E tests (run with `--ignored`)

## Test Locations

| Location | Type |
|----------|------|
| `src/**/*.rs` `#[cfg(test)]` | Unit tests |
| `tests/cli_test.rs` | CLI integration |
| `tests/e2e/*.rs` | Real Claude E2E tests |

## Guidelines

1. Unit tests in source files, E2E in `tests/e2e/`
2. E2E tests must be `#[ignore]`
3. Use `tempfile::TempDir` for isolated databases
4. Use `--nocapture` for debugging

## Instructions

/e2e-testing

## Tags

testing, e2e, claude_rs, integration
