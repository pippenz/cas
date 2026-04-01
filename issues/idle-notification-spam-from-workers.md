# Idle Notification Spam Fills Supervisor Context Window

## Summary

Workers send `{"type":"idle_notification","from":"<name>","timestamp":"...","idleReason":"available"}` JSON messages every time their turn ends. When multiple workers are idle simultaneously (which is most of the time — workers are idle between tasks), these notifications flood the supervisor's conversation context.

## Example From Today

After workers completed tasks, the supervisor received 15+ idle notifications within 2 minutes:
```
golden-koala-97: {"type":"idle_notification"...}
golden-koala-97: {"type":"idle_notification"...}
solid-heron-10: {"type":"idle_notification"...}
golden-koala-97: {"type":"idle_notification"...}
solid-heron-10: {"type":"idle_notification"...}
```

Each notification consumes ~50-100 tokens of context. 15 notifications = ~1,000 tokens of noise.

## Impact

- Supervisor context window fills with repetitive JSON
- Important worker messages (completion reports, blockers) get buried between idle pings
- Supervisor wastes turns acknowledging or ignoring idle notifications
- In long sessions, context compression may drop important earlier context to make room for idle spam

## Proposed Fixes

1. **Deduplicate**: Don't send idle notification if the previous message from the same worker was also an idle notification
2. **Batch**: Collect idle notifications and deliver as a single summary: "Workers idle: golden-koala-97, solid-heron-10"
3. **Suppress after shutdown**: Never send idle notifications for workers that have been told to stand by or shut down
4. **Rate limit**: Max 1 idle notification per worker per 5 minutes
