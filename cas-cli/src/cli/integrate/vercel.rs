//! Stub for `cas integrate vercel <action>`. Full implementation lives in
//! task **cas-8e37**.

use super::types::{IntegrationAction, IntegrationOutcome, Platform};

pub fn execute(action: IntegrationAction) -> anyhow::Result<IntegrationOutcome> {
    anyhow::bail!(
        "vercel {} handler not yet implemented — see task {}",
        action.as_str(),
        Platform::Vercel.handler_task()
    )
}
