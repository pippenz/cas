# Phase 8 env audit — per-project `.env*` files on target

**Task**: cas-dece (absorbs Phase 4)
**Generated**: 2026-04-11T10:34:50-04:00
**Target**: `daniel@87.99.156.244:~/projects/`

This audit inventories every `.env*` file that traveled in Phase 3, reports
KEY names only (never values), flags localhost/127.0.0.1/pippenz references,
and classifies each file as `benign` (references to local services the app
doesn't actually reach at runtime — e.g., docker postgres overridden by a
cloud endpoint in `.env.local`) or `actionable` (references the app
actually uses, which will break on the server).

Classification logic: for each file with localhost refs, we check whether
another `.env*` in the same project (higher in the load order —
`.env.local` typically wins) overrides the localhost-referenced key with a
non-localhost value. If yes, benign. If no, actionable.

## Summary table

| Project | env files (count) | Localhost refs total | Benign | Actionable |
|---|---|---|---|---|
| abundant-mines | 5 | 7 | 0 | 7 |
| closure-club | 1 | 0 | 0 | 0 |
| country-liberty | 0 | 0 | 0 | 0 |
| domdms | 6 | 8 | 0 | 8 |
| edws | 0 | 0 | 0 | 0 |
| fixy-quasar | 4 | 9 | 0 | 9 |
| fixyrs | 3 | 4 | 0 | 4 |
| full-package-media | 0 | 0 | 0 | 0 |
| gabber-studio | 6 | 5 | 2 | 3 |
| git-mcp-server | 1 | 0 | 0 | 0 |
| homeschool-whisper | 4 | 4 | 0 | 4 |
| logging | 1 | 0 | 0 | 0 |
| memory-lane | 1 | 0 | 0 | 0 |
| memory-lane-cloud | 1 | 1 | 0 | 1 |
| ozer | 3 | 0 | 0 | 0 |
| pantheon | 4 | 4 | 0 | 4 |
| petra-stella-cloud | 3 | 0 | 0 | 0 |
| petrastella-aws | 1 | 0 | 0 | 0 |
| pixel-hive | 5 | 6 | 0 | 6 |
| prospect_path | 3 | 7 | 0 | 7 |
| pulse-card | 7 | 6 | 0 | 6 |
| rocketship-template-new-instance | 3 | 3 | 0 | 3 |
| spaceship_template | 8 | 17 | 0 | 17 |
| tooling | 0 | 0 | 0 | 0 |
| tracetix | 2 | 3 | 0 | 3 |
| verified-path | 0 | 0 | 0 | 0 |

## REQUIRES HUMAN — actionable localhost refs

The following env keys point at localhost in their primary `.env` and are NOT overridden by `.env.local`. If the server runs these apps, these need rewriting to the target's networked values:

