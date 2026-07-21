//! Cross-harness delivery routing matrix (cas-47b7 — verification half of EPIC
//! cas-ca04; cas-4484 — full 3×3 bidirectional closure for EPIC cas-873a).
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
//!
//! cas-4484 closes the STATIC evidence gap left by cas-474b: Grok-supervisor
//! mixed-worker pairings (and the remaining codex-sup / grok-worker cell) are
//! first-class automated shapes, bringing the matrix to all 9 pairings × 2
//! directions = 18 direction-specific contracts.

use std::str::FromStr;

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
    /// is launched only by a **Claude** supervisor; a Codex- or Grok-supervised
    /// factory runs `teams = None`. This mirrors `FactoryDaemon::teams.is_some()`,
    /// the value `choose_channel` is fed at the call sites.
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

/// Full 3×3 supervisor×worker matrix (Claude, Codex, Grok). Indices 0–5 keep
/// their historical positions so named tests that pin `SHAPES[n]` stay stable;
/// cas-4484 appends the three previously missing cells at indices 6–8.
const SHAPES: [FactoryShape; 9] = [
    // 0. claude-sup → codex-worker: the original bug (codex worker unreachable).
    FactoryShape {
        supervisor: Claude,
        worker: Codex,
        label: "claude-sup / codex-worker",
    },
    // 1. codex-sup → claude-worker: codex supervisor, claude worker.
    FactoryShape {
        supervisor: Codex,
        worker: Claude,
        label: "codex-sup / claude-worker",
    },
    // 2. codex-sup → codex-worker: codex-only factory.
    FactoryShape {
        supervisor: Codex,
        worker: Codex,
        label: "codex-sup / codex-worker",
    },
    // 3. claude-sup → claude-worker: the all-Claude regression baseline.
    FactoryShape {
        supervisor: Claude,
        worker: Claude,
        label: "claude-sup / claude-worker",
    },
    // 4. claude-sup → grok-worker: Grok has no CC Agent-Teams membership
    //    (EPIC cas-8888 delta #4), so it must be PTY-delivered even though
    //    the supervisor is in teams mode — the same shape of bug cas-b68a
    //    fixed for Codex, now proven for Grok too.
    FactoryShape {
        supervisor: Claude,
        worker: Grok,
        label: "claude-sup / grok-worker",
    },
    // 5. grok-sup → grok-worker: grok-only factory (teams never active,
    //    since teams_active() keys on supervisor == Claude).
    FactoryShape {
        supervisor: Grok,
        worker: Grok,
        label: "grok-sup / grok-worker",
    },
    // 6. codex-sup → grok-worker: non-teams factory; both ends PTY (cas-4484).
    FactoryShape {
        supervisor: Codex,
        worker: Grok,
        label: "codex-sup / grok-worker",
    },
    // 7. grok-sup → codex-worker: previously STATIC in cas-474b — Grok
    //    supervisor, Codex worker; teams never active; Codex always framed PTY.
    FactoryShape {
        supervisor: Grok,
        worker: Codex,
        label: "grok-sup / codex-worker",
    },
    // 8. grok-sup → claude-worker: previously STATIC in cas-474b — Grok
    //    supervisor, Claude worker; Claude uses PTY fallback (no teams).
    FactoryShape {
        supervisor: Grok,
        worker: Claude,
        label: "grok-sup / claude-worker",
    },
];

/// Expected routing for one cell, derived from the contract (NOT from the
/// implementation): Codex recipients are always PTY + gated + framed
/// (`CODEX_WORKER_INSTRUCTIONS`/`CODEX_SUPERVISOR_INSTRUCTIONS` explicitly
/// key on the literal "Message from <sender>: " prefix — see
/// `pty_payload_needs_framing`'s doc comment). Grok recipients are always
/// PTY + gated (EPIC cas-8888: no team-transport) but NOT framed — no such
/// prompt convention has been authored for Grok, and its design otherwise
/// mirrors Claude's (native hooks, real TUI textbox), so it behaves like
/// Claude's PTY-fallback case for framing purposes. Claude recipients use
/// the team inbox iff teams are active (no gate, no framing), else fall
/// back to bare PTY (gated, unframed).
fn expected(recipient: SupervisorCli, teams_active: bool) -> (DeliveryChannel, bool, bool) {
    match recipient {
        Codex => (DeliveryChannel::Pty, true, true),
        Grok => (DeliveryChannel::Pty, true, false),
        Claude if teams_active => (DeliveryChannel::TeamsInbox, false, false),
        Claude => (DeliveryChannel::Pty, true, false),
    }
}

