//! Recipient-aware message delivery (cas-b68a).
//!
//! Every supervisor→worker (and worker→supervisor / director→agent) message must
//! reach the recipient over a channel the recipient can actually read:
//!
//! - A **Claude** agent in a native Agent-Teams factory reads its inbox files, so
//!   delivery goes through [`TeamsManager::write_to_inbox`].
//! - A **Codex** agent is *not* a member of the Claude team and never polls an
//!   inbox; the only channel it can receive is a direct PTY write
//!   ([`Mux::inject`]) performed by the daemon that holds its PTY master.
//!
//! The historical bug was that every delivery site branched on whether the
//! **supervisor** was in teams mode (`self.teams.is_some()`), not on the
//! **recipient's** harness. A Codex worker under a Claude supervisor therefore had
//! its messages written to an inbox it could never read, and the PTY path — its
//! only viable channel — was never taken. This module centralises the routing
//! decision so it can no longer drift per call site.

use cas_mux::SupervisorCli;

use super::super::FactoryDaemon;

/// The channel a message should be delivered over, decided by the recipient's
/// harness and whether the factory is running native Agent Teams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryChannel {
    /// Direct PTY write via `Mux::inject`.
    Pty,
    /// Claude Agent-Teams inbox file via `TeamsManager::write_to_inbox`.
    TeamsInbox,
}

/// Pure routing decision — the single source of truth for *recipient-aware*
/// delivery. Kept free of `self` so it is exhaustively unit-testable.
///
/// - **Codex** recipients are *always* PTY-delivered: they cannot read a Claude
///   team inbox, so this holds even when the supervisor is in teams mode. This is
///   the load-bearing fix for cas-b68a.
/// - **Claude** recipients use the team inbox when teams are active, and fall back
///   to PTY when they are not (codex-only / non-teams factories).
pub(crate) fn choose_channel(harness: SupervisorCli, teams_active: bool) -> DeliveryChannel {
    match harness {
        SupervisorCli::Codex => DeliveryChannel::Pty,
        SupervisorCli::Claude => {
            if teams_active {
                DeliveryChannel::TeamsInbox
            } else {
                DeliveryChannel::Pty
            }
        }
    }
}

/// Whether a message to `harness` must clear the PTY pane-readiness gate before
/// injection. True exactly when the message is PTY-delivered — i.e. for every
/// Codex recipient (even under teams) and for everyone in a non-teams factory.
///
/// Claude inbox writes are plain file writes with no readline race, so they never
/// need the gate.
pub(crate) fn requires_pty_readiness_gate(harness: SupervisorCli, teams_active: bool) -> bool {
    matches!(choose_channel(harness, teams_active), DeliveryChannel::Pty)
}

/// Whether a PTY-delivered payload must carry the literal `Message from <sender>: `
/// framing. True iff the recipient is **Codex**, independent of teams mode.
///
/// The Codex worker/supervisor prompts (sibling task cas-83c8) key on EXACTLY this
/// prefix to recognise an injected turn as an actionable instruction, and they do
/// so in *every* codex factory — including a codex-only factory (teams=None) where
/// a codex-supervisor→codex-worker message is still PTY-injected. So framing is a
/// property of the recipient's harness, not of teams mode. A Claude recipient
/// reached via the PTY fallback (codex-supervised factory, teams=None) must NOT be
/// framed — it isn't a codex prompt and stays byte-for-byte bare.
pub(crate) fn pty_payload_needs_framing(harness: SupervisorCli) -> bool {
    matches!(harness, SupervisorCli::Codex)
}

/// Prefix PTY-delivered text with literal sender attribution.
///
/// Emits exactly `Message from <sender>: <text>` — no summary interpolation before
/// the colon — because the Codex prompt (cas-83c8) matches on that literal prefix.
/// `source` is the human-readable sender name ("supervisor" or a worker name).
fn attribute_for_pty(source: &str, text: &str) -> String {
    format!("Message from {source}: {text}")
}

