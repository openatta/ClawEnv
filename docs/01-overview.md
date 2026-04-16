# 1. Project Overview

## 1.1 What is ClawEnv

ClawEnv is a cross-platform sandbox installer, launcher, and manager for OpenClaw
(and the broader claw ecosystem). It provides a native GUI + CLI for managing AI
agent instances in isolated environments.

**Core value**: One-click install of OpenClaw in a secure sandbox, with full
lifecycle management (install, start/stop, upgrade, delete), system tray
integration, and developer tools.

## 1.2 Supported Platforms

| Platform | Min Version | Sandbox Backend | Native Mode |
|----------|------------|-----------------|-------------|
| macOS | 11 (Big Sur)+ | Lima (VZ) | Yes |
| Windows | 10 2004+ | WSL2 | Yes |
| Linux | Ubuntu 22.04+ / Fedora 36+ | Podman | Yes |

All platforms also support **Native mode** (no sandbox, OpenClaw installed
directly on the host via Node.js).

## 1.3 Key Features (v0.2.0)

- **GUI Install Wizard**: 7-step guided installation with system checks
- **Sandbox Isolation**: Alpine Linux VM per instance (Lima / WSL2 / Podman)
- **Multi-Instance**: Multiple OpenClaw instances with 20-port block allocation
- **CLI-First Architecture**: All business logic via `clawcli`, GUI is thin shell
- **System Tray**: Background monitoring, health notifications, quick actions
- **Terminal**: In-browser terminal (ttyd + xterm.js) per sandbox VM
- **Browser HIL**: Human-in-the-Loop via noVNC when agent needs manual help
- **MCP Bridge**: Host file/exec access from sandbox via MCP protocol
- **Exec Approval**: Agent exec commands require user approval (configurable)
- **Auto-Update Check**: Periodic npm registry polling with upgrade prompts
- **Autostart**: OS-level launch-at-login (LaunchAgent / Registry / .desktop)

## 1.4 Workspace Structure

```
core/            # Core logic (platform-agnostic, no UI deps)
tauri/           # Tauri v2 GUI app (System Tray)
cli/             # Pure CLI (clawcli)
src/             # Frontend SolidJS + TailwindCSS
assets/          # Lima templates, icons, MCP plugin bundles
plugins/         # MCP Bridge + HIL Skill source (TypeScript)
scripts/         # Build helpers, test scripts
docs/            # Spec documents (this directory)
```
