---
from: Petra Stella Cloud team
date: 2026-04-13
priority: P2
completed: 2026-04-13
completed_by: cas-src (task cas-5860)
resolution: |
  Shipped Option C (runtime self-check) plus install-script hardening and docs.
  - `cas-cli/src/duplicate_check.rs` scans PATH on startup, warns once to stderr when
    multiple `cas` binaries exist with differing mtimes. Gated to TTY; skipped for
    `hook`/`serve`/`factory`; silenced by `CAS_SUPPRESS_DUPLICATE_WARNING=1`; forced
    on in non-TTY contexts by `CAS_WARN_DUPLICATES=1`.
  - `scripts/cas-install.sh` now always installs to `~/.local/bin/cas` (canonical) and
    warns when other `cas` binaries are on PATH.
  - Canonical install path documented in `cas-cli/docs/CONTRIBUTING.md`.
---

# Bug: Multiple `cas` binaries on PATH diverge silently

## Problem

Four `cas` binaries are present on at least one developer machine:

```
/home/pippenz/.local/bin/cas       # first on PATH — what hooks invoke
/usr/bin/cas
/usr/local/bin/cas
/home/pippenz/.cargo/bin/cas
```

Only `~/.local/bin/cas` was updated when shipping the Stop-hook fix (commit `baa540b`). The other three are stale copies from earlier installs (`cargo install`, distro package, `make install`, etc.). They sit there silently rotting.

This becomes a real problem when:

1. **PATH order changes** (different shell, different terminal, different invocation context — e.g., a launchd/systemd service vs. an interactive zsh) — a stale binary suddenly becomes the active one and reintroduces fixed bugs.
2. **A subagent or external script invokes `cas` via absolute path** — they pin to a specific binary that may be months out of date.
3. **Developer onboarding** — new contributor runs `cargo install cas` and ends up with a stale copy alongside the freshly-built one. Confusion when `cas --version` reports something unexpected.

## Repro

```bash
# On affected machine
which -a cas
ls -la /usr/bin/cas /usr/local/bin/cas ~/.cargo/bin/cas ~/.local/bin/cas 2>/dev/null
# Compare mtimes — only the build-target one is current
```

## Proposed fix

Pick **one** canonical install location and provide tooling to keep the rest in sync (or removed):

**Option A — install script.** A `scripts/install.sh` that:
- Builds release
- Installs to a single canonical path (e.g., `~/.local/bin/cas`)
- Detects other `cas` binaries on PATH and either symlinks them to the canonical one or warns loudly with removal instructions

**Option B — Makefile target.**
```bash
make install        # builds + installs to ~/.local/bin
make install-check  # warns about stale duplicates
```

**Option C — runtime self-check.** On startup, `cas` checks `which -a cas` and warns to stderr if duplicates exist with different mtimes. Cheap, no install changes needed, surfaces the problem immediately.

C is the lowest-effort, lowest-risk option and probably the right starting point. A or B can follow once the surface area is known.

## Acceptance criteria

- [ ] Either: an install script/Makefile target that handles duplicate detection, OR a runtime warning when stale duplicates are present on PATH
- [ ] Documented in `README.md` or `CONTRIBUTING.md` so new contributors know the canonical install path
- [ ] No false positives when only one `cas` binary exists (the common case)

## Notes

This was discovered while debugging the Stop-hook schema bug. The fix in `~/cas-src` working tree had been written but never built/installed, and four binaries on PATH made it non-obvious which one was actually serving requests.

---
completed: 2026-04-13
completed_by: cas-5860
commits: 8b0e97d, a10ff25, a26b439
resolution: |
  Runtime self-check implemented per Option C. cas-cli/src/duplicate_check.rs
  scans PATH for `cas` executables, compares mtimes, emits a single-line stderr
  warning when they diverge. Gated by TTY + subcommand allowlist (hook, serve,
  factory, bridge silenced by default). Suppressible via
  CAS_SUPPRESS_DUPLICATE_WARNING=1; force-enabled via CAS_WARN_DUPLICATES=1.
  scripts/cas-install.sh canonicalized to ~/.local/bin. Path documented in
  cas-cli/docs/CONTRIBUTING.md. 15/15 unit tests pass on main.
---
