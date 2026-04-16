# 6. ClawEnv Lite — Offline Installer for End Users

## Overview

ClawEnv Lite is a simplified, offline-only version for non-technical users.
Users receive a folder containing the Lite app + pre-packaged environment
bundles. Double-click → select package → install → use.

## Distribution Structure

```
ClawEnv-Lite/
├── ClawEnv-Lite.app (macOS) or ClawEnv-Lite.exe (Windows)
├── macos-aarch64-20260415.tar.gz     (optional: native bundle)
├── lima-aarch64-20260415.tar.gz      (optional: sandbox image)
├── windows-aarch64-20260415.tar.gz   (optional: native bundle)
├── wsl2-aarch64-20260415.tar.gz      (optional: sandbox image)
└── README.txt
```

## Install Flow (3 steps)

### Step 1: Package Selection
- Scan app's directory for `*.tar.gz` files
- Parse filename: `{platform}-{arch}-{timestamp}.tar.gz`
- Filter by current platform + architecture
- Show radio list with type (Native/Sandbox), size
- Sandbox prerequisites check (Lima/WSL2 availability)

### Step 2: Proxy Configuration (sandbox only)
- Auto-detect system proxy
- Manual input option
- Skip for native mode (uses system proxy automatically)

### Step 3: Install
- Extract tar.gz to `~/.clawenv/`
- Write config.toml
- Start gateway
- Show staged progress

## Management GUI (post-install)

Single instance view — no Home page, no multi-instance tabs.

Features retained:
- Start / Stop / Restart with operation modal
- Open Control Panel (browser)
- Configure (ports, etc.)
- Export backup
- Delete instance
- Terminal (sandbox only)
- Chromium install (sandbox only)
- Browser HIL (sandbox only)
- Settings (proxy + diagnostics)

Features removed vs full version:
- No Home page
- No "+ Add" instance button
- No install wizard (replaced by package scanner)
- No online install
- No multi-instance support

## Code Structure

```
lite/
├── src/
│   ├── LiteApp.tsx       # Entry: install or manage
│   ├── LiteInstall.tsx   # 3-step offline installer
│   └── LiteMain.tsx      # Single instance management
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json
└── tauri.conf.json       # Lite Tauri config
```

Shares with main app:
- `core/` — 100% reused (all backend logic)
- `cli/` — 100% reused
- `tauri/src/` — 100% reused (all IPC handlers)
- `src/components/` — reused (Terminal, NoVncPanel, ExportProgress, etc.)
- `src/i18n.ts` — reused
- `src/types.ts` — reused

## Build

```bash
cd lite && npm install && cargo tauri build
```

Output: `ClawEnv-Lite.app` (macOS) / `ClawEnv-Lite-setup.exe` (Windows)

## Platform Package Detection

| File Pattern | Type | Platform |
|-------------|------|----------|
| `macos-aarch64-*.tar.gz` | Native Bundle | macOS ARM |
| `macos-x86_64-*.tar.gz` | Native Bundle | macOS Intel |
| `lima-aarch64-*.tar.gz` | Sandbox Image | macOS ARM |
| `lima-x86_64-*.tar.gz` | Sandbox Image | macOS Intel |
| `windows-aarch64-*.tar.gz` | Native Bundle | Windows ARM |
| `windows-x86_64-*.tar.gz` | Native Bundle | Windows x64 |
| `wsl2-aarch64-*.tar.gz` | Sandbox Image | Windows ARM |
| `wsl2-x86_64-*.tar.gz` | Sandbox Image | Windows x64 |
