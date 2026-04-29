//! Stub for `cas integrate neon <action>`. Full implementation lives in
//! task **cas-1ece**.

use super::types::{IntegrationAction, IntegrationOutcome, Platform};

pub fn execute(action: IntegrationAction) -> anyhow::Result<IntegrationOutcome> {
    anyhow::bail!(
        "neon {} handler not yet implemented — see task {}",
        action.as_str(),
        Platform::Neon.handler_task()
    )
}
