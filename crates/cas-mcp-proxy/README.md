# cas-mcp-proxy

MCP proxy engine for CAS. Connects to upstream MCP servers and exposes their tools through a unified search and execute interface.

## Configuration

Upstream servers are configured in `.cas/proxy.toml` (project-scoped) and `~/.config/code-mode-mcp/config.toml` (user-scoped). Project config takes precedence.

### Supported transports

**Stdio** — spawns a child process:
```toml
[servers.my-server]
transport = "stdio"
command = "npx"
args = ["mcp-server-git"]
env = { HOME = "/tmp" }
```

**HTTP** — streamable HTTP connection:
```toml
[servers.sentry]
transport = "http"
url = "https://mcp.sentry.dev/mcp"
auth = "your-token"
```

**SSE** — server-sent events:
```toml
[servers.my-sse]
transport = "sse"
url = "https://example.com/sse"
```

## Search

`ProxyEngine::search(query, max_length)` filters the tool catalog:

- **Keywords**: case-insensitive substring match on tool name and description
- **Server filter**: `server:github issue` filters to the `github` server first
- **Empty query**: returns all tools

## Execute

`ProxyEngine::execute(code, max_length)` dispatches tool calls:

**JSON dispatch** (preferred):
```json
{ "server": "github", "tool": "list_issues", "args": { "repo": "myorg/app" } }
```

**Batch** (parallel execution):
```json
[
  { "server": "github", "tool": "list_issues", "args": { "repo": "app" } },
  { "server": "sentry", "tool": "list_errors", "args": { "project": "be" } }
]
```

**Dot-call syntax** (fallback):
```
github.list_issues({"repo": "myorg/app"})
```

## Hot-reload

The daemon watches `.cas/proxy.toml` for changes. On config change, `ProxyEngine::reload()` compares stored configs against new ones, disconnects removed servers, reconnects changed ones, and leaves unchanged servers connected.

## Feature flag

Enable with `cargo build --features mcp-proxy`. Without the feature, proxy commands return a helpful error message.
