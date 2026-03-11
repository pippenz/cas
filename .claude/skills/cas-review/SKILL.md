---
name: cas-review
description: Review staged code changes against CAS rules before commits
---

# review

# Code Review

Review staged code changes against CAS rules before committing.

## Usage

Invoke /review to spawn the code-reviewer subagent which will:
1. Get all CAS rules
2. Read staged files (git diff --cached)
3. Check each file against applicable rules
4. Report violations and suggestions

## Output

The code-reviewer returns a verdict:
- **APPROVED**: Ready to commit
- **NEEDS CHANGES**: Must fix violations first

## Tags

review, code, rules, commit
