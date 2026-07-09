//! Cross-harness delivery routing matrix (cas-47b7 — verification half of EPIC
//! cas-ca04).
//!
//! cas-b68a's own unit tests (in `delivery.rs`) cover the routing *primitives*
//! one argument-pair at a time. This module pins the **end-to-end semantic
//! matrix** the EPIC acceptance criteria are written against: for every
//! `(supervisor_harness, worker_harness)` factory shape and **both directions**
//! of a message (downward `target=worker`, upward `target=supervisor`), the
//! recipient-aware routing must choose the right channel, honor the PTY
//! readiness gate, and frame codex recipients.
//!
//! This is a pure regression test over `choose_channel` /
//! `requires_pty_readiness_gate` / `pty_payload_needs_framing` — it never
//! touches a live MCP server, a PTY, or a Teams inbox (per the live-MCP testing
//! convention: fixture-driven in `cargo test`, live behavior smoke-tested
//! manually and documented separately).
//!
//! The matrix deliberately re-derives the *expected* answer from first
//! principles inside each row so a future change to the routing logic that
//! happens to keep the primitives self-consistent but breaks a real factory
//! shape still trips a red cell here.

use cas_mux::SupervisorCli::{self, Claude, Codex, Grok};

use super::delivery::{
    DeliveryChannel, choose_channel, pty_payload_needs_framing, requires_pty_readiness_gate,
};

/// Which end of the supervisor↔worker link a message is addressed to.
#[derive(Debug, Clone, Copy)]
enum Direction {
    /// `target=<worker>` — supervisor (or director) → worker.
    Downward,
    /// `target=supervisor` — worker → supervisor.
    Upward,
}

/// A single factory shape: who supervises, who works.
#[derive(Debug, Clone, Copy)]
struct FactoryShape {
    supervisor: SupervisorCli,
    worker: SupervisorCli,
    label: &'static str,
}

impl FactoryShape {
    /// Whether native Claude Agent Teams is active for this factory. Teams mode
    /// is launched only by a **Claude** supervisor; a Codex-supervised factory
    /// runs `teams = None`. This mirrors `FactoryDaemon::teams.is_some()`, the
    /// value `choose_channel` is fed at the call sites.
    fn teams_active(&self) -> bool {
        self.supervisor == Claude
    }

    /// The harness of the message *recipient* for a given direction — the only
    /// thing the recipient-aware router keys on.
    fn recipient(&self, dir: Direction) -> SupervisorCli {
        match dir {
            Direction::Downward => self.worker,
            Direction::Upward => self.supervisor,
        }
    }
}

/// The full combo matrix the EPIC enumerates. EPIC cas-8888 (cas-9a31,
/// Phase 1) appended the two Grok combos (indices 4-5) — existing
/// index-based tests below (`SHAPES[0]`..`SHAPES[3]`) are unaffected
/// since the original four keep their positions.
const SHAPES: [FactoryShape; 6] = [
    // 1. claude-sup → codex-worker: the original bug (codex worker unreachable).
    FactoryShape { supervisor: Claude, worker: Codex, label: "claude-sup / codex-worker" },
    // 2. codex-sup → claude-worker: codex supervisor, claude worker.
    FactoryShape { supervisor: Codex, worker: Claude, label: "codex-sup / claude-worker" },
    // 3. codex-sup → codex-worker: codex-only factory.
    FactoryShape { supervisor: Codex, worker: Codex, label: "codex-sup / codex-worker" },
    // 4. claude-sup → claude-worker: the all-Claude regression baseline.
    FactoryShape { supervisor: Claude, worker: Claude, label: "claude-sup / claude-worker" },
    // 5. claude-sup → grok-worker: Grok has no CC Agent-Teams membership
    //    (EPIC cas-8888 delta #4), so it must be PTY-delivered even though
    //    the supervisor is in teams mode — the same shape of bug cas-b68a
    //    fixed for Codex, now proven for Grok too.
    FactoryShape { supervisor: Claude, worker: Grok, label: "claude-sup / grok-worker" },
    // 6. grok-sup → grok-worker: grok-only factory (teams never active,
    //    since teams_active() keys on supervisor == Claude).
    FactoryShape { supervisor: Grok, worker: Grok, label: "grok-sup / grok-worker" },
];

/// Expected routing for one cell, derived from the contract (NOT from the
/// implementation): Codex and Grok recipients are always PTY + gated +
/// framed (EPIC cas-8888: Grok has no team-transport, coordinates the
/// Codex way); Claude recipients use the team inbox iff teams are active
/// (no gate, no framing), else fall back to bare PTY (gated, unframed).
fn expected(recipient: SupervisorCli, teams_active: bool) -> (DeliveryChannel, bool, bool) {
    match recipient {
        Codex | Grok => (DeliveryChannel::Pty, true, true),
        Claude if teams_active => (DeliveryChannel::TeamsInbox, false, false),
        Claude => (DeliveryChannel::Pty, true, false),
    }
}

