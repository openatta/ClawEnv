# ClawEnv

[![CI](https://github.com/openatta/ClawEnv/actions/workflows/ci.yml/badge.svg)](https://github.com/openatta/ClawEnv/actions/workflows/ci.yml)

[中文文档](docs/README-zh.md)

> Cross-platform sandbox installer, launcher & manager for OpenClaw AI agents.

ClawEnv creates a secure, isolated Alpine Linux sandbox on your system — powered by **Lima** (macOS), **WSL2** (Windows), or **Podman** (Linux) — so AI agents run safely without affecting your host OS.

## Features

- **One-Click Install**: GUI wizard with system checks, proxy detection, and progress tracking
- **Sandbox Isolation**: Each instance runs in its own Alpine Linux VM/container
- **Native Mode**: Optional host-native install (no sandbox) for developers
- **Multi-Instance**: Run multiple OpenClaw instances with automatic port allocation
- **System Tray**: Background health monitoring, notifications, quick start/stop
- **In-Browser Terminal**: ttyd + xterm.js terminal per sandbox VM
- **Browser HIL**: Human-in-the-Loop via noVNC when agents need manual help (CAPTCHA, 2FA)
- **MCP Bridge**: Agents can access host files/commands through permission-controlled bridge
- **Exec Approval**: Agent exec commands popup for user confirmation
- **Auto-Update**: Periodic version checks with upgrade prompts
- **Autostart**: Optional launch-at-login (default off)

## Quick Start

```bash
# macOS / Linux
cargo tauri build
open target/release/bundle/macos/ClawEnv.app   # macOS
# or run the binary directly

# Windows (requires Rust + Node.js)
cargo tauri build
# Run target\release\bundle\nsis\ClawEnv_*-setup.exe
```

## Architecture

```
GUI (SolidJS + Tauri) ──IPC──► clawenv-cli --json <command>
                                    │
                          ┌─────────┼─────────┐
                          ▼         ▼         ▼
                        Lima      WSL2     Podman
                       (macOS)   (Win)    (Linux)
                          │         │         │
                          └────Alpine Linux───┘
                                    │
                              OpenClaw Agent
```

**CLI-first**: All business logic in `clawenv-cli`. GUI is a thin presentation shell.

## Port Allocation (per instance)

| Offset | Service | Instance 1 | Instance 2 |
|--------|---------|-----------|-----------|
| +0 | Gateway | 3000 | 3020 |
| +1 | Terminal (ttyd) | 3001 | 3021 |
| +2 | MCP Bridge | 3002 | 3022 |
| +3 | CDP (browser) | 3003 | 3023 |
| +4 | VNC (noVNC) | 3004 | 3024 |

## Tech Stack

- **Backend**: Rust 2021, Tokio async
- **GUI**: Tauri v2 (native WebView)
- **Frontend**: SolidJS + TailwindCSS v4
- **CLI**: clap v4 (derive)
- **Config**: TOML
- **Bridge**: Axum HTTP server
- **Sandbox**: Alpine Linux 3.23

## Development

```bash
npm install              # Frontend deps
cargo tauri dev          # Dev mode (hot reload)
cargo test --workspace   # 83 tests
cargo clippy --workspace # Lint
```

## Docs

- [Overview](docs/01-overview.md)
- [Architecture](docs/02-architecture.md)
- [Tech Stack](docs/03-tech-stack.md)
- [Sandbox Implementation](docs/04-sandbox.md)

## License

MIT