/// Assert channel + readiness gate + framing for one matrix cell.
fn assert_cell(shape: FactoryShape, dir: Direction) {
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

/// Exhaustive 3×3 × bidirectional matrix: 9 pairings × 2 directions = 18
/// direction-specific contracts. Proves the two previously STATIC Grok-
/// supervisor mixed-worker rows from cas-474b are automated first-class shapes.
#[test]
fn delivery_matrix_all_combos_both_directions() {
    assert_eq!(
        SHAPES.len(),
        9,
        "matrix must enumerate all Claude/Codex/Grok supervisor×worker pairings"
    );

    // Every (supervisor, worker) pair appears exactly once.
    let mut seen = std::collections::BTreeSet::new();
    for shape in SHAPES {
        let key = (shape.supervisor.as_str(), shape.worker.as_str());
        assert!(
            seen.insert(key),
            "duplicate factory shape in SHAPES: {}",
            shape.label
        );
    }
    for sup in [Claude, Codex, Grok] {
        for worker in [Claude, Codex, Grok] {
            assert!(
                seen.contains(&(sup.as_str(), worker.as_str())),
                "missing factory shape: {}-sup / {}-worker",
                sup.as_str(),
                worker.as_str()
            );
        }
    }

    let mut cells = 0usize;
    for shape in SHAPES {
        for dir in [Direction::Downward, Direction::Upward] {
            assert_cell(shape, dir);
            cells += 1;
        }
    }
    assert_eq!(
        cells, 18,
        "expected 9 pairings × 2 directions = 18 direction-specific contracts"
    );

    // Explicitly pin the two cas-474b STATIC rows so a future SHAPES trim that
    // keeps len==9 via other cells still fails if these drop out.
    let grok_codex = SHAPES
        .iter()
        .find(|s| s.supervisor == Grok && s.worker == Codex)
        .expect("grok-sup / codex-worker must be a first-class matrix shape");
    let grok_claude = SHAPES
        .iter()
        .find(|s| s.supervisor == Grok && s.worker == Claude)
        .expect("grok-sup / claude-worker must be a first-class matrix shape");
    for shape in [*grok_codex, *grok_claude] {
        assert!(
            !shape.teams_active(),
            "[{}] Grok supervisor never launches Agent Teams",
            shape.label
        );
        for dir in [Direction::Downward, Direction::Upward] {
            assert_cell(shape, dir);
        }
    }
}

/// Named focus on the cas-474b STATIC gap: Grok supervisor with Codex or Claude
/// workers — both directions, non-teams, recipient-aware channel/gate/framing.
#[test]
fn grok_supervisor_mixed_workers_both_directions_are_automated() {
    // Downward: grok → codex worker = framed PTY + gate.
    let gc = SHAPES[7];
    assert_eq!(gc.supervisor, Grok);
    assert_eq!(gc.worker, Codex);
    assert!(!gc.teams_active());
    assert_eq!(
        choose_channel(gc.recipient(Direction::Downward), false),
        DeliveryChannel::Pty
    );
    assert!(requires_pty_readiness_gate(Codex, false));
    assert!(pty_payload_needs_framing(Codex));
    // Upward: codex worker → grok supervisor = unframed PTY + gate (Grok, not Codex).
    assert_eq!(
        choose_channel(gc.recipient(Direction::Upward), false),
        DeliveryChannel::Pty
    );
    assert!(requires_pty_readiness_gate(Grok, false));
    assert!(!pty_payload_needs_framing(Grok));

    // Downward: grok → claude worker = bare PTY fallback (no teams).
    let gcl = SHAPES[8];
    assert_eq!(gcl.supervisor, Grok);
    assert_eq!(gcl.worker, Claude);
    assert!(!gcl.teams_active());
    assert_eq!(
        choose_channel(gcl.recipient(Direction::Downward), false),
        DeliveryChannel::Pty
    );
    assert!(requires_pty_readiness_gate(Claude, false));
    assert!(!pty_payload_needs_framing(Claude));
    // Upward: claude worker → grok supervisor = unframed PTY.
    assert_eq!(
        choose_channel(gcl.recipient(Direction::Upward), false),
        DeliveryChannel::Pty
    );
    assert!(!pty_payload_needs_framing(Grok));
}

/// Teams-active vs PTY-fallback for Claude recipients: only a Claude supervisor
/// turns on teams; under Codex/Grok supervisors Claude must fall back to PTY.
#[test]
fn claude_recipient_teams_active_versus_pty_fallback() {
    // Teams path: claude-sup / claude-worker, both directions → inbox.
    let teams_shape = SHAPES[3];
    assert!(teams_shape.teams_active());
    for dir in [Direction::Downward, Direction::Upward] {
        let r = teams_shape.recipient(dir);
        assert_eq!(r, Claude);
        assert_eq!(
            choose_channel(r, true),
            DeliveryChannel::TeamsInbox,
            "teams-active Claude must use TeamsInbox ({dir:?})"
        );
        assert!(!requires_pty_readiness_gate(r, true));
        assert!(!pty_payload_needs_framing(r));
    }

    // PTY-fallback: Claude worker under Codex supervisor (shape 1) and under
    // Grok supervisor (shape 8).
    for shape in [SHAPES[1], SHAPES[8]] {
        assert!(!shape.teams_active(), "{}", shape.label);
        let worker = shape.recipient(Direction::Downward);
        assert_eq!(worker, Claude);
        assert_eq!(
            choose_channel(worker, false),
            DeliveryChannel::Pty,
            "[{}] Claude worker without teams must PTY-fallback",
            shape.label
        );
        assert!(requires_pty_readiness_gate(worker, false));
        assert!(!pty_payload_needs_framing(worker));
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
    assert_eq!(
        choose_channel(recipient, shape.teams_active()),
        DeliveryChannel::Pty
    );
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
    assert_eq!(
        choose_channel(recipient, shape.teams_active()),
        DeliveryChannel::Pty
    );
    assert!(requires_pty_readiness_gate(recipient, shape.teams_active()));
    assert!(pty_payload_needs_framing(recipient));

    // And the cross-harness variant: codex supervisor with a *claude* worker.
    let cross = SHAPES[1]; // codex-sup / claude-worker
    let codex_sup = cross.recipient(Direction::Upward);
    assert_eq!(codex_sup, Codex);
    assert_eq!(
        choose_channel(codex_sup, cross.teams_active()),
        DeliveryChannel::Pty
    );
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
/// grok worker gets PTY (unframed), a claude worker gets the inbox — within the
/// *same* (teams-active) broadcast. This models the per-worker loop in
/// `queue_and_events.rs` (cas-b68a) without needing a live mux.
#[test]
fn all_workers_broadcast_routes_per_recipient_in_mixed_factory() {
    // A claude-supervised (teams active) factory broadcasting to a mixed pool.
    let teams_active = true;
    // Codex member of the pool → PTY + framed.
    assert_eq!(choose_channel(Codex, teams_active), DeliveryChannel::Pty);
    assert!(pty_payload_needs_framing(Codex));
    // Grok member of the pool → PTY, unframed (no team-transport, no codex prefix).
    assert_eq!(choose_channel(Grok, teams_active), DeliveryChannel::Pty);
    assert!(!pty_payload_needs_framing(Grok));
    // Claude member of the same pool → inbox, unframed.
    assert_eq!(
        choose_channel(Claude, teams_active),
        DeliveryChannel::TeamsInbox
    );
    assert!(!pty_payload_needs_framing(Claude));
}

/// Error path: an unsupported/unknown harness string must not parse into a
/// `SupervisorCli` that could silently inherit Claude's TeamsInbox path (or any
/// other channel). Routing only accepts the closed enum; parse rejects unknowns.
#[test]
fn unknown_harness_cannot_silently_inherit_channel() {
    for bad in ["", "gpt", "cursor", "composer", "unknown", "codexx", "claude-code"] {
        let err = SupervisorCli::from_str(bad).expect_err("unknown harness must not parse");
        assert!(
            err.contains("unsupported harness"),
            "expected unsupported-harness error for {bad:?}, got {err:?}"
        );
    }

    // Under teams_active, only Claude may use TeamsInbox — Codex and Grok must
    // never inherit that channel even when the factory is teams-mode.
    assert_eq!(
        choose_channel(Claude, true),
        DeliveryChannel::TeamsInbox,
        "Claude is the sole TeamsInbox recipient under teams"
    );
    for non_claude in [Codex, Grok] {
        assert_eq!(
            choose_channel(non_claude, true),
            DeliveryChannel::Pty,
            "{non_claude:?} must not inherit TeamsInbox when teams_active"
        );
    }
}
