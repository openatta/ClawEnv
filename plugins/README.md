# ClawEnv Plugins

## mcp-bridge — MCP Server for Host Access

An MCP (Model Context Protocol) server that exposes ClawEnv's Bridge API
as tools for AI agents running inside sandboxes.

### Architecture

```
Sandbox (Lima/Podman/WSL2)              Host Machine
┌──────────────────────┐           ┌──────────────────┐
│  OpenClaw Agent      │           │  ClawEnv App     │
│       │              │           │       │          │
│       ▼              │           │  Bridge Server   │
│  mcp-bridge          │  HTTP     │  :3100           │
│  (MCP stdio server)  │─────────►│  /api/file/*     │
│                      │           │  /api/exec       │
└──────────────────────┘           └──────────────────┘
```

### Tools Provided

| Tool | Description | Bridge Endpoint |
|------|-------------|-----------------|
| `file_read` | Read host file | POST /api/file/read |
| `file_write` | Write host file | POST /api/file/write |
| `file_list` | List host directory | POST /api/file/list |
| `exec` | Execute host command | POST /api/exec |
| `browser_open` | Open URL in host browser | POST /api/exec (open cmd) |

### Build & Usage

```bash
# Build (from plugins/mcp-bridge/)
cd plugins/mcp-bridge
cargo build --release

# Run inside sandbox (OpenClaw connects via MCP stdio)
./target/release/clawenv-mcp-bridge --bridge-url http://host.lima.internal:3100
```

### OpenClaw Integration

Add to OpenClaw's MCP config:
```json
{
  "mcpServers": {
    "clawenv": {
      "command": "/path/to/clawenv-mcp-bridge",
      "args": ["--bridge-url", "http://host.lima.internal:3100"]
    }
  }
}
```

### Host URL by Platform

| Platform | Bridge URL |
|----------|-----------|
| Lima (macOS) | `http://host.lima.internal:3100` |
| Podman (Linux) | `http://host.containers.internal:3100` |
| WSL2 (Windows) | `http://$(cat /etc/resolv.conf \| grep nameserver \| awk '{print $2}'):3100` |

### Prerequisites

- ClawEnv Bridge Server enabled (Settings → Bridge → Enable)
- Permission rules configured in Bridge settings

### Security

All tool calls go through Bridge Server's permission engine:
- File access: glob whitelist/deny patterns
- Command exec: allow/deny command lists
- Operations requiring approval: HTTP 403 (approval UI in Phase 4)
