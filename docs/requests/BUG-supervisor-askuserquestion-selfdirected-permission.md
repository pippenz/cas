# BUG: Supervisor AskUserQuestion surfaces as self-directed permission prompt in factory mode

**Filed:** 2026-07-22
**Source project:** Penguinz (factory session `Penguinz-zen-swan-26`, supervisor `agile-phoenix-4` / session `5340a10a-0f8b-4c78-b471-8f9b9bf0ddf4`)
**Severity:** medium (degrades supervisor intake; caused wasted worker dispatch downstream)
**Classification:** partial CAS bug — half routing/gating defect, half skill-guidance gap

## Observed

During intake of a new user request, the factory **supervisor** called the built-in
`AskUserQuestion` tool with questions genuinely aimed at the **human** (approach
selection + version clarification, structured options). Instead of reaching the human
as an interactive question dialog, it surfaced as a **permission request apparently
sent to the supervisor itself** and paused the system. The human had to manually
reject it and commented: *"you sent a permission request to yourself, bad way to start."*

## Expected

One of:
1. The question is **relayed to the human** (e.g., via the director channel that
   already injects human messages into the supervisor's turns), or
2. `AskUserQuestion` is **blocked in factory mode for supervisors** — exactly like
   `SendMessage` already is — with an error instructing "ask in plain text and end
   your turn; the director relays human replies."

Either is fine; the current middle state (call accepted, then mis-surfaced as a
self-permission prompt) is the worst outcome.

## Why it's only *partially* a CAS bug

The `cas-supervisor` skill hard-rules say: *"Never use AskUserQuestion for agent
communication. It is only for the **human** user and pauses the system."* This
wording affirmatively suggests AskUserQuestion **is** the right tool for
human-directed questions — but in factory topology the supervisor has no direct
human UI surface, so that path can't work. The skill guidance and the runtime
gating disagree; a supervisor following the skill text hits this trap.

## Downstream impact (why it matters)

After the misfire, the supervisor over-corrected: skipped clarification entirely and
dispatched a worker on an assumed approach, which missing facts later invalidated
(worker recalled, task reset). A clean block-with-guidance at call time would have
prevented the whole chain.

## Suggested fixes

1. **Gate:** In factory mode, deny `AskUserQuestion` for supervisor/worker roles with
   an actionable message ("ask the human in plain text; end turn; director relays
   the reply") — mirroring the existing SendMessage block.
2. **Skill text:** Update `cas-supervisor` hard rules + `references/intake.md` to say:
   in factory mode, human-directed questions go in the plain-text reply, never
   through AskUserQuestion.
3. (Optional, nicer) **Relay:** teach the director to render a supervisor's
   AskUserQuestion to the human and inject the structured answer back.

## Repro sketch

1. Start a factory session with a supervisor (agent-teams topology, director-mediated user I/O).
2. As supervisor, invoke `AskUserQuestion` with any human-directed question set.
3. Observe it surfaces as a permission prompt on the supervisor's own session rather
   than reaching the human; system pauses until manually rejected.
