# ClawEnv

[中文文档](docs/README-zh.md)

> Cross-platform sandbox installer, launcher & manager for the Claw ecosystem (OpenClaw, NanoClaw, and more).

ClawEnv creates a secure, isolated Alpine Linux sandbox on your system — powered by **Lima** (macOS), **WSL2** (Windows), or **Podman** (Linux) — so AI agents run safely without affecting your host OS.

## Features

- **Multi-Claw Support** — Install and manage any Claw product via a pluggable [ClawDescriptor](assets/claw-registry.toml) registry. Add new products with zero code changes.
- **Three-Platform Parity** — Identical experience across macOS, Windows, and Linux. Same sandbox model, same UI, same CLI.
- **One-Click Install** — Guided wizard handles sandbox creation, package installation, API key storage, and gateway startup.
- **Dynamic UI** — SolidJS frontend with instance-driven icon bar, claw type picker, and per-instance management pages.
- **Mirror Configuration** — One-click `preset = "china"` for domestic Alpine/npm/Node.js mirrors. Custom mirrors supported.
- **Native Bundle** — Offline installation via pre-packaged Node.js + node_modules bundles.
- **System Tray** — Background operation with health monitoring, instance controls, and notifications.

## Architecture

```
┌──────────────────────────────────────────────────┐
│                  Host OS                          │
│                                                   │
│  Windows 11        macOS 12+         Linux        │
│  ┌──────────┐   ┌──────────┐   ┌──────────┐     │
│  │   WSL2   │   │   Lima   │   │  Podman  │     │
│  │ (Alpine) │   │ (Alpine) │   │ (Alpine) │     │
│  │  Claw ☆  │   │  Claw ☆  │   │  Claw ☆  │     │
│  └──────────┘   └──────────┘   └──────────┘     │
│        ▲              ▲              ▲            │
│        └──────────────┴──────────────┘            │
│                       │                           │
│            ┌──────────┴──────────┐                │
│            │      ClawEnv       │                │
│            │  Rust + Tauri v2   │                │
│            │  GUI ◄──IPC──► CLI │                │
│            └────────────────────┘                │
└──────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

| Platform | Requirement |
|----------|-------------|
| macOS | Lima (auto-installed) |
| Windows | WSL2 (auto-installed with UAC prompt) |
| Linux | Podman |

### Install & Run

```bash
# Clone
git clone https://github.com/openatta/ClawEnv.git
cd ClawEnv

# Install frontend dependencies
npm install

# Development mode
cargo tauri dev

# Production build
cargo tauri build
```

### CLI

```bash
# Install OpenClaw in a sandbox
clawenv install --claw-type openclaw --name default

# Install a different claw product
clawenv install --claw-type nanoclaw --name secure-agent

# List instances
clawenv list

# Start/stop
clawenv start --name default
clawenv stop --name default
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend | Rust 2021 edition |
| GUI | Tauri v2 (native WebView) |
| Frontend | SolidJS + TailwindCSS v4 + TypeScript |
| CLI | clap v4 (derive mode) |
| Sandbox | Alpine Linux — Lima / WSL2 / Podman |
| Config | TOML (`~/.clawenv/config.toml`) |

## Project Structure

```
core/            # Core logic (platform-agnostic, no UI deps)
  src/claw/      #   ClawDescriptor + ClawRegistry
  src/sandbox/   #   WSL2 / Lima / Podman backends
  src/manager/   #   Install / upgrade / instance management
  src/config/    #   Config models, mirrors, proxy, keychain
tauri/           # Tauri GUI app (System Tray, IPC handlers)
cli/             # CLI (developer mode)
src/             # Frontend SolidJS
  components/    #   IconBar, UpgradePrompt, Terminal
  pages/         #   Home, ClawPage, SandboxPage, Settings, Install
  layouts/       #   MainLayout
assets/          # Lima templates, Containerfile, claw-registry.toml
scripts/         # Test framework, packaging, Windows remote helper
docs/            # Specification docs (16 files)
```

## Testing

```bash
# L1+L2: Unit tests + mock flow tests (< 1 second)
cargo test -p clawenv-core

# L3: Real sandbox lifecycle test
bash scripts/test-claw-lifecycle.sh openclaw

# Full test suite with parallel runner
bash scripts/test-claw-runner.sh --parallel 2

# Windows remote test
bash scripts/win-remote.sh test
```

See [scripts/README.md](scripts/README.md) for the complete testing guide.

## Claw Registry

ClawEnv supports any Claw product defined in [`assets/claw-registry.toml`](assets/claw-registry.toml). Currently verified:

| Product | Status | Notes |
|---------|--------|-------|
| **OpenClaw** | ✅ Verified | Full lifecycle tested, v2026.4.10 |
| **NanoClaw** | Registered | Security-focused alternative |

See [docs/13-claw-registry.md](docs/13-claw-registry.md) for the full ecosystem analysis (47 products).

## Documentation

| # | Document | Content |
|---|----------|---------|
| 1 | [Overview](docs/01-overview.md) | Background, goals, feasibility |
| 2 | [Architecture](docs/02-architecture.md) | Three-platform sandbox model |
| 3 | [Tech Stack](docs/03-tech-stack.md) | Rust/Tauri/SolidJS choices |
| 4 | [Sandbox](docs/04-sandbox.md) | WSL2/Lima/Podman implementation |
| 5 | [Launcher](docs/05-launcher.md) | Launch state machine |
| 6 | [Main UI](docs/06-main-ui.md) | Slack-style layout, dynamic IconBar |
| 13 | [Claw Registry](docs/13-claw-registry.md) | 47 products, verification matrix |
| 14 | [Repackaging Analysis](docs/14-claw-repackaging-analysis.md) | Domestic products are OpenClaw wrappers |
| 15 | [Windows Cross-Dev](docs/15-cross-dev-windows.md) | SSH remote build/test guide |

## License

MIT
