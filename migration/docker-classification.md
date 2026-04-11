# Phase 2a: Docker Compose Volume Classification

**Task**: cas-4333 (epic cas-28d4 — Petrastella → Hetzner migration)
**Worker**: mighty-viper-52
**Date**: 2026-04-11
**Scope**: 9 projects with `docker-compose.yml` stacks
**Mode**: read-only static analysis (docker daemon not running on this host)

---

## Headline finding (read this first)

**All 9 projects share the same rocketship-template stack** (postgres:15-alpine + redis:7-alpine + bind-mount backend + bind-mount frontend). Each project declares exactly **2 named volumes**: `postgres_data` and `redis_data`. **Total volumes inventoried: 18.**

**Every one of those 18 volumes is classified `empty_or_unknown` → recommendation `skip`** for three converging reasons:

1. **Backends do not use the local docker Postgres.** Every `apps/backend/.env` overrides `DATABASE_URL` with a Neon (managed cloud Postgres) endpoint. The compose-default `postgresql://postgres:postgres@postgres:5432/...` is never reached at runtime. Evidence: per-project `.env` host fields tabulated below.
2. **Backends do not use the local docker Redis.** No project's `apps/backend/.env` defines a `REDIS_URL` / `REDIS_HOST` pointing at the docker redis service. One project (`domdms`) defines an `UPSTASH_REDIS_REST_URL` (cloud Upstash) — every other project has no Redis configuration at all. The local `redis_data` volumes have nothing reading or writing them from app code.
3. **The docker host currently holds zero project volumes.** `sudo ls /var/lib/docker/volumes/` returns only `backingFsBlockDev` and `metadata.db` — no project-prefixed volume directories exist. Combined with `systemctl is-active docker → inactive` and the ext4 timestamps (`/var/lib/docker` last touched 2026-03-18), the stacks are not currently up and have not been recently. There is no live data on disk to preserve.

**Bottom-line recommendation**: Phase 5 should `docker compose up -d postgres redis` on the Hetzner server with **empty initial volumes** for any project where the supervisor still wants the local stack alive. No tarball transfers required for any volume. **Furthermore, the supervisor should consider whether the docker-compose stacks need to ship to Hetzner at all** — see "Risk callouts" below.

---

## Summary table

| Project              | Stack services                          | Volume count | Recommendation summary | Top concern                                                            |
|----------------------|-----------------------------------------|--------------|------------------------|------------------------------------------------------------------------|
| pulse-card           | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon; volumes empty on host; demo seed exists if needed      |
| gabber-studio        | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon; volumes empty on host; no seed script                  |
| fixyrs               | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon; volumes empty on host; no seed script                  |
| homeschool-whisper   | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon; volumes empty on host; no seed script                  |
| domdms               | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon, redis → Upstash; volumes empty on host                 |
| prospect_path        | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon; vector-search seed script is *maintenance*, not bootstrap |
| pantheon             | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon; volumes empty on host; no seed script                  |
| spaceship_template   | postgres, backend, frontend, redis      | 2            | 2 skip                 | Bare template, zero prisma migrations, almost certainly never run      |
| abundant-mines       | postgres, backend, frontend, redis      | 2            | 2 skip                 | Backend → Neon; volumes empty on host; no seed script                  |
| **Total**            |                                         | **18**       | **18 skip**            |                                                                        |

---

## Per-project sections

The 9 stacks are byte-equivalent except for line endings and one container-name prefix (`domdms-*` vs `rocketship-*`). The schema is the same; the per-project differences worth surfacing are: prisma migration count, presence of seed scripts, and the actual runtime DB target. Common stack details are stated once below and then referenced.

### Common stack (applies to all 9 projects)

**File**: `<project>/docker-compose.yml` (lines 1–82, single file, no override files found)

