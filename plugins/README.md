# ClawEnv Plugins

This directory will contain OpenClaw plugins that extend ClawEnv's
Bridge Server as a remote MCP (Model Context Protocol) service.

## Planned Structure

```
plugins/
├── mcp-bridge/          # Bridge Server as MCP remote tool provider
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs      # MCP server entry
│       ├── tools/       # Tool implementations
│       │   ├── file.rs  # File read/write/list tools
│       │   ├── exec.rs  # Command execution tool
│       │   └── web.rs   # Web browsing tool
│       └── auth.rs      # Permission & auth management
├── openclaw-skills/     # OpenClaw skill templates
│   ├── web-scraper/
│   ├── data-analyzer/
│   └── email-sender/
└── README.md
```

## MCP Bridge

The MCP Bridge exposes ClawEnv's Bridge Server APIs as MCP tools,
allowing OpenClaw agents running inside sandboxes to:

- Read/write files on the host (with permission control)
- Execute commands on the host (with approval workflow)
- Browse the web via host's network
- Access host's clipboard, notifications, etc.

## Development

Coming in v0.4+. See docs/12-multi-instance.md for architecture.
