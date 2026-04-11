# Cloud fixes cross-reference (cas-2eb3 epic)

Fixes that close the cross-project contamination vector (`epic cas-2eb3`)
span two repos. This doc is the audit trail from the cas-src side.

## Context

Petra Stella Cloud sync had three interacting bugs that let one project's
data leak into another's local store on pull and get re-uploaded to the
wrong project on push:

1. **Client-side**: `get_project_canonical_id()` could return `None` for
   projects at filesystem root, and the callers in `cloud/syncer/pull.rs`
   silently dropped the `project_id` query param when `None` was returned.
2. **Server-side pull**: When a request omitted `project_id`, the route
   simply skipped the filter clause and returned *every entity for the
   user*, ignoring which project it actually came from.
3. **Server-side push**: The `onConflictDoUpdate` target key was
   `(userId, entityType, id)` with no `projectId` component, so two
   projects pushing the same entity id would overwrite each other.

Fixes were shipped in three tasks (cas-c19e + cas-bd2f + cas-0bdc). Work
landed in two repos because the fix spans the client (cas-cli, Rust) and
the server (petra-stella-cloud, TypeScript + drizzle).

## cas-c19e — client-side fallback for filesystem-root projects

- **Task**: cas-c19e (closed 2026-04-11)
- **Repo**: cas-src
- **Commit on main**: 859391c (cherry-picked from factory/smooth-heron-69 @ 458093a)
- **File touched**: `cas-cli/src/cloud/config.rs`

Adds a deterministic sha256 path-hash fallback (`local:<16 hex chars>`)
that fires only when folder-name derivation fails. Every valid CAS project
now has a stable, unique project_id even if it lives at filesystem root.

**Note**: pre-existing `OnceLock<Option<String>>` caches `None` forever on
the first failed resolution. This was surfaced during code review and
filed as **cas-2c77 (P1)** — it must be fixed before the cas-2eb3 epic
can be claimed fully closed.

## cas-bd2f — server-side pull scope when project_id is absent

- **Task**: cas-bd2f (closed 2026-04-11)
- **Repo**: petra-stella-cloud
- **Branch**: `fix/cas-bd2f-0bdc-cloud-scope` (NOT main — local-only, not pushed)
- **Commit**: c4b98c6
- **Files touched**: `app/api/sync/pull/route.ts`, `app/api/teams/[teamId]/sync/pull/route.ts`

When a pull request arrives without `project_id`, the route now adds
`isNull(syncEntities.projectId)` to the WHERE clause so the server returns
only rows with NULL project_id (not every row for the user). Closes the
"absent = returns everything" contamination vector.

Complements an earlier partial fix at `85e0dc4` (March 24, on
`epic/petra-stella-cloud-post-launch-improvements-cas-137d`, never
merged) which added the `project_id` query param plumbing but did not
fix the absent-fallthrough. The two fixes are complementary, not
duplicate.

## cas-0bdc — server-side push conflict key includes project_id

- **Task**: cas-0bdc (closed 2026-04-11)
- **Repo**: petra-stella-cloud
- **Branch**: `fix/cas-bd2f-0bdc-cloud-scope` (NOT main)
- **Commits**: 8637c29 (personal push), 25b32f8 (team push mirror), c4f8d14 (team push auto-register gated on actual upsert)
- **Files touched**: `app/api/sync/push/route.ts`, `app/api/teams/[teamId]/sync/push/route.ts`

Chose "option B-plus": removed `projectId` from the conflict-update SET
clause and added a `setWhere` guard, so a push against an existing row
with a different project_id is a silent no-op rather than a cross-project
overwrite. No drizzle schema migration needed (no PK widen). If the
"same entity id in different projects" capability is later desired, it's
a separate PK-widen task.

**NOT introduced**: `NULLS NOT DISTINCT` on the unique index, because the
existing Postgres default (NULLs are distinct) is actually safer during
the backfill period — legacy NULL-project rows don't collide with
themselves.

## Downstream follow-up bugs (filed under cas-2eb3)

Code review during this work surfaced three additional P1 bugs that must
ship before cas-2eb3 can be claimed fully resolved:

1. **cas-2c77** — OnceLock caches None permanently, defeating the cas-c19e
   fallback if the first call fires before `find_cas_root()` can resolve.
   cas-cli/Rust fix.
2. **cas-d656** — Server push must reject NULL `project_canonical_id` at
   handler level. Without this, a buggy or malicious client can still
   create NULL-project rows on the server. petra-stella-cloud/TypeScript
   fix.
3. **cas-f645** — Client `push_batch` calls `mark_synced` on any HTTP 200
   without inspecting the response body. After cas-0bdc, the server
   silently skips cross-project conflicts → the client believes the push
   succeeded → silent data loss on next fresh-machine clone. Both client
   and server-side changes needed.

All three are tracked under cas-2eb3 and remain open.

## REQUIRES HUMAN before cloud fixes deploy

The three petra-stella-cloud commits are **local only** on a feature
branch. They have NOT been pushed and have NOT been deployed to Vercel.
Before they take effect in production:

1. **Review the feature branch**: `git -C ~/Petrastella/petra-stella-cloud log fix/cas-bd2f-0bdc-cloud-scope --oneline`
2. **Run the test suite**: `pnpm test` in the petra-stella-cloud repo
3. **Merge to main**: typical git flow — merge or rebase the feature branch
4. **Push to origin**: triggers Vercel auto-deploy. Watch the deploy logs; the fix-pack changes behavior on pull/push routes so unit tests are critical.
5. **Verify post-deploy**: a single `cas cloud push --dry-run` should show a reduced backlog (cas-2eb3 vector closed), and an end-to-end push against a test project should NOT contaminate another project's pull.

The three follow-up bugs (cas-2c77, cas-d656, cas-f645) should be
scheduled before the initial push to origin — they're cheap to fix and
shipping them together closes the epic fully.

## Protocol note

The worker (smooth-heron-69) originally committed the three petra-stella-cloud
fixes directly to `main` rather than a feature branch. Supervisor moved
the commits to `fix/cas-bd2f-0bdc-cloud-scope` and reset
petra-stella-cloud/main back to `999ebd9` (pre-existing local-only commit
for event archival cron) before any push could happen. The commits are
preserved verbatim on the feature branch with identical hashes.
