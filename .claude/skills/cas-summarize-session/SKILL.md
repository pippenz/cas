---
name: cas-summarize-session
description: Generate session summary before stopping. Use when ending a session to capture accomplishments.
---

# summarize-session

# Session Summary

Generate a concise summary of the current session before stopping.

## When to Use

Call this skill when you're about to stop and want to capture what was accomplished.

## Process

As the parent agent with full context, YOU should:

1. **Review your conversation** - What tasks were worked on? What was accomplished?

2. **Check CAS for related data:**
   - mcp__cas__task action: mine - Tasks you touched
   - mcp__cas__memory action: recent, limit: 10 - Recent learnings

3. **Create the summary** with this structure:
   - Completed: [Task ID] Brief outcome
   - In Progress: [Task ID] Current state  
   - Key Decisions: Decision about X
   - Files Changed: path/file.rs - What changed
   - Next Session: Suggested starting point

4. **Store the summary:**
   mcp__cas__memory action: remember, content: summary, entry_type: context, tags: session,summary

## Guidelines

- Keep it under 500 words
- Focus on outcomes, not process
- Include task IDs for traceability
- Note any blockers for next session

## Tags

session, summary, stop