impl FactoryDaemon {
    /// Deliver `text` to `target` over the channel the recipient can actually
    /// read, decided by the recipient's harness (cas-b68a).
    ///
    /// `target` may be the logical name `"supervisor"`, the supervisor's pane
    /// name, or a worker name. `source` is the (already team-resolved) sender
    /// name; `summary` is the optional one-line preview carried to Claude inboxes
    /// and used for PTY attribution.
    ///
    /// Returns `Ok(())` on a successful write to the chosen channel.
    pub(crate) async fn deliver_to_worker(
        &self,
        target: &str,
        source: &str,
        text: &str,
        summary: Option<&str>,
    ) -> anyhow::Result<()> {
        // Normalise the target into the two name forms the two channels expect:
        //   - `pane_target`  : the real pane id `Mux::inject` routes on
        //   - `inbox_target` : the logical team member name `write_to_inbox` expects
        let pane_target = if target == "supervisor" {
            self.app.supervisor_name()
        } else {
            target
        };
        let inbox_target = if pane_target == self.app.supervisor_name() {
            "supervisor"
        } else {
            pane_target
        };

        let teams_active = self.teams.is_some();
        let harness = self.app.harness_for(pane_target);

        match choose_channel(harness, teams_active) {
            DeliveryChannel::TeamsInbox => {
                // Safe: TeamsInbox is only chosen when teams_active, i.e. teams.is_some().
                let teams = self
                    .teams
                    .as_ref()
                    .expect("TeamsInbox channel requires active teams");
                teams.write_to_inbox(inbox_target, source, text, summary, None)
            }
            DeliveryChannel::Pty => {
                // Frame based on the RECIPIENT's harness, not teams mode: a Codex
                // recipient always gets the literal `Message from <sender>: ` prefix
                // its prompt keys on (even codex-only, teams=None); a Claude
                // recipient reached via the PTY fallback stays byte-for-byte bare.
                if pty_payload_needs_framing(harness) {
                    let framed = attribute_for_pty(source, text);
                    self.app
                        .mux
                        .inject(pane_target, &framed)
                        .await
                        .map_err(Into::into)
                } else {
                    self.app
                        .mux
                        .inject(pane_target, text)
                        .await
                        .map_err(Into::into)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_recipient_always_pty_even_under_teams() {
        // AC1: a Codex recipient is PTY-delivered even when the supervisor runs
        // native Agent Teams (teams_active = true). This is the core bug fix.
        assert_eq!(
            choose_channel(SupervisorCli::Codex, true),
            DeliveryChannel::Pty
        );
        assert_eq!(
            choose_channel(SupervisorCli::Codex, false),
            DeliveryChannel::Pty
        );
    }

    #[test]
    fn claude_recipient_uses_inbox_when_teams_active_else_pty() {
        // AC3: Claude teammates still go through the team inbox under teams...
        assert_eq!(
            choose_channel(SupervisorCli::Claude, true),
            DeliveryChannel::TeamsInbox
        );
        // ...and fall back to PTY in a non-teams (codex-only / plain PTY) factory.
        assert_eq!(
            choose_channel(SupervisorCli::Claude, false),
            DeliveryChannel::Pty
        );
    }

    #[test]
    fn readiness_gate_required_exactly_for_pty_delivery() {
        // Codex always PTY → always gated (note b: first message was dropped
        // during codex startup because the gate was skipped under teams).
        assert!(requires_pty_readiness_gate(SupervisorCli::Codex, true));
        assert!(requires_pty_readiness_gate(SupervisorCli::Codex, false));
        // Claude under teams → inbox file write, no readline race, no gate.
        assert!(!requires_pty_readiness_gate(SupervisorCli::Claude, true));
        // Claude without teams → PTY → gated.
        assert!(requires_pty_readiness_gate(SupervisorCli::Claude, false));
    }

    #[test]
    fn pty_framing_keys_on_codex_recipient_not_teams_mode() {
        // Codex recipient → framed in EVERY factory (incl. codex-only / teams=None),
        // because the codex prompt (cas-83c8) keys on the literal prefix.
        assert!(pty_payload_needs_framing(SupervisorCli::Codex));
        // Claude recipient via PTY fallback (codex-supervised, teams=None) → bare.
        assert!(!pty_payload_needs_framing(SupervisorCli::Claude));
    }

    #[test]
    fn attribution_uses_literal_sender_prefix() {
        // Exactly `Message from <sender>: <text>` — the string the codex prompt
        // matches on. No summary interpolation before the colon.
        assert_eq!(
            attribute_for_pty("supervisor", "do the thing"),
            "Message from supervisor: do the thing"
        );
        assert_eq!(
            attribute_for_pty("worker-3", "start cas-1234"),
            "Message from worker-3: start cas-1234"
        );
    }
}
