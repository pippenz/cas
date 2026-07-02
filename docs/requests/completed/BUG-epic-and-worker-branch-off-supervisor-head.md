---
from: Petra Stella — Ozer factory session
date: 2026-06-09
priority: P2
resolution: fixed
resolved: 2026-07-02
---

# Epic creation and `spawn_workers` branch off the supervisor's current HEAD, not the configured main branch

## Resolution

Fixed in `cas-8937` on 2026-07-02.

- Epic creation paths now create epic branches from the detected trunk/default branch instead of the supervisor's incidental checkout.
- Default-branch detection now prefers remote/default/trunk refs over the current `HEAD` symref, so local-only factory sessions do not accidentally treat a checked-out feature or epic branch as trunk.
- Initial fork-first workers, dynamic `spawn_workers`, and respawned workers now create worker worktrees from the active epic branch when one exists, otherwise from the detected trunk.
- Regression coverage now proves default-branch detection ignores a checked-out feature branch, epic branch creation uses trunk instead of current `HEAD`, and dynamic worker base selection chooses active epic/trunk instead of supervisor `HEAD`.

In factory mode, both the epic branch (created by `task create` with `task_type=epic`) and worker worktrees (created by `coordination spawn_workers`) are based on **whatever branch the supervisor happens to have checked out**, rather than the repo's configured main/trunk branch. When the supervisor is sitting on an unrelated feature/epic branch, the new epic and all its workers silently inherit that branch's lineage — a contaminated base that is missing recent trunk commits and carries unrelated ones. Nothing in the output surfaces the chosen base, so it's invisible unless the supervisor manually inspects SHAs.

## Affected version

Observed 2026-06-09 on the Ozer factory (`Richards-LLC/ozer-health`). Repo's documented main branch is `staging`.

## Symptom

Supervisor session was checked out on `epic/ai-links-...-cas-2bfd` (a feature epic, tip `16edadc4`, which is **1 ahead / 6 behind `origin/staging`**). The supervisor then:

1. `task create` `task_type=epic` → created `epic/sow-09-...-cas-fa1f`. **Epic branched off `16edadc4`** (the supervisor's current HEAD), not off `origin/staging` (`15683f6f`).
2. `coordination spawn_workers` `isolate=true` → worker worktree `factory/eng-b` came up at **`16edadc4`** as well — based on the supervisor's HEAD, not on the epic branch the worker belongs to.

Net effect: the worker began implementing on a tree **missing 6 staging commits** — including `15683f6f`, the `setPersonPropertiesForFlags` fix in `apps/frontend/plugins/posthog.client.ts` — and **carrying an unrelated commit** (`16edadc4`, an ai-links CODEMAP regen). The worker edited `posthog.client.ts` directly, so its changes were layered on a stale version of the exact file the missing fix had patched. Caught only by manual `git merge-base`/`log` inspection, after the worker had already produced uncommitted work.

## Two distinct problems

1. **Epic base.** `task create epic` should branch the epic off the repo's configured main/PR-base branch (here `staging`), or at minimum warn/confirm when the supervisor's current branch is not that trunk. Silently using the incidental HEAD is a footgun.
2. **Worker base.** `spawn_workers` should base worker worktrees off the **epic branch** the work belongs to (or accept an explicit base ref), not the supervisor's current HEAD. A worker on `factory/<name>` whose merge target is the epic should be cut *from* the epic.

## Impact

- Silent contaminated base → workers build on stale trunk, risking conflicts/regressions (e.g. re-introducing a bug a missing trunk commit already fixed).
- Invisible: neither the epic-create nor the spawn output reports the resolved base branch/SHA.
- Recovery is manual and fiddly: rebuild the epic off `origin/staging`, then `git rebase --onto <new-epic> <old-base> factory/<name>` each in-flight worker, resolving conflicts that only exist because of the bad base.

## Suggested fix

- Base epic branches on the configured main/trunk branch by default (resolve from repo config / default-branch), independent of the supervisor's checkout; warn if that branch can't be determined or differs from HEAD.
- Base worker worktrees on the associated epic branch (or expose a `base`/`from` ref on `spawn_workers`).
- Echo the resolved base branch + SHA in both the epic-create and spawn outputs so the supervisor can catch a wrong base immediately.

## Pre-Fix Operator Mitigation

Supervisor must `git checkout <main>` (e.g. `staging`) and pull before `task create epic`, and ensure workers are spawned from the epic branch; otherwise realign each worker worktree post-spawn (`git -C <worktree> fetch && git rebase --onto <epic> <bad-base>`).

## Repro

1. Check out any non-trunk branch in a factory repo whose main is `staging`.
2. `task create` an epic → observe the new `epic/*` branch points at the current HEAD, not `staging`.
3. `spawn_workers` → observe `factory/<name>` points at the supervisor's HEAD, not the epic branch.
