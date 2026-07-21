# `cas --version` format breaks the supervisor-checklist binary-freshness recipe

**Date:** 2026-07-21
**Reporter:** warm-falcon-13 (supervisor, ozer project)
**Severity:** trivial (docs/CLI mismatch), but it's step 0 of every factory session

## Symptom
The cas-supervisor-checklist skill (cas-d0f9 pre-flight) says:

```
cas --version | awk '{print $NF}'   # hash the running binary was built from
```

Actual output today:

```
$ cas --version
cas 2.27.0 (9b52e17-dirty 2026-07-16)
$ cas --version | awk '{print $NF}'
2026-07-16)
```

`$NF` now grabs the build date + stray paren, so the follow-up
`git log --oneline HEAD --not <running-hash>` dies with `fatal: bad revision '2026-07-16)'`.

## Expected
Either the checklist recipe updates to extract the hash (e.g. `sed -E 's/.*\(([0-9a-f]+).*/\1/'`), or better: add `cas --version --short-hash` (or `cas factory freshness-check` doing the whole comparison server-side) so the recipe can't drift again. Also worth deciding what `-dirty` should mean for the check — today the hash matches HEAD but the binary was built from a dirty tree, which the recipe silently treats as fresh.


## Completion

- **completed:** 2026-07-21
- **epic:** cas-887b — Factory reliability: open docs/requests bugs → main
- **completed_by:** cas-5a01
- **status:** Fixed on epic tip; report archived from `docs/requests/`.
