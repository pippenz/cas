# Supervisor Has No Reliable Way to Know Which Database to Query

## Summary

The supervisor (and agents in general) repeatedly query the wrong Neon database project because there's no single source of truth for database connection mapping. In today's session, the supervisor:

1. First queried `withered-river-688585` thinking it was only a staging DB — it's actually "petrastella dev" which hosts the PRODUCTION ozerhealth database
2. Then queried `broad-unit-52453806` ("ozer_staging") — the correct staging DB
3. Then couldn't find the prod DB because it didn't know to look in "petrastella dev"
4. User had to intervene 3 times before the correct database was queried

The project name "petrastella dev" is misleading — it hosts production data for multiple apps.

## Root Cause

- No CLAUDE.md or memory entry mapped Neon project IDs to environments
- The .env file has two DATABASE_URL entries (prod commented out, staging commented in, or vice versa) with no labels
- The Neon project name "petrastella dev" doesn't indicate it's the production database
- Multiple Neon orgs (Petra Stella, Daniel) add confusion

## Fix Applied

Memory file created at `~/.claude/projects/-home-pippenz-Petrastella-ozer/memory/neon-databases.md` with full mapping. Also added to MEMORY.md.

## Broader Issue for CAS

CAS should support a `context` or `environment` system where critical infrastructure mappings (database projects, deployment URLs, API keys) are stored once and surfaced automatically when agents need to query databases. Currently each agent independently discovers (or fails to discover) this information.

Consider: a `mcp__cas__context action=get key=neon_prod_project` style lookup that returns saved infrastructure context.
