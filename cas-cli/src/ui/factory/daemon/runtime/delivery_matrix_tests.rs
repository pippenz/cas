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
//! The top-level matrix here is a pure regression test over `choose_channel` /
//! `requires_pty_readiness_gate` / `pty_payload_needs_framing` — it never
//! touches a live MCP server, a PTY, or a Teams inbox (per the live-MCP testing
//! convention: fixture-driven in `cargo test`, live behavior smoke-tested
//! manually and documented separately).
//!
//! The appended `supervisor_claude_delivery` submodule (cas-6257, EPIC cas-873a
//! Unit 3) goes one layer deeper: it drives the **real** `TeamsManager` inbox
//! adapter (under an isolated temp `HOME`) and the **real** `SqlitePromptQueueStore`
//! to prove that a normal supervisor→Claude message lands on the durable inbox
//! turn surface with the same bookkeeping as the director-events lane — still no
//! live PTY / MCP server, only on-disk adapters.
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
    assert!(
        shape.teams_active(),
        "claude supervisor implies teams active"
    );
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
///
/// Valid harness spelling for Codex is exactly `"codex"` (`crates/cas-mux/src/harness.rs`).
/// `"codexx"` appears below only as an intentional near-miss typo sentinel — it is
/// never an accepted alias or production identifier.
#[test]
fn unknown_harness_cannot_silently_inherit_channel() {
    // Positive: the sole valid Codex harness spelling.
    assert_eq!(
        SupervisorCli::from_str("codex"),
        Ok(Codex),
        "valid harness spelling is exactly \"codex\""
    );
    // Positive: the other supported harnesses round-trip by their canonical names.
    assert_eq!(SupervisorCli::from_str("claude"), Ok(Claude));
    assert_eq!(SupervisorCli::from_str("grok"), Ok(Grok));

    // Negative: unknown / unsupported strings must not parse.
    for bad in ["", "gpt", "cursor", "composer", "unknown", "claude-code"] {
        let err = SupervisorCli::from_str(bad).expect_err("unknown harness must not parse");
        assert!(
            err.contains("unsupported harness"),
            "expected unsupported-harness error for {bad:?}, got {err:?}"
        );
    }

    // Near-miss sentinel: "codexx" is a typo of "codex", not an alias.
    // Must stay rejected so a silent-accept of the misspelling cannot inherit
    // any channel (including Claude's TeamsInbox under teams_active).
    let near_miss = SupervisorCli::from_str("codexx")
        .expect_err("codexx is an intentional near-miss typo sentinel, not a valid harness");
    assert!(
        near_miss.contains("unsupported harness"),
        "expected unsupported-harness error for near-miss \"codexx\", got {near_miss:?}"
    );

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

/// cas-6257 (EPIC cas-873a Unit 3): end-to-end coverage that a **normal**
/// supervisor coordination message reaches a Claude worker's durable turn
/// surface (the Agent-Teams inbox) with delivery bookkeeping identical to the
/// director-events lane.
///
/// Unlike the routing-primitive matrix above, these drive the **real adapters**:
/// a real [`TeamsManager::write_to_inbox`] (isolated temp `HOME`) and a real
/// [`SqlitePromptQueueStore`] (temp cas dir), applying the real
/// [`classify_queued_delivery`] bookkeeping contract. They mirror the
/// non-urgent, single-Claude-target path of `process_prompt_queue`: peek →
/// deliver over the recipient's channel → mark the row processed **only** after
/// a successful inbox handoff. No live PTY / MCP server is touched.
#[cfg(test)]
mod supervisor_claude_delivery {
    use std::path::Path;

    use cas_mux::SupervisorCli::Claude;
    use cas_store::{PromptQueueStore, SqlitePromptQueueStore};
    use tempfile::TempDir;

    use super::super::delivery::{
        DeliveryChannel, QueuedDeliveryOutcome, choose_channel, classify_queued_delivery,
    };
    use super::super::teams::{DIRECTOR_AGENT_NAME, InboxMessage, TeamsManager};

    /// Run `f` with a real `TeamsManager` under an **isolated, serialized** temp
    /// `HOME`. Delegates HOME isolation to the crate-wide
    /// [`crate::test_support::with_temp_home`] (guarded by the shared
    /// `HOME_MUTEX`), so these tests never race the many other HOME-mutating lib
    /// tests — the failure mode that a private lock would miss. When
    /// `create_inboxes` is true the inbox dir is created (write_to_inbox needs it
    /// to exist); leaving it false drives the inbox-write-failure path
    /// deterministically (missing parent dir). `f` receives the manager, the
    /// resolved inboxes dir, and the session name. The temp HOME is auto-removed.
    fn with_team_session(
        label: &str,
        create_inboxes: bool,
        f: impl FnOnce(&TeamsManager, &Path, &str),
    ) {
        crate::test_support::with_temp_home(|home| {
            let session = format!("cas6257-{label}");
            let teams = TeamsManager::new(&session);
            let inboxes = home
                .join(".claude")
                .join("teams")
                .join(&session)
                .join("inboxes");
            if create_inboxes {
                std::fs::create_dir_all(&inboxes).expect("mk inboxes dir");
            }
            f(&teams, &inboxes, &session);
        });
    }

    fn open_queue(dir: &Path) -> SqlitePromptQueueStore {
        let store = SqlitePromptQueueStore::open(dir).expect("open prompt queue");
        store.init().expect("init prompt queue");
        store
    }

    fn read_inbox(inboxes: &Path, target: &str) -> Vec<InboxMessage> {
        let path = inboxes.join(format!("{target}.json"));
        if !path.exists() {
            return Vec::new();
        }
        let content = std::fs::read_to_string(&path).expect("read inbox");
        serde_json::from_str(&content).expect("parse inbox")
    }

    /// Faithfully mirror the non-urgent, single-Claude-target delivery loop of
    /// `process_prompt_queue`: peek the session's rows, deliver each over the
    /// REAL Claude channel (a teams-active Claude recipient routes to the inbox,
    /// which is exactly what `deliver_to_worker` does), then apply the REAL
    /// bookkeeping contract — `mark_processed` iff the handoff succeeded.
    /// Returns the number of rows marked processed.
    fn drain_claude(
        queue: &SqlitePromptQueueStore,
        teams: &TeamsManager,
        targets: &[&str],
        session: &str,
    ) -> usize {
        // Pin the production routing decision so this harness can't drift from
        // choose_channel: a teams-active Claude recipient uses the inbox.
        assert_eq!(choose_channel(Claude, true), DeliveryChannel::TeamsInbox);

        let prompts = queue
            .peek_for_targets(targets, Some(session), 10)
            .expect("peek");
        let mut marked = 0usize;
        for q in prompts {
            let delivered =
                teams.write_to_inbox(&q.target, &q.source, &q.prompt, q.summary.as_deref(), None);
            // A live Claude teams worker/supervisor has a pane and is a current
            // session member, so the only failure mode here is a failed inbox
            // handoff → Retry (never MarkProcessed).
            match classify_queued_delivery(delivered.is_ok(), true, true) {
                QueuedDeliveryOutcome::MarkProcessed => {
                    queue.mark_processed(q.id).expect("mark processed");
                    marked += 1;
                }
                QueuedDeliveryOutcome::Retry | QueuedDeliveryOutcome::Abandon => {}
            }
        }
        marked
    }

    /// Happy path: a normal supervisor→Claude message lands in the worker's inbox
    /// and its queue row is marked processed.
    #[test]
    fn supervisor_message_reaches_claude_worker_inbox_and_is_marked_processed() {
        let qdir = TempDir::new().unwrap();
        let queue = open_queue(qdir.path());

        with_team_session("happy", true, |teams, inboxes, session| {
            queue
                .enqueue_with_session("supervisor", "swift-fox", "start cas-1234", session)
                .unwrap();

            let marked = drain_claude(&queue, teams, &["swift-fox"], session);
            assert_eq!(marked, 1, "the delivered row must be marked processed");

            let inbox = read_inbox(inboxes, "swift-fox");
            assert_eq!(inbox.len(), 1, "message must land in the worker inbox");
            assert_eq!(inbox[0].from, "supervisor");
            assert_eq!(inbox[0].text, "start cas-1234");

            // Row consumed — no longer pending.
            assert!(
                queue
                    .peek_for_targets(&["swift-fox"], Some(session), 10)
                    .unwrap()
                    .is_empty(),
                "processed row must not be re-peeked"
            );
        });
    }

    /// FIFO: a serial burst of 10 supervisor messages surfaces in enqueue order.
    #[test]
    fn supervisor_message_burst_preserves_fifo_order_into_claude_inbox() {
        let qdir = TempDir::new().unwrap();
        let queue = open_queue(qdir.path());

        with_team_session("fifo", true, |teams, inboxes, session| {
            let expected: Vec<String> = (0..10).map(|i| format!("msg-{i:02}")).collect();
            for text in &expected {
                queue
                    .enqueue_with_session("supervisor", "swift-fox", text, session)
                    .unwrap();
            }

            let marked = drain_claude(&queue, teams, &["swift-fox"], session);
            assert_eq!(marked, 10, "all 10 rows delivered + processed");

            let got: Vec<String> = read_inbox(inboxes, "swift-fox")
                .into_iter()
                .map(|m| m.text)
                .collect();
            assert_eq!(got, expected, "inbox must preserve enqueue (FIFO) order");
        });
    }

    /// Reverse path: 10 worker→supervisor messages reach the supervisor's inbox
    /// (the supervisor is a Claude teammate) in order.
    #[test]
    fn worker_messages_reach_supervisor_inbox_in_order() {
        let qdir = TempDir::new().unwrap();
        let queue = open_queue(qdir.path());

        with_team_session("reverse", true, |teams, inboxes, session| {
            let expected: Vec<String> = (0..10).map(|i| format!("status-{i:02}")).collect();
            for text in &expected {
                queue
                    .enqueue_with_session("swift-fox", "supervisor", text, session)
                    .unwrap();
            }

            let marked = drain_claude(&queue, teams, &["supervisor"], session);
            assert_eq!(marked, 10);

            let got: Vec<String> = read_inbox(inboxes, "supervisor")
                .into_iter()
                .map(|m| m.text)
                .collect();
            assert_eq!(got, expected, "supervisor inbox must be FIFO");
            assert!(
                read_inbox(inboxes, "supervisor")
                    .iter()
                    .all(|m| m.from == "swift-fox"),
                "reverse-path sender must be the worker"
            );
        });
    }

    /// Error path: an inbox write failure (inbox dir unavailable) leaves the row
    /// **retryable** and does NOT advance `processed_at` — matching the durable
    /// director-events lane's at-least-once semantics.
    #[test]
    fn failed_inbox_write_leaves_row_retryable_and_unprocessed() {
        let qdir = TempDir::new().unwrap();
        let queue = open_queue(qdir.path());

        // create_inboxes=false → the inbox directory does not exist, so
        // write_to_inbox's initial `[]` write fails (missing parent).
        with_team_session("error", false, |teams, inboxes, session| {
            queue
                .enqueue_with_session("supervisor", "swift-fox", "please retry me", session)
                .unwrap();

            let marked = drain_claude(&queue, teams, &["swift-fox"], session);
            assert_eq!(marked, 0, "a failed handoff must NOT be marked processed");

            // Nothing landed on disk.
            assert!(
                read_inbox(inboxes, "swift-fox").is_empty(),
                "no message should be persisted on write failure"
            );

            // The row is still pending → retryable on the next tick.
            let still_pending = queue
                .peek_for_targets(&["swift-fox"], Some(session), 10)
                .unwrap();
            assert_eq!(
                still_pending.len(),
                1,
                "unprocessed row must remain peekable (retryable)"
            );
            assert!(
                still_pending[0].processed_at.is_none(),
                "processed_at must not advance on a failed inbox handoff"
            );
        });
    }

    /// Director-events lane stays green: a director-sourced message travels the
    /// identical durable inbox mechanism and lands with `from = "director"`,
    /// proving supervisor and director messages share one seam.
    #[test]
    fn director_message_uses_the_same_durable_inbox_lane() {
        let qdir = TempDir::new().unwrap();
        let queue = open_queue(qdir.path());

        with_team_session("director", true, |teams, inboxes, session| {
            queue
                .enqueue_with_session(
                    DIRECTOR_AGENT_NAME,
                    "swift-fox",
                    "Reminder #7: merge is ready",
                    session,
                )
                .unwrap();

            let marked = drain_claude(&queue, teams, &["swift-fox"], session);
            assert_eq!(marked, 1);

            let inbox = read_inbox(inboxes, "swift-fox");
            assert_eq!(inbox.len(), 1);
            assert_eq!(
                inbox[0].from, DIRECTOR_AGENT_NAME,
                "director messages ride the same inbox lane as supervisor messages"
            );
            assert_eq!(inbox[0].text, "Reminder #7: merge is ready");
        });
    }
}
