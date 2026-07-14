# Factory integrity — close/merge/auth fixes (2026-07-14)

Channel: #cas-internal (`C0B44GUKDK2`). Two top-level posts.

Shipped on `main` as merge `88efc72` (no version bump; still 2.27.0).

---

## Post 1 — User

Factory used to leave you stuck after “done” work: closes bounced even when the branch was integrated, Stop didn’t always stop a Grok turn, and supervisors could miss merge-queue work. Now close and cancel behave like they mean it — finished work stays finished, Stop/Esc cut the turn, and merge waits surface clearly.

- Closing a task no longer thrash-retries verify/close after the task is already closed.
- Grok Stop and Escape cancel an in-flight turn instead of leaving the pane spinning.
- When a task is waiting only on merge, that state is visible and actionable instead of looking like a random hang.
- Isolated workers can close without being blocked by unrelated dirt on the shared main checkout.
- Pasting multi-line text into a prompt no longer pretends a turn already started.

## Post 2 — Dev

Close/merge integrity hardened end-to-end: origin-parent proof fail-closed on unknown git state; worktree_merge never silently targets trunk; explicit task_id must bind to the System-B worker; epic verification owner transfer is authorized; lightweight close lint scopes to the worker range; stale close/verify after Closed is suppressed; Grok turn-cancel + bracketed-paste submit classification fixed.

- Close merge gate: origin/`<parent>` rescue requires KnownZero from the success-bearing unmerged-count helper; Unknown (missing ref / failed merge-base / rev-list) rejects.
- worktree_merge: assignee/focused-epic target resolution; explicit `allow_trunk` ≠ dirty `force`; task_id authorization (assignee/lease must match worker).
- Task update: `epic_verification_owner` is a controlled transfer (owner/supervisor → live supervisor/director identity), with trim/canonicalize at write boundaries.
- Close lifecycle: lightweight structural lint uses worker worktree merge-base..HEAD; Closed + halt paths no-op stale close/verify.
- Mux/TUI: Grok active-turn cancel; `BracketedPasteTracker` so keystream CR/LF inside `\e[200~`…`\e[201~` is not prompt submit.
- Release mechanics: merge commit `88efc72` on `main`; epic tip `ab43087`; cargo version remains **2.27.0** (no tag bump this ship).
