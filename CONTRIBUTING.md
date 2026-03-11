# Contributing to CAS

Thank you for your interest in CAS! This document explains how you can participate in the project.

## Contribution Model

CAS is **source-available** under the MIT license. The code is public for transparency, learning, and self-hosting — but we do not accept pull requests at this time.

This is similar to projects like SQLite and Litestream, where a small team maintains the codebase to ensure quality and coherence.

## How to Participate

### Report Bugs

Found a bug? Please [open an issue](https://github.com/codingagentsystem/cas/issues/new) with:

- A clear description of what happened vs. what you expected
- Steps to reproduce the issue
- Your OS, CAS version (`cas --version`), and relevant configuration

### Suggest Features

Have an idea? Start a conversation in [Discussions](https://github.com/codingagentsystem/cas/discussions):

- **Ideas** — Feature requests and suggestions
- **Q&A** — Questions about usage or architecture
- **Show and Tell** — Share what you've built with CAS

### Build from Source

You're welcome to build, modify, and run CAS locally:

```bash
git clone https://github.com/codingagentsystem/cas.git
cd cas
cargo build --release
```

See the [README](README.md) for full build instructions.

## Why No PRs?

CAS is built by a small team with a strong opinion on architecture and code quality. Accepting external PRs introduces coordination overhead that slows us down at this stage. We revisit this policy periodically.

Your bug reports and feature ideas still directly shape the project — many features started as community suggestions.

## Code of Conduct

Be respectful and constructive in all interactions. We're here to build great tools for AI-assisted development.
