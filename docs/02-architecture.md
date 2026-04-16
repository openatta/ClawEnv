# 2. Architecture

## 2.1 Three-Platform Sandbox Model

WSL2, Lima, and Podman are **peer-level** implementations. Each platform uses its
optimal isolation mechanism. No nesting (no containers inside VMs).

```
Host OS
├── Windows 10/11        ├── macOS 12+          ├── Linux
│   ┌──────────┐         │   ┌──────────┐       │   ┌──────────┐
│   │   WSL2   │         │   │   Lima   │       │   │  Podman  │
│   │ (Hyper-V)│         │   │  (VZ VM) │       │   │(container│
│   │ Alpine   │         │   │ Alpine   │       │   │ Alpine   │
│   │ OpenClaw │         │   │ OpenClaw │       │   │ OpenClaw │
│   └──────────┘         │   └──────────┘       │   └──────────┘
│        ▲               │        ▲             │        ▲
└────────┴───────────────┴────────┴─────────────┴────────┘
                         │
              ┌──────────┴──────────┐
              │      ClawEnv        │
              │  GUI (Tauri) ← IPC → CLI  │
              └─────────────────────┘
```

## 2.2 CLI-First Design (Architecture Law #8)

All business logic runs through `clawcli --json`. The Tauri GUI is a thin
presentation shell that spawns CLI subprocesses via `cli_bridge`.

```
Frontend (SolidJS) → invoke() → Tauri IPC → cli_bridge::run_cli()
                                              → spawn clawcli --json <cmd>
                                              → parse JSON stdout (CliEvent)
                                              → return to frontend
```

This ensures:
- CLI works standalone (headless servers, CI)
- GUI and CLI always share identical logic
- Testing is straightforward (CLI e2e tests)

## 2.3 Port Allocation

Each instance reserves a **20-port block** starting from its gateway port:

| Offset | Service | Default (instance 1) |
|--------|---------|---------------------|
| +0 | Gateway (HTTP/WebSocket) | 3000 |
| +1 | ttyd (terminal) | 3001 |
| +2 | MCP Bridge | 3002 |
| +3 | CDP (Chrome DevTools) | 3003 |
| +4 | VNC WebSocket (noVNC) | 3004 |
| +5~+19 | Reserved | 3005-3019 |

Second instance: 3020, 3021, 3022, ... Third: 3040, etc.

`allocate_port(base, offset)` checks if the target port is free (TcpListener
bind test) and auto-increments within the block if occupied.

## 2.4 Plugin System

Two MCP plugins are auto-deployed into each sandbox during install:

| Plugin | Purpose | Endpoint |
|--------|---------|----------|
| **mcp-bridge** | Host file/exec access from sandbox | Bridge Server /api/* |
| **hil-skill** | Human-in-the-Loop browser intervention | Bridge Server /api/hil/* |

Bridge Server runs on the host (per-instance bridge_port), plugins run inside
the sandbox as MCP stdio servers registered with OpenClaw.

## 2.5 HIL (Human-in-the-Loop) Flow

```
Agent detects CAPTCHA → calls hil_request MCP tool
  → hil-skill.mjs POSTs to Bridge /api/hil/request (blocks)
  → Bridge emits Tauri event → SandboxPage opens NoVncPanel
  → User operates browser via noVNC, clicks "Continue Auto"
  → browser_resume_headless + /api/hil/complete
  → Bridge unblocks → hil-skill returns → Agent continues
```

## 2.6 Exec Approval Flow

```
Agent calls /api/exec with command
  → Bridge checks permissions (allow/deny lists)
  → If require_approval contains "exec":
    → Bridge blocks, emits "exec-approval-required" event
    → App.tsx shows approval dialog with command preview
    → User clicks Approve/Deny
    → Bridge executes or rejects, returns to agent
```

## 2.7 Architecture Laws

1. Sandbox backends are peer-level (WSL2 / Lima / Podman), no nesting
2. Tauri IPC is async — long operations use event streaming
3. Credentials stored in system Keychain (keyring crate)
4. Chromium runs inside sandbox only, noVNC transmits pixels
5. App.tsx LaunchState is the sole top-level router
6. System Tray lifecycle independent of main window
7. Claw management pages use ClawDescriptor for dynamic adaptation
8. CLI is core — GUI is thin shell via cli_bridge
9. All dynamic shell variables escaped via shell_quote() / powershell_quote()