| Project | File | Key |
|---|---|---|
| abundant-mines | ./.env.graphiti | NEO4J_URI |
| abundant-mines | ./apps/frontend/.env | API_BASE |
| abundant-mines | ./apps/frontend/.env | NUXT_PUBLIC_API_BASE |
| abundant-mines | ./apps/frontend/.env | NUXT_PUBLIC_SITE_URL |
| abundant-mines | ./apps/backend/.env | FRONTEND_URL |
| abundant-mines | ./apps/backend/.env | QBO_REDIRECT_URI |
| abundant-mines | ./.env.playwright | TEST_BACKEND_URL |
| domdms | ./.env.graphiti | NEO4J_URI |
| domdms | ./apps/backend/.env.example | BASE_API_URL |
| domdms | ./apps/backend/.env.example | CORS_ORIGIN |
| domdms | ./apps/backend/.env | APP_URL |
| domdms | ./apps/backend/.env | BASE_API_URL |
| domdms | ./apps/backend/.env | DROPBOX_REDIRECT_URI |
| domdms | ./apps/backend/.env | FRONTEND_URL |
| domdms | ./apps/backend/.env | NUXT_PUBLIC_API_BASE |
| fixy-quasar | ./apps/.env.backup | APP_URL |
| fixy-quasar | ./apps/.env.backup | FIREBASE_AUTH_EMULATOR_HOST |
| fixy-quasar | ./apps/.env.backup | SERVER_URL |
| fixy-quasar | ./apps/.env.backup | TRANSCRIPT_URL |
| fixy-quasar | ./apps/.env.local | APP_URL |
| fixy-quasar | ./apps/.env.local | FIREBASE_AUTH_EMULATOR_HOST |
| fixy-quasar | ./apps/.env.local | SERVER_URL |
| fixy-quasar | ./apps/app/.env.dev | SERVER_URL |
| fixy-quasar | ./apps/app/.env.dev | STREAMING_SERVER |
| fixyrs | ./.env.graphiti | NEO4J_URI |
| fixyrs | ./apps/frontend/.env | NUXT_PUBLIC_API_BASE |
| fixyrs | ./apps/frontend/.env | NUXT_PUBLIC_SITE_URL |
| fixyrs | ./apps/backend/.env | FRONTEND_URL |
| gabber-studio | ./.env.graphiti | NEO4J_URI |
| gabber-studio | ./apps/backend/.env | BASE_API_URL |
| gabber-studio | ./apps/backend/.env | FRONTEND_URL |
| homeschool-whisper | ./.env.graphiti | NEO4J_URI |
| homeschool-whisper | ./apps/frontend/.env | NUXT_PUBLIC_SERVER_URL |
| homeschool-whisper | ./apps/frontend/.env | NUXT_PUBLIC_SITE_URL |
| homeschool-whisper | ./apps/backend/.env | FRONTEND_URL |
| memory-lane-cloud | ./packages/api/.env.local | NEXT_PUBLIC_API_URL |
| pantheon | ./apps/frontend/.env | NUXT_PUBLIC_API_BASE |
| pantheon | ./apps/frontend/.env | NUXT_PUBLIC_SITE_URL |
| pantheon | ./apps/backend/.env | DROPBOX_REDIRECT_URI |
| pantheon | ./apps/backend/.env | FRONTEND_URL |
| pixel-hive | ./.env.graphiti | NEO4J_URI |
| pixel-hive | ./apps/frontend/.env.example | API_BASE |
| pixel-hive | ./apps/frontend/.env.example | NUXT_PUBLIC_SITE_URL |
| pixel-hive | ./apps/backend/.env.example | DATABASE_URL |
| pixel-hive | ./apps/backend/.env.example | DIRECT_URL |
| pixel-hive | ./apps/backend/.env.example | FRONTEND_URL |
| prospect_path | ./.env.graphiti | NEO4J_URI |
| prospect_path | ./apps/frontend/.env | API_BASE |
| prospect_path | ./apps/frontend/.env | NUXT_PUBLIC_API_BASE |
| prospect_path | ./apps/frontend/.env | NUXT_PUBLIC_SITE_URL |
| prospect_path | ./apps/frontend/.env | SERVER_URL |
| prospect_path | ./apps/backend/.env | CORS_ORIGIN |
| prospect_path | ./apps/backend/.env | FRONTEND_URL |
| pulse-card | ./.env.graphiti | NEO4J_URI |
| pulse-card | ./apps/frontend/.env | API_BASE |
| pulse-card | ./apps/frontend/.env | NUXT_PUBLIC_API_BASE |
| pulse-card | ./apps/frontend/.env | NUXT_PUBLIC_SERVER_URL |
| pulse-card | ./apps/frontend/.env | NUXT_PUBLIC_SITE_URL |
| pulse-card | ./apps/backend/.env | FRONTEND_URL |
| rocketship-template-new-instance | ./domdms/apps/backend/.env | DROPBOX_REDIRECT_URI |
| rocketship-template-new-instance | ./domdms/apps/backend/.env | FRONTEND_URL |
| rocketship-template-new-instance | ./.env | GCLOUD_SDK_PATH |
| spaceship_template | ./.env.graphiti | NEO4J_URI |
| spaceship_template | ./Reference_Code/apps/.env.backup | APP_URL |
| spaceship_template | ./Reference_Code/apps/.env.backup | FIREBASE_AUTH_EMULATOR_HOST |
| spaceship_template | ./Reference_Code/apps/.env.backup | SERVER_URL |
| spaceship_template | ./Reference_Code/apps/.env.backup | TRANSCRIPT_URL |
| spaceship_template | ./Reference_Code/apps/.env.local | APP_URL |
| spaceship_template | ./Reference_Code/apps/.env.local | FIREBASE_AUTH_EMULATOR_HOST |
| spaceship_template | ./Reference_Code/apps/.env.local | SERVER_URL |
| spaceship_template | ./Reference_Code/apps/app/.env.dev | SERVER_URL |
| spaceship_template | ./Reference_Code/apps/app/.env.dev | STREAMING_SERVER |
| spaceship_template | ./apps/frontend/.env | API_BASE |
| spaceship_template | ./apps/frontend/.env | NUXT_PUBLIC_API_BASE |
| spaceship_template | ./apps/frontend/.env | NUXT_PUBLIC_SITE_URL |
| spaceship_template | ./apps/frontend/.env | SERVER_URL |
| spaceship_template | ./apps/backend/.env | CORS_ORIGIN |
| spaceship_template | ./apps/backend/.env | DROPBOX_REDIRECT_URI |
| spaceship_template | ./apps/backend/.env | FRONTEND_URL |
| tracetix | ./apps/frontend/.env | NUXT_PUBLIC_SERVER_URL |
| tracetix | ./apps/backend/.env | BASE_API_URL |
| tracetix | ./apps/backend/.env | FRONTEND_URL |

## Classification methodology

1. For each project, list all `.env` + `.env.*` files (excluding `node_modules`, `.git`).
2. For each file, extract KEY names (no values) of lines whose value matches localhost / 127.0.0.1 / 0.0.0.0 / pippenz.
3. For each such key, check if `.env.local` (the highest-priority loader in Node ecosystems) defines the same key with a non-localhost value. If yes → **benign**. If no → **actionable**.
4. Values are never captured or logged. Only keys, file paths, counts, sizes, and modes.

## Caveats

- Projects that use env loaders other than the standard Node dotenv chain may have different override semantics. The spec assumes the common case.
- Python projects and Next.js 14 projects use `.env.local` similarly, so the heuristic generalizes.
- The heuristic does NOT detect runtime-constant overrides (e.g., `DATABASE_URL` hardcoded in `prisma.config.ts`). Any such cases would appear in the actionable list and require human review.