**Services**:
| Service  | Image              | Mount                                       | Type                |
|----------|--------------------|---------------------------------------------|---------------------|
| postgres | `postgres:15-alpine` | `postgres_data:/var/lib/postgresql/data`   | Named volume        |
| backend  | (built locally)    | `./apps/backend:/app`, `/app/node_modules` | Bind mount + anon   |
| frontend | (built locally)    | `./apps/frontend:/app`, `/app/node_modules`| Bind mount + anon   |
| redis    | `redis:7-alpine`   | `redis_data:/data`                         | Named volume        |

**Named volumes (declared at compose top-level)**: `postgres_data`, `redis_data`. With docker compose's default project-name behavior these resolve to `<project>_postgres_data` and `<project>_redis_data` on the host.

**Bind mounts (out of scope per task — not "named volumes")**: `./apps/backend`, `./apps/frontend` (source code), and anonymous `/app/node_modules` volumes (rebuilt on container start).

**Tmpfs mounts**: none.

**Compose-default `DATABASE_URL`** (compose `backend.environment`, line 32): `postgresql //<DEV_USER>:<DEV_PW>@postgres:5432/<db>?sslmode=disable` (literal value uses the docker postgres image default; redacted here to satisfy secrets-grep — see compose file lines 9–11 for the actual dev default, which is the upstream postgres image's documented default credential and is already public in git). **This default is overridden** in every project's `apps/backend/.env` (see per-project rows below).

---

### 1. pulse-card

- **Compose**: `pulse-card/docker-compose.yml` (default rocketship template)
- **Backend runtime DB target** (`apps/backend/.env`, `DATABASE_URL` host field): `ep-restless-salad-adlu099v.c-2.us-east-1.aws.neon.tech/neondb` → **Neon, not local docker postgres**
- **Prisma migrations**: 7 (range `20250101000000_snapshot_system` … `20260410180000_add_social_post_unique_constraint`)
- **Seed scripts**:
  - `apps/backend/scripts/seed-admin-users.ts` — bootstraps Firebase admin users from env var `ADMIN_SEED_EMAILS`
  - `apps/backend/scripts/seed-demo-data.ts` — generates a hardcoded demo dataset (CAMPAIGNS array with `Summer Glow Campaign`, `Product Launch Q2`, `Wellness Wednesday Series`, etc.; uses `daniel@petrastella.io` as default test user)

#### Volumes
| Volume                       | Role            | Classification     | Evidence                                                                                                                                                                                                                                              | Recommendation                                                                                                                  |
|------------------------------|-----------------|--------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------|
| `pulse-card_postgres_data`   | App database    | `empty_or_unknown` | Backend `.env` `DATABASE_URL` points at Neon, not the docker postgres service; `/var/lib/docker/volumes/` contains no project volumes; docker daemon inactive. The local docker postgres is unreached at runtime.                                    | **`skip`** — let the server stack initialize an empty postgres on first `up`. If demo data is wanted, run `tsx scripts/seed-demo-data.ts` against Neon (or against a local docker postgres only if you choose to switch back to local). |
| `pulse-card_redis_data`      | Cache (per compose comment line 66 "for caching and sessions") | `empty_or_unknown` | No `REDIS_URL` / `REDIS_HOST` in `apps/backend/.env`; redis service has no consumer in app config; no host volume exists.                                                                                                                            | **`skip`** — cache, regenerable by definition.                                                                                 |

---

### 2. gabber-studio

- **Compose**: `gabber-studio/docker-compose.yml` (default rocketship template; only difference from pulse-card is line endings)
- **Backend runtime DB target**: `ep-old-glade-adr9m9b4-pooler.c-2.us-east-1.aws.neon.tech/neondb` → **Neon**
- **Prisma migrations**: 20+ (most recent: `20260211_add_knowledge_base_tables.sql`, includes content-pipeline + analytics indexes — high development activity)
- **Seed scripts**: none in `apps/backend/scripts/`, none in `apps/backend/prisma/seed*`, no `db:seed` npm script in `package.json`

#### Volumes
| Volume                            | Role | Classification     | Evidence                                                                                                                       | Recommendation |
|-----------------------------------|------|--------------------|--------------------------------------------------------------------------------------------------------------------------------|----------------|
| `gabber-studio_postgres_data`     | App database  | `empty_or_unknown` | Backend `.env` → Neon. No host volume on this machine. If a different machine ever populated this volume, that data is not present here.    | **`skip`** — stack initializes empty on Hetzner. |
| `gabber-studio_redis_data`        | Cache         | `empty_or_unknown` | No Redis client config in `apps/backend/.env`. Volume unused.                                                                 | **`skip`**     |

---

### 3. fixyrs

- **Compose**: `fixyrs/docker-compose.yml` (default rocketship template)
- **Backend runtime DB target**: `ep-tiny-wind-a4pksdsa.us-east-1.aws.neon.tech/neondb` → **Neon**
- **Prisma migrations**: 1 (`20251024140711_init`) — newer / less active project
- **Seed scripts**: none

#### Volumes
| Volume                  | Role | Classification     | Evidence                                                                                                            | Recommendation |
|-------------------------|------|--------------------|---------------------------------------------------------------------------------------------------------------------|----------------|
| `fixyrs_postgres_data`  | App database | `empty_or_unknown` | Backend `.env` → Neon. Single init migration suggests this project has barely moved beyond template state locally. | **`skip`**     |
| `fixyrs_redis_data`     | Cache        | `empty_or_unknown` | No Redis client config. Unused.                                                                                    | **`skip`**     |

---

### 4. homeschool-whisper

- **Compose**: `homeschool-whisper/docker-compose.yml` (default rocketship template)
- **Backend runtime DB target**: `ep-withered-wind-ad7gafto-pooler.c-2.us-east-1.aws.neon.tech/neondb` → **Neon**
- **Prisma migrations**: 1 (`20260312172255_init`) — newer
- **Seed scripts**: none

#### Volumes
| Volume                           | Role | Classification     | Evidence                                                                            | Recommendation |
|----------------------------------|------|--------------------|-------------------------------------------------------------------------------------|----------------|
| `homeschool-whisper_postgres_data` | App database | `empty_or_unknown` | Backend `.env` → Neon. Single init migration. No host volume.                       | **`skip`**     |
| `homeschool-whisper_redis_data`    | Cache        | `empty_or_unknown` | No Redis client config. Unused.                                                    | **`skip`**     |

---

### 5. domdms

- **Compose**: `domdms/docker-compose.yml` (rocketship template, container names prefixed `domdms-` instead of `rocketship-`; same volume structure)
- **Backend runtime DB target**: `ep-lingering-scene-ae14a7z6-pooler.c-2.us-east-2.aws.neon.tech/neondb` → **Neon**
- **Cloud Redis**: `apps/backend/.env` defines `UPSTASH_REDIS_REST_URL=https://workable-ocelot-82611.upstash.io` (Upstash REST API; the local docker redis is bypassed)
- **Prisma migrations**: 20+ (very active — includes lead-message infrastructure, brand voice, validation library)
- **Seed scripts**:
  - `apps/backend/scripts/seed-lead-messages.js` — populates a lead-message library by stage (`opening`, `discovery`, `qualifying`, …) with sample message templates. Uses `prisma` directly. Idempotent inserts of fixture data.

#### Volumes
| Volume                  | Role | Classification     | Evidence                                                                                                                                              | Recommendation                                                                                                  |
|-------------------------|------|--------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------|
| `domdms_postgres_data`  | App database | `empty_or_unknown` | Backend `.env` → Neon. No host volume. Real domdms data lives in Neon, not in any docker volume.                                                      | **`skip`** — if a local fixture DB is desired in the future, run `node scripts/seed-lead-messages.js` against it. |
| `domdms_redis_data`     | Cache        | `empty_or_unknown` | App config uses **Upstash** (cloud REST API), not the local docker redis. The compose `redis` service is dead infrastructure.                          | **`skip`**     |

---

### 6. prospect_path

- **Compose**: `prospect_path/docker-compose.yml` (default rocketship template)
- **Backend runtime DB target**: `ep-muddy-heart-ad4e8we7-pooler.c-2.us-east-1.aws.neon.tech/neondb` → **Neon**
- **Prisma migrations**: 20+ (athlete-profile system, vector search, position archetypes — feature-rich)
- **Seed scripts**:
  - `apps/backend/prisma/seed-vector-search.ts` — **NOT a fresh-DB bootstrap.** Header docstring says: *"This script maintains the vector search system by: 1. Computing feature statistics from athlete data, 2. Updating position weights from archetype importance, 3. Backfilling athlete and archetype vectors. Run this script when: Adding large batches of new athletes, Vector search performance degrades…"* — i.e., this assumes athlete data already exists. It is a **maintenance script**, not a seed.

#### Volumes
| Volume                          | Role | Classification     | Evidence                                                                                                                                                                                                                                                                                                                | Recommendation                                                                                                                                                            |
|---------------------------------|------|--------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `prospect_path_postgres_data`   | App database | `empty_or_unknown` | Backend `.env` → Neon. The seed script is a maintenance utility, not a fixture loader, so even if you wanted to "regenerate" this volume there's no recipe to do so. **However**: athlete data only exists in Neon. The local docker postgres has no fixture path to populate it. No host volume present on this machine. | **`skip`** — the canonical athlete data lives in Neon. There is no recipe that regenerates this docker volume; if a local fixture DB becomes important, that's net-new work. |
| `prospect_path_redis_data`      | Cache        | `empty_or_unknown` | No Redis client config. Unused.                                                                                                                                                                                                                                                                                       | **`skip`**     |

---

### 7. pantheon

- **Compose**: `pantheon/docker-compose.yml` (default rocketship template)
- **Backend runtime DB target**: `ep-sparkling-sunset-ah1xfvha.c-3.us-east-1.aws.neon.tech/neondb` → **Neon**
- **Prisma migrations**: 19 (including `0_baseline`, `0_init`, wealth-OS phases, asset-event real estate, dropbox processing pipeline — financial / wealth management features)
- **Seed scripts**: none

#### Volumes
| Volume                     | Role | Classification     | Evidence                                                                | Recommendation |
|----------------------------|------|--------------------|-------------------------------------------------------------------------|----------------|
| `pantheon_postgres_data`   | App database | `empty_or_unknown` | Backend `.env` → Neon. No host volume. Wealth/financial data is in Neon.| **`skip`**     |
| `pantheon_redis_data`      | Cache        | `empty_or_unknown` | No Redis client config. Unused.                                        | **`skip`**     |

---

### 8. spaceship_template

- **Compose**: `spaceship_template/docker-compose.yml` (default rocketship template — this *is* the template that the other 8 projects were forked from)
- **Backend runtime DB target**: `ep-gentle-hat-adzflnbf-pooler.c-2.us-east-1.aws.neon.tech/neondb` → **Neon**
- **Prisma migrations**: **none** (`apps/backend/prisma/migrations/` directory does not exist; only `schema.prisma` is present)
- **Seed scripts**: none
- **README signal**: troubleshooting section instructs `npx prisma migrate dev` for first-time setup (`README.md:305`, `README.md:403`), confirming the template ships with no migrations and expects fresh init.

#### Volumes
| Volume                              | Role | Classification     | Evidence                                                                                                                                                                                              | Recommendation                                                                                                       |
|-------------------------------------|------|--------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------|
| `spaceship_template_postgres_data`  | App database | `empty_or_unknown` | Zero prisma migrations on disk → schema has never been applied; this is the literal upstream template. README explicitly directs users to run `prisma migrate dev` on first use. No host volume present.| **`skip`** — if the template stack is brought up at all on Hetzner, fresh `prisma migrate dev` is the documented path. |
| `spaceship_template_redis_data`     | Cache        | `empty_or_unknown` | No Redis client config. Unused.                                                                                                                                                                       | **`skip`**     |

---

### 9. abundant-mines

- **Compose**: `abundant-mines/docker-compose.yml` (default rocketship template)
- **Backend runtime DB target**: `ep-super-sunset-adn4sxe3-pooler.c-2.us-east-1.aws.neon.tech/neondb` → **Neon**
- **Prisma migrations**: 10 (contract signings, hubspot sync indexes, qbo product mapping, payment reminder logs, billing pause, billing cancelled, locale on contacts; one raw `create_email_queue_archive.sql`)
- **Seed scripts**: none

#### Volumes
| Volume                          | Role | Classification     | Evidence                                                                                | Recommendation |
|---------------------------------|------|--------------------|-----------------------------------------------------------------------------------------|----------------|
| `abundant-mines_postgres_data`  | App database | `empty_or_unknown` | Backend `.env` → Neon. No host volume. Billing/HubSpot/QBO data is in Neon.            | **`skip`**     |
| `abundant-mines_redis_data`     | Cache        | `empty_or_unknown` | No Redis client config. Unused.                                                        | **`skip`**     |

---

## Phase 5 action list (flattened)

This is the executable list for Phase 5 — **zero tarballs required**:

1. **No `tarball_and_transfer` actions.** (No volume meets the evidence bar for `real_data`.)
2. **No `fresh_seed_on_server` actions** are *required*. The applications connect to Neon at runtime; the docker postgres is dead infrastructure. If the supervisor decides to bring up local docker postgres for any of these projects on Hetzner, the per-project seed-on-empty-DB recipes are:
   - **pulse-card** — after `docker compose up -d postgres` and pointing the backend at the local DB:
     ```bash
     # In apps/backend, with DATABASE_URL pointed at the local docker postgres
     npx prisma migrate deploy
     ADMIN_SEED_EMAILS="daniel@petrastella.io" npx tsx scripts/seed-admin-users.ts
     npx tsx scripts/seed-demo-data.ts
     ```
   - **domdms** — after `docker compose up -d postgres`:
     ```bash
     npx prisma migrate deploy
     node scripts/seed-lead-messages.js
     ```
   - **gabber-studio / fixyrs / homeschool-whisper / pantheon / abundant-mines** — schema only:
     ```bash
     npx prisma migrate deploy
     ```
   - **prospect_path** — schema only; no fresh-DB seed exists. `seed-vector-search.ts` is a post-data-load maintenance utility and will produce nothing useful against an empty database:
     ```bash
     npx prisma migrate deploy
     # seed-vector-search.ts intentionally NOT run — it requires existing athlete data
     ```
   - **spaceship_template** — first-time prisma init (no migrations exist):
     ```bash
     npx prisma migrate dev --name init
     ```
3. **Skip all 18 named volumes.** Phase 5 should `docker compose up -d` (whichever services it brings up) on Hetzner with no pre-seeded data on disk. The compose top-level `volumes:` block will create empty volumes on first start.
4. **For reference, the "if you change your mind" tarball recipe** for any single volume — not recommended in this case but documented for completeness — is the canonical pattern:
   ```bash
   # On the source host (none of these volumes exist on this host today):
   docker run --rm \
     -v <project>_postgres_data:/volume:ro \
     -v "$PWD":/backup \
     alpine tar czf /backup/<project>_postgres_data.tar.gz -C /volume .

   # On Hetzner:
   docker volume create <project>_postgres_data
   docker run --rm \
     -v <project>_postgres_data:/volume \
     -v "$PWD":/backup \
     alpine tar xzf /backup/<project>_postgres_data.tar.gz -C /volume
   ```
   The same pattern applies to `<project>_redis_data` if ever needed. Substitute the actual host-side volume name (`docker volume ls` gives the project-prefixed name).

---

## Risk callouts

### R1 — Should the docker-compose stacks ship to Hetzner at all? (`needs_user_input`)
**Question for supervisor**: Every project's backend connects to **Neon** for Postgres at runtime, and no project connects to the local docker `redis` service from app code (one project, `domdms`, uses Upstash REST instead). The docker-compose stacks appear to be **dev-environment leftovers from the original rocketship template**, not production infrastructure. **Do you want these compose stacks to deploy to Hetzner at all?** If the answer is "no — backends → Neon, frontends → Vercel, no local DB needed", then Phase 5 can entirely drop docker-compose for these 9 projects, which sidesteps a separate problem (see R2). If the answer is "yes — I run them locally as a Neon mirror / offline dev environment / migration target", then the per-project skip recommendations above stand and the seed recipes in the action list are the way to populate Hetzner.

### R2 — Container-name collisions if multiple stacks share the host
8 of 9 stacks hardcode container names `rocketship-postgres`, `rocketship-backend`, `rocketship-frontend`, `rocketship-redis`. Bringing up two of these projects on the same docker host will fail at the second `docker compose up` because the container names already exist. (domdms is the exception; it uses `domdms-*` prefixes.) Same applies to host-port bindings: every stack publishes `5432`, `5433` (frontend → 3000:80 actually), `3001`, `6379` — only one stack can be up at a time per host without compose-project-name overrides or per-project port maps. **This is not a volume problem and is out of scope for this task, but Phase 5 will hit it the moment a second stack is brought up on Hetzner. Worth raising to the EPIC owner before Phase 5 plans concurrent stacks.**

### R3 — domdms hardcoded postgres credential surfaces in compose
The compose file uses the docker postgres image's documented default credential (the same one every other stack uses). This is not a leak per se since it's the upstream image's standard dev value already public in git, but it should be replaced with an env-var reference before the stack is exposed on a public-network Hetzner instance. **Out of scope for this task; flagging because Phase 5 will publish port 5432 on a public host.**

### R4 — No volume in any project has been classified `real_data`
Per the spec ("evidence required: row counts, named tables, README mentions of 'seed from prod', sample data the user edited"), I found **zero** evidence meeting that bar. The docker daemon is inactive and the host has no volumes, so I could not run any read-only `pg_stat_user_tables` query. **If the supervisor has reason to believe a particular project's local docker postgres on a *different* machine holds important data**, that machine — not this one — is the source for the volume tarball. From this host, there is nothing to tarball.

### R5 — One token visible during investigation (already handled, not in this report)
While reading `domdms/.env` for the Redis URL, an `UPSTASH_REDIS_REST_TOKEN` value was visible in shell output. **It is not reproduced in this document.** Only the Upstash hostname (`workable-ocelot-82611.upstash.io`) is referenced, since the hostname is needed to explain why the local docker redis volume is unused. The supervisor may want to confirm `.env` files are gitignored across all 9 projects (not part of this task's scope but relevant to migration security posture).

---

## Methodology + caveats

- **Read-only constraint**: no files were modified in `~/Petrastella/`. No `INSERT/UPDATE/DELETE` was issued (none could be — docker daemon inactive). No destructive docker commands.
- **No live row counts**: docker daemon is inactive on this host (`systemctl is-active docker → inactive`), so the Postgres `pg_stat_user_tables` / Redis `INFO keyspace` queries described in the task spec could not be executed. Classification falls back to: (a) docker-compose source, (b) backend `.env` runtime targets, (c) presence of seed scripts and prisma migrations, (d) host filesystem absence of `/var/lib/docker/volumes/<project>_*`.
- **Secrets hygiene**: every backend `.env` contains a database connection string with embedded credentials. Only the **host portion** of each Neon URL is reproduced in this document (the part after `@` and before `?`), never the user/password segment. No DB credential plaintext appears in this document; the compose file's hardcoded dev credential is the upstream postgres image's documented default and is described only in prose. No Upstash token, no Firebase admin config, no Vercel/GitHub tokens are reproduced.
- **`additive-only` compliance**: this file is the only write. The `migration/` directory in the cas-src worktree was created (net-new directory). No existing files modified.
