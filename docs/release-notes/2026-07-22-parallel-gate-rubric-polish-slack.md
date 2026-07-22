# 2026-07-22 — Parallel gate green + rubric polish (v2.28.2) — #cas-internal drafts

## Post 1 — User

The project's own test gate cried wolf on every run — a handful of tests failed randomly whenever the suite ran at normal speed, so "red" stopped meaning anything and everyone learned to re-run in slow mode. Now the full-speed gate is trustworthy again.

- Test failures now mean something is actually broken, not that two tests bumped into each other.
- The supervisor playbook got a consistency pass: every worker-spawn example now spells out exactly which engine, model, and effort to use, the per-harness copies say the same thing, and message examples are complete enough to copy-paste without errors.

## Post 2 — Dev

The default-parallelism `cargo test` gate failed nondeterministically on every run; root cause fixed, and the routing rubric got its final review-gap remediation.

- `supervisor_push` lifecycle tests guarded `CAS_FACTORY_SESSION` with a module-local `static Mutex` — useless against env mutation from other modules (`assignment_freshness_branch_tests` in update.rs mutated the same var with no lock at all). One panic poisoned the local mutex and cascaded `PoisonError`s through the module. All env-mutating tests now serialize on the process-wide poison-tolerant `test_env_lock()` with a panic-safe RAII guard restoring prior state. Proof: 5 consecutive green default-parallel `cargo test -p cas --lib` runs (2729 tests each) — no test semantics weakened, test-only diff.
- Supervisor rubric: all copyable spawn recipes across the three harness twins are explicit `cli=codex model=gpt-5.6-sol` with tiered effort, `reference.md` twins normalized (transfer lifecycle guidance included everywhere), workflow coordination examples carry `summary=`, and a new builtins guard test (`test_supervisor_rubric_recipes_and_reference_twins_stay_normalized`) locks the invariants — written red-first against the drift it prevents.
