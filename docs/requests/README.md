# Cross-Team Request Convention

This directory is the **inbox** for work requests from other Petrastella teams.

## How It Works

- **Incoming requests:** Other teams drop `BUG-*.md` or `FEATURE-*.md` files here
- **Completion:** Move finished requests to `completed/` with a completion note appended
- **Your outbox:** To request work from another team, drop a file in *their* `docs/requests/` directory

## File Naming

```
BUG-<slug>.md        # Bug fix request
FEATURE-<slug>.md    # Feature request
```

Use lowercase kebab-case slugs. Keep names short and descriptive.

## Required Frontmatter

```markdown
---
from: CAS CLI team | Petra Stella Cloud team
date: 2026-04-12
priority: P0 | P1 | P2 | P3
cas_task: cas-xxxx          # optional — tracking task in source repo
---
```

## Completion Flow

1. Pick up the request, create a task in your tracker
2. Do the work, ship it
3. Append a completion block to the bottom of the file:

```markdown
---
completed: 2026-04-15
completed_by: cas-xxxx | cloud-xxxx
commit: abc1234
---
```

4. Move the file to `completed/`:
   ```
   git mv docs/requests/BUG-whatever.md docs/requests/completed/
   ```

5. Commit the move

## Quick Check

```bash
# What's pending?
ls docs/requests/*.md

# What's been done?
ls docs/requests/completed/
```

## Repo Pairs

| Your Repo | Their Inbox |
|---|---|
| cas-src | ~/Petrastella/petra-stella-cloud/docs/requests/ |
| petra-stella-cloud | ~/Petrastella/cas-src/docs/requests/ |
