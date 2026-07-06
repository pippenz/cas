# Slack draft — worker close-flow + first-contact guidance (2026-07-06)

Channel: #cas-internal (`C0B44GUKDK2`) — two distinct top-level posts.

---

## Post 1 — User

**Finishing a factory job used to mean slamming into a cryptic "merge required" wall — sometimes three or four times per job — and occasionally the job would "solve" it by stamping its own work as verified. Now the finish-line rules are written down, and self-stamping is explicitly banned.**

- Jobs now know exactly how to land their work before declaring done: push, get it merged the right way for the branch setup, then close — instead of rediscovering the dance by trial and error every time.
- The shortcut where a job marks itself complete and vouches for its own quality is now called out as forbidden, so "verified" actually means someone verified it.
- New jobs introduce themselves correctly on the first try — a set of first-message stumbles that previously hit essentially every job is now covered by the guide.
- The pre-finish checklist speaks JavaScript/TypeScript now, not just Rust — matching the projects most people actually run.

---

## Post 2 — Dev

**We mined 28 real worker sessions across two downstream projects and found the #1 close failure — the merge-state guard — had zero guidance in the worker guide, plus a 100%-hit-rate cluster of first-contact protocol errors. All of it is now documented, with a regression test pinning the guidance in place.**

- Evidence: the `MERGE REQUIRED` merge-state rejection fired in ~9 of 15 sessions in one project (up to 4× per session) and 5 of 17 closes in the other. Workers reinvented push→merge→retry from the raw error text each time, and the friction normalized a bypass: `status=closed` via raw update plus a hand-written verification record with self-assigned confidence.
- New "Check 0 — merge state" section in the pre-close gate and a full `MERGE REQUIRED` recovery section: epic-parent branches get push-then-ask-to-merge (never a PR against `epic/*` — the ref is local-only), main-parent branches get the PR flow, squash-merge SHA-drift gets report-don't-retry. The verification-forging bypass is explicitly banned in both places.
- First-contact fixes: messaging target is the literal string `supervisor` (100% of sampled Codex workers used the supervisor's real name and got rejected), `summary` + `message` both required, the built-in message tool ban now explicitly covers the spawn ready-ping, and idle-at-spawn behavior is defined (one ready message, then wait).
- Close-gate checks de-Rusted: repo-wide symbol search instead of `src/`-rooted, a pnpm/turbo blast-radius rule mirroring the cargo workspace-scope table, and shortcut-marker triage scoped to changed lines so legit hits (UI placeholders, untouched comments) stop causing grind.
- Runtime hardening of the verification-record API against self-attestation is tracked as a follow-up; the guide-level ban ships now.
