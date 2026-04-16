# ClawEnv

[![CI](https://github.com/openatta/ClawEnv/actions/workflows/ci.yml/badge.svg)](https://github.com/openatta/ClawEnv/actions/workflows/ci.yml)

[中文文档](docs/README-zh.md)

> Cross-platform sandbox installer, launcher & manager for OpenClaw AI agents.

ClawEnv creates a secure, isolated Alpine Linux sandbox on your system — powered by **Lima** (macOS), **WSL2** (Windows), or **Podman** (Linux) — so AI agents run safely without affecting your host OS.

## Why ClawEnv?

- **Secure by default** — AI agents run inside an isolated sandbox (Alpine Linux VM/container), never touching your host files or system unless you explicitly allow it
- **Zero dependencies** — ClawEnv downloads and manages its own Node.js and Git; no Homebrew, no system installers, no admin privileges needed
- **Import / Export** — Package your entire environment (sandbox image or native bundle) as a single `.tar.gz` file; move it between machines with one click
- **Permission-controlled bridge** — Agents access host files and commands only through a configurable allowlist/denylist with user approval popups
- **Human-in-the-Loop** — When an agent hits a CAPTCHA or 2FA, the browser switches to interactive mode (noVNC) so you can step in and continue
- **Multi-instance** — Run multiple OpenClaw instances side by side, each with its own 20-port block, configuration, and lifecycle

## Download

| Platform | Download |
|----------|----------|
| macOS (Apple Silicon) | [ClawEnv_0.2.0_aarch64.dmg](https://github.com/openatta/ClawEnv/releases/tag/v0.2.0) |
| Windows (ARM64) | [ClawEnv_0.2.0_arm64-setup.exe](https://github.com/openatta/ClawEnv/releases/tag/v0.2.0) |

## Features

- **One-Click Install** — GUI wizard with system checks, proxy detection, progress tracking
- **Sandbox Isolation** — Each instance in its own Alpine Linux VM/container
- **Native Mode** — Optional host-native install for developers (no VM overhead)
- **Import / Export** — Sandbox images and native bundles, with file validation
- **System Tray** — Background health monitoring, notifications, quit options
- **In-Browser Terminal** — ttyd + xterm.js per sandbox VM
- **Browser HIL** — Human-in-the-Loop via noVNC for CAPTCHA/2FA
- **MCP Bridge** — Host file/command access with permission control
- **Exec Approval** — Agent commands require user confirmation
- **Diagnostics** — Check instance/config consistency and auto-repair
- **Autostart** — Optional launch-at-login
- **Bilingual UI** — Chinese and English

## Architecture

```
GUI (SolidJS + Tauri) ──IPC──► clawcli --json <command>
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

**CLI-first**: All business logic in `clawcli`. GUI is a thin presentation shell.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend | Rust 2021, Tokio async |
| GUI | Tauri v2 (native WebView) |
| Frontend | SolidJS + TailwindCSS v4 |
| CLI | clap v4 |
| Bridge | Axum HTTP server |
| Sandbox | Alpine Linux 3.23 |

## Docs

- [Overview](docs/01-overview.md)
- [Architecture](docs/02-architecture.md)
- [Tech Stack](docs/03-tech-stack.md)
- [Sandbox Implementation](docs/04-sandbox.md)
- [Packaging & Distribution](docs/05-packaging.md)

## License

MIT
