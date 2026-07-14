---
from: Ozer supervisor (pippenz @ /home/pippenz/Petrastella/ozer)
date: 2026-07-14
priority: P3
---

# BUG: Review-envelope validation reports one missing field per attempt; Finding schema undocumented; gate error text recommends the wrong mode

Three small quality-of-life defects in the `task close` code-review gate, hit in sequence today:

## 1. Piecemeal validation errors

Submitting `code_review_findings` with findings missing optional-looking fields produced serial rejections — first:

```
missing field `why_it_matters` at line 1 column 252
```

then, after adding that field everywhere:

```
missing field `evidence` at line 1 column 424
```

Each attempt costs a full close-call round trip with a large JSON payload. Validation should report **all** missing/invalid fields for all findings in one response.

## 2. Required Finding shape is undocumented at the point of failure

Both errors say only `Expected shape: {residual: Finding[], pre_existing: Finding[], mode: string}` — the `Finding` fields (`title`, `severity`, `file`, `line`, `why_it_matters`, `evidence`, `autofix_class`, `owner`, `confidence`, `pre_existing`, …?) are nowhere in the error, the `task` tool description, or the skill doc's Step 5. Callers have to discover the schema by trial or by fishing a prior workflow result out of a journal.

## 3. `CODE_REVIEW_REQUIRED` guidance contradicts the ownership model

The rejection text says:

> 1. Invoke the cas-code-review skill … with **mode=autofix** and the current diff.

but the skill doc states autofix is the **legacy `owner=worker`** path and the default `[code_review] owner = "supervisor"` config routes through `interactive` (or `headless` for skill-to-skill). A supervisor obeying the error verbatim would run the wrong mode. The error text should branch on (or at least mention) the configured owner, and for supervisors suggest `mode=interactive`/`headless`.

## Environment

- `cas 2.27.0 (dd8bcbd-dirty 2026-07-11)`, supervisor closing tasks `cas-5372` / `cas-ea3e`, session `07275a32-c0d5-4695-abbb-5c04663df721`, project `/home/pippenz/Petrastella/ozer`