#[test]
fn delivery_matrix_all_combos_both_directions() {
    for shape in SHAPES {
        for dir in [Direction::Downward, Direction::Upward] {
            let recipient = shape.recipient(dir);
            let teams = shape.teams_active();
            let (want_channel, want_gate, want_framing) = expected(recipient, teams);

            let got_channel = choose_channel(recipient, teams);
            let got_gate = requires_pty_readiness_gate(recipient, teams);
            let got_framing = pty_payload_needs_framing(recipient);

            assert_eq!(
                got_channel, want_channel,
                "[{}] {dir:?}: recipient={recipient:?} teams={teams} \
                 expected channel {want_channel:?}, got {got_channel:?}",
                shape.label
            );
            assert_eq!(
                got_gate, want_gate,
                "[{}] {dir:?}: recipient={recipient:?} teams={teams} \
                 expected readiness-gate {want_gate}, got {got_gate}",
                shape.label
            );
            assert_eq!(
                got_framing, want_framing,
                "[{}] {dir:?}: recipient={recipient:?} teams={teams} \
                 expected framing {want_framing}, got {got_framing}",
                shape.label
            );
        }
    }
}

/// The load-bearing fix: a Codex worker under a Claude (teams) supervisor is
/// PTY-delivered, gated, and framed — never written to a team inbox it cannot
/// read. (EPIC cas-ca04 AC1 / cas-b68a root cause, downward leg.)
#[test]
fn claude_sup_to_codex_worker_is_pty_gated_framed() {
    let shape = SHAPES[0];
    assert!(shape.teams_active(), "claude supervisor implies teams active");
    let recipient = shape.recipient(Direction::Downward);
    assert_eq!(recipient, Codex);
    assert_eq!(choose_channel(recipient, shape.teams_active()), DeliveryChannel::Pty);
    assert!(requires_pty_readiness_gate(recipient, shape.teams_active()));
    assert!(pty_payload_needs_framing(recipient));
}

/// The upward mirror: a Codex *supervisor* must be woken by a worker message
/// over the PTY (teams=None), framed so its prompt recognizes the injected
/// turn. This is the leg that lets a codex supervisor triage worker
/// status/blocker/ready messages. (EPIC cas-ca04 note (a), upward leg.)
#[test]
fn worker_to_codex_supervisor_is_pty_gated_framed() {
    let shape = SHAPES[2]; // codex-sup / codex-worker (codex-only factory)
    assert!(!shape.teams_active(), "codex supervisor implies no teams");
    let recipient = shape.recipient(Direction::Upward);
    assert_eq!(recipient, Codex);
    assert_eq!(choose_channel(recipient, shape.teams_active()), DeliveryChannel::Pty);
    assert!(requires_pty_readiness_gate(recipient, shape.teams_active()));
    assert!(pty_payload_needs_framing(recipient));

    // And the cross-harness variant: codex supervisor with a *claude* worker.
    let cross = SHAPES[1]; // codex-sup / claude-worker
    let codex_sup = cross.recipient(Direction::Upward);
    assert_eq!(codex_sup, Codex);
    assert_eq!(choose_channel(codex_sup, cross.teams_active()), DeliveryChannel::Pty);
    assert!(pty_payload_needs_framing(codex_sup));
}

/// Regression: the all-Claude factory is byte-for-byte unchanged — both
/// directions go through the team inbox with no gate and no framing. (EPIC
/// cas-ca04 AC "claude-sup→claude-worker shown unchanged".)
#[test]
fn all_claude_factory_uses_inbox_both_directions_unchanged() {
    let shape = SHAPES[3];
    assert!(shape.teams_active());
    for dir in [Direction::Downward, Direction::Upward] {
        let recipient = shape.recipient(dir);
        assert_eq!(recipient, Claude);
        assert_eq!(
            choose_channel(recipient, shape.teams_active()),
            DeliveryChannel::TeamsInbox,
            "all-claude {dir:?} must stay on the team inbox"
        );
        assert!(
            !requires_pty_readiness_gate(recipient, shape.teams_active()),
            "inbox writes never need the PTY gate"
        );
        assert!(
            !pty_payload_needs_framing(recipient),
            "claude recipients are never framed"
        );
    }
}

/// `all_workers` fan-out resolves each worker individually, so a broadcast in a
/// mixed-harness factory must route per-recipient: a codex worker gets PTY, a
/// claude worker gets the inbox — within the *same* (teams-active) broadcast.
/// This models the per-worker loop in `queue_and_events.rs` (cas-b68a) without
/// needing a live mux.
#[test]
fn all_workers_broadcast_routes_per_recipient_in_mixed_factory() {
    // A claude-supervised (teams active) factory broadcasting to a mixed pool.
    let teams_active = true;
    // Codex member of the pool → PTY + framed.
    assert_eq!(choose_channel(Codex, teams_active), DeliveryChannel::Pty);
    assert!(pty_payload_needs_framing(Codex));
    // Claude member of the same pool → inbox, unframed.
    assert_eq!(choose_channel(Claude, teams_active), DeliveryChannel::TeamsInbox);
    assert!(!pty_payload_needs_framing(Claude));
}
