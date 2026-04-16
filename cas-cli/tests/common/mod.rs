//! Shared fixtures for `cas-cli/tests/` integration tests.
//!
//! Cargo compiles each `tests/*.rs` file as its own binary. Common
//! fixtures live here and are pulled in via `mod common;` at the top
//! of each test file that needs them. This is the canonical place
//! for test-UUIDs and builder-style config/cli helpers so a rotation
//! is a single-file edit instead of a multi-file grep.

use cas::cli::Cli;
use cas::cloud::CloudConfig;

/// Fixture team UUID used across every team-sync / team-memories test.
///
/// Kept here (not in `cas::store::share_policy::TEST_TEAM_UUID`) so the
/// integration-test binaries don't have to reach into crate-private
/// test modules — the production crate gates its copy behind
/// `#[cfg(test)]` for in-crate unit tests only.
pub const TEST_TEAM: &str = "550e8400-e29b-41d4-a716-446655440000";

/// Build a `Cli` configured for JSON output — the shape used by every
/// integration test that routes through `execute_*` helpers which
/// check `cli.json` before writing human output.
#[allow(dead_code)] // used by subset of test files; cargo warns per-binary
pub fn make_cli_json() -> Cli {
    Cli {
        json: true,
        full: false,
        verbose: false,
        command: None,
    }
}

/// Build a `CloudConfig` pointed at `endpoint` with a valid test
/// token and the shared `TEST_TEAM` already configured via
/// `set_team`. Matches what `cas login + cas cloud team set` would
/// leave on disk.
#[allow(dead_code)]
pub fn make_cloud_config(endpoint: impl Into<String>) -> CloudConfig {
    let mut cfg = CloudConfig::default();
    cfg.endpoint = endpoint.into();
    cfg.token = Some("test-token".to_string());
    cfg.set_team(TEST_TEAM, "test-team");
    cfg
}
