# ClawEnv Plugins

## mcp-bridge — MCP Tools for Host Access

A standard MCP (Model Context Protocol) server that exposes ClawEnv's
Bridge API as tools for OpenClaw agents running inside sandboxes.

Built with TypeScript + `@modelcontextprotocol/sdk`.

### Architecture

```
Sandbox (Lima/Podman/WSL2)              Host Machine
┌────────────────────┐              ┌─────────────────┐
│ OpenClaw Agent     │              │ ClawEnv App     │
│   │ (MCP stdio)    │              │   │             │
│   ▼                │              │ Bridge Server   │
│ mcp-bridge.mjs     │───HTTP──────►│ :3100           │
│ (4.7KB Node.js)    │              │ /api/file/*     │
│                    │              │ /api/exec       │
└────────────────────┘              └─────────────────┘
```

### Tools

| Tool | Description |
|------|-------------|
| `file_read` | Read a file on the host |
| `file_write` | Write a file on the host |
| `file_list` | List directory contents |
| `exec` | Execute a command on the host |
| `host_info` | Get Bridge Server status + permissions |

### Quick Start

```bash
# Build
cd plugins/mcp-bridge
npm install
npm run build

# Register with OpenClaw (inside sandbox)
openclaw mcp set clawenv '{"command":"node","args":["/workspace/mcp-bridge/dist/index.mjs"]}'

# Verify
openclaw mcp list
```

### Host URL by Platform

| Platform | URL |
|----------|-----|
| Lima (macOS) | `http://host.lima.internal:3100` |
| Podman (Linux) | `http://host.containers.internal:3100` |
| WSL2 (Windows) | Auto-detect from `/etc/resolv.conf` |

### Prerequisites

1. ClawEnv Bridge Server enabled (Settings → Bridge → Enable)
2. Permission rules configured
3. Node.js available in sandbox (installed by default)
