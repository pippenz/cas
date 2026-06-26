# Release Slack Rubric

**This is a hard rule. It runs on EVERY PR/merge to `main` (every release push).**

After a release is pushed + tagged, post to **#cas-internal** (`C0B44GUKDK2`). Always **two distinct top-level posts** (not threaded replies) — one per audience:

1. **User-perspective post**
2. **Dev-perspective post**

## Shape (identical for both posts)

1. **Open with the punch** — one or two lines of plain, punchy language framed as **how it was → how it is now**. Lead with the change in experience, not the mechanism.
2. **Details below** — bullets fleshing out the punch, written from that post's perspective.

## Voice rules

- **User post: ALWAYS plain language.** Describe what the user feels/sees. No jargon dumps.
- **Dev post: may be more technical** — code/behavior level is fine.
- **BOTH posts:**
  - **No CAS-internal agent actions.** Do not narrate supervisor/worker/factory/director orchestration, task lifecycle bookkeeping, who-closed-what, epics, etc.
  - **No ticket numbers.** No `cas-xxxx`, no epic IDs. Describe the change, not the tracking artifact.
  - Lead with the before→after punch; keep it tight.

## Why

These posts are for a product/stakeholder audience. They communicate *impact*, not internal process. The user always gets the plain-language story; the dev gets the same story with technical substance — neither gets the factory's internal plumbing or ticket IDs.

## Checklist per release

- [ ] Release pushed to `origin/main` + tag pushed
- [ ] Post 1 (user): punch (was→now) + plain-language details
- [ ] Post 2 (dev): punch (was→now) + technical details
- [ ] Both: zero ticket numbers, zero internal-agent narration
