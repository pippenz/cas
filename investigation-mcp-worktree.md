# Investigation: Why MCP Tools Don't Load in Worktrees

**Worker**: clever-hound-53
**Worktree**: `/home/pippenz/cas-src/.cas/worktrees/clever-hound-53`
**Date**: 2026-03-25

## Environment Checks

### 1. CAS_ROOT
```
CAS_ROOT=/home/pippenz/cas-src/.cas
```
**Result**: Correctly set. Points to main project's `.cas` directory.

### 2. .mcp.json
```json
{
  "mcpServers": {
    "cas": {
      "args": ["serve"],
      "command": "cas"
    }
  }
}
```
**Result**: Present and correct. Simple config using `cas serve`.

### 3. cas binary
```
$ which cas
cas  # shell function wrapping `command cas`
$ command cas --help
# Works — lists all subcommands
```
**Result**: Binary is findable and executable.

### 4. .git file (worktree confirmation)
```
gitdir: /home/pippenz/cas-src/.git/worktrees/clever-hound-53
```
**Result**: Confirmed worktree. Points back to main repo's git dir.

### 5. cas serve process
```
PID 739194 -> CWD: /home/pippenz/cas-src/.cas/worktrees/clever-hound-53
  ENV CAS_ROOT: CAS_ROOT=/home/pippenz/cas-src/.cas
```
**Result**: Running, correct CWD, correct CAS_ROOT.

### 6. Database file descriptors
```
/proc/739194/fd/10 -> /home/pippenz/cas-src/.cas/cas.db
/proc/739194/fd/11 -> /home/pippenz/cas-src/.cas/cas.db
/proc/739194/fd/12 -> /home/pippenz/cas-src/.cas/cas.db-wal
```
**Result**: MCP server has the MAIN cas.db open (not a worktree-local copy).

## Root Cause

**MCP tools DO load in worktrees — but with a 2-4 minute startup delay.**

The issue is NOT that tools fail to connect. It's that they take too long to respond on first contact, causing Claude Code to report them as "still connecting".

### Evidence: Request Timeouts in Logs

From `/home/pippenz/cas-src/.cas/logs/cas-2026-03-25.log`:
```
15:17:51 — MCP error -32001: Request timed out
15:18:01 — Request timed out
15:18:16 — 5 timeouts in rapid succession
15:18:26 — 6 more timeouts in < 1 second
```

### Cause: SQLite Contention (Thundering Herd)

**6 `cas serve` processes** all hitting the same `cas.db` simultaneously:

| PID | CWD | Role |
|-----|-----|------|
| 6375 | /home/pippenz/cas-src | Old session |
| 734918 | /home/pippenz/cas-src | Supervisor |
| 738521 | .cas/worktrees/watchful-badger-90 | Worker |
| 738701 | .cas/worktrees/noble-lion-41 | Worker |
| 738964 | .cas/worktrees/keen-phoenix-63 | Worker |
| 739194 | .cas/worktrees/clever-hound-53 | Worker |

All 6 share `CAS_ROOT=/home/pippenz/cas-src/.cas` and open the same SQLite database.

### Contributing Factors

1. **Concurrent store initialization**: Each `cas serve` opens ~10 store types, each calling `CREATE TABLE IF NOT EXISTS`. With 5 processes doing writes simultaneously, SQLite WAL + 5s busy_timeout causes cascading waits.

2. **Embedded daemon per server**: Each `cas serve` spawns an `EmbeddedDaemon` for code indexing. Six daemons indexing the same project = CPU/IO contention.

3. **Cloud sync on startup**: Each server attempts a cloud pull (5s timeout, background task). Opens additional store connections.

4. **Eager agent registration**: Each worker does a DB write at startup for registration — 4 workers simultaneously adds to contention.

### Why Workers Perceive "Unavailable"

Claude Code's MCP client has its own timeout for the `initialize` → `list_tools` handshake. When `cas serve` is blocked on SQLite, it can't respond to the handshake in time. Claude Code then reports "still connecting: cas" indefinitely.

The worker agent sees `ToolSearch` return no results, concludes MCP is broken, wastes 2-4 turns trying to diagnose/fix, runs `cas init` (creating a duplicate `.cas/`), and messages the supervisor.

## Additional Discovery

Running `cas init -y` in a worktree creates a **second** `.cas/` directory:
```
/home/pippenz/cas-src/.cas/worktrees/clever-hound-53/.cas/cas.db  (807K, empty)
/home/pippenz/cas-src/.cas/cas.db                                  (4.1M, main)
```
This is harmful and must be prohibited in worker guidance.

## Recommended Fix Path

1. **Immediate (prompt-level)**: Update `cas-worker.md` skill to detect worktree mode and skip MCP retries → go to Fallback Workflow (done in cas-096f, commit c27ff9d)

2. **Short-term (code)**: Stagger worker startup with 2-3s delays between spawns to avoid thundering herd

3. **Medium-term (code)**:
   - Disable embedded daemon for worker MCP servers (workers don't need code indexing)
   - Skip cloud sync for workers (supervisor handles cloud)
   - Reduce store init writes (lazy initialization)

4. **Long-term**: Consider a single shared MCP server process for the factory instead of per-agent servers
