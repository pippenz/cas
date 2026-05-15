# Contributing to CAS

Thank you for your interest in CAS.

## Contribution Model

This is the `pippenz/cas` development fork. The upstream `codingagentsystem/cas` is source-available and does not accept PRs — this fork does, on a **case-by-case** basis.

- **Small, high-signal fixes** (bug fixes with clean diffs, warning hygiene, regression repros) are welcome. Send a PR.
- **Larger changes** (new features, refactors, public API changes) — please open an issue first so we can talk about scope before you spend time on a diff.
- **No process** beyond that. We trust contributors to be honest about what they're sending.

## How to Participate

### Report Bugs

Open an [issue](https://github.com/pippenz/cas/issues/new) with:

- A clear description of what happened vs. what you expected
- Steps to reproduce
- Your OS, CAS version (`cas --version`), and relevant configuration

### Suggest Features

Open a [discussion](https://github.com/pippenz/cas/discussions) before writing code, especially for anything that touches the public CLI / MCP surface or the daemon protocol.

### Build from Source

```bash
git clone https://github.com/pippenz/cas.git
cd cas
cargo build --release
```

See the [README](README.md) for full build instructions.

## Code of Conduct

Be respectful and constructive in all interactions.
