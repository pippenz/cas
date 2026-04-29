//! Stub for `cas integrate github <action>`. Full implementation lives in
//! task **cas-f425**.

use super::types::{IntegrationAction, IntegrationOutcome, Platform};

pub fn execute(action: IntegrationAction) -> anyhow::Result<IntegrationOutcome> {
    anyhow::bail!(
        "github {} handler not yet implemented — see task {}",
        action.as_str(),
        Platform::Github.handler_task()
    )
}
