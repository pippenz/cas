# CAS - Coding Agent System

Unified context system for AI agents: persistent memory, tasks, rules, and skills across sessions.

## Installation

### Quick Install (Recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/codingagentsystem/cas/main/install.sh | bash
```

### Homebrew (macOS)

```bash
brew install codingagentsystem/tap/cas
```

### Manual Download

Download the latest release for your platform from the [Releases page](https://github.com/codingagentsystem/cas/releases).

| Platform | Architecture | Download |
|----------|--------------|----------|
| macOS | Apple Silicon (M1/M2/M3) | [cas-aarch64-apple-darwin.tar.gz](https://github.com/codingagentsystem/cas/releases/latest/download/cas-aarch64-apple-darwin.tar.gz) |
| Linux | x86_64 | [cas-x86_64-unknown-linux-gnu.tar.gz](https://github.com/codingagentsystem/cas/releases/latest/download/cas-x86_64-unknown-linux-gnu.tar.gz) |

## Quick Start

```bash
# Initialize CAS in your project
cas init

# Store a memory
cas remember "The API uses JWT tokens for authentication"

# Create a task
cas task create "Implement user login" --priority 1

# Search across all context
cas search "authentication"

# Start the MCP server for Claude Code integration
cas serve
```

## Features

- **Persistent Memory** - Store learnings, preferences, and context that persists across sessions
- **Task Tracking** - Track work with priorities, dependencies, and progress notes
- **Rules Engine** - Define and enforce coding standards and patterns
- **Skills System** - Create reusable workflows and automations
- **MCP Server** - Integrates with Claude Code as an MCP server
- **Hybrid Search** - BM25 + semantic search for finding relevant context

## Claude Code Integration

Add CAS as an MCP server in your Claude Code settings:

```json
{
  "mcpServers": {
    "cas": {
      "command": "cas",
      "args": ["serve"]
    }
  }
}
```

## Updating

CAS includes a built-in self-update mechanism:

```bash
cas update           # Update to latest version
cas update --check   # Check for updates without installing
```

## System Requirements

- **macOS**: Apple Silicon (M1/M2/M3) - Intel Macs not currently supported
- **Linux**: x86_64 (glibc)

## Documentation

- [Getting Started Guide](https://github.com/codingagentsystem/cas/wiki)
- [MCP Integration](https://github.com/codingagentsystem/cas/wiki/MCP-Integration)
- [Configuration](https://github.com/codingagentsystem/cas/wiki/Configuration)

## Support

- [Issues](https://github.com/codingagentsystem/cas/issues) - Bug reports and feature requests
- [Discussions](https://github.com/codingagentsystem/cas/discussions) - Questions and community

## License

Proprietary - All rights reserved.
