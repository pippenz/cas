//! `cas default <provider>` — persist the supervisor harness without launching.
//!
//! This is the standalone "persist + exit" surface of the provider-shortcut
//! feature (cas-7f2c).  The `--default` flag on `cas claude` / `cas codex`
//! covers "launch AND persist"; this module covers "persist only" (no launch).

use clap::Args;

use crate::config::{Config, LlmConfig, LlmRoleConfig};
use crate::store::known_repos::host_cas_dir;

/// Arguments for `cas default`
#[derive(Args, Debug, Clone)]
pub struct DefaultArgs {
    /// Provider to persist as the default supervisor harness (`claude` or `codex`)
    pub provider: String,
}

/// Persist `[llm.supervisor] harness = "<provider>"` to `~/.cas/config.toml`
/// and print a one-line confirmation.  Exits 0 on success; non-zero on an
/// unknown provider or a filesystem error.
pub fn execute(args: &DefaultArgs) -> anyhow::Result<()> {
    let provider = validate_provider(&args.provider)?;

    let host_dir = host_cas_dir();
    std::fs::create_dir_all(&host_dir)?;

    let mut config = Config::load(&host_dir).unwrap_or_default();
    set_supervisor_harness(&mut config, provider);
    config.save(&host_dir)?;

    println!("supervisor default set to {provider}");
    Ok(())
}

/// Validate that `s` is "claude" or "codex"; return the normalised lowercase
/// string on success, or a descriptive error on failure.
pub(crate) fn validate_provider(s: &str) -> anyhow::Result<&str> {
    match s.trim() {
        "claude" => Ok("claude"),
        "codex" => Ok("codex"),
        other => anyhow::bail!(
            "Unknown provider '{other}'. Use 'claude' or 'codex'."
        ),
    }
}

/// Write `[llm.supervisor] harness = <provider>` into `config`, creating
/// the `llm` / `supervisor` sections if absent.
pub(crate) fn set_supervisor_harness(config: &mut Config, provider: &str) {
    let llm = config.llm.get_or_insert_with(LlmConfig::default);
    let supervisor = llm.supervisor.get_or_insert_with(LlmRoleConfig::default);
    supervisor.harness = Some(provider.to_string());
}
