# 17. Build Guide

ClawEnv 的本地构建指南，覆盖 macOS 和 Windows 两个平台。

## 17.1 Architecture Overview

构建产物分为两部分：

| 产物 | 说明 | 构建工具 |
|------|------|---------|
| **clawcli** | CLI 二进制（核心业务逻辑） | `cargo build -p clawcli` |
| **ClawEnv.app / .exe** | Tauri GUI（嵌入 clawcli 作为 sidecar） | `npx tauri build` |

```
构建流程:
  1. cargo build -p clawcli --release    → target/release/clawcli
  2. copy-cli-sidecar.cjs                → tauri/binaries/clawcli-{triple}
  3. npm run build                       → dist/ (SolidJS frontend)
  4. cargo tauri build                   → bundle (DMG / NSIS installer)
```

## 17.2 Prerequisites

### Common (All Platforms)

| 工具 | 版本要求 | 用途 |
|------|---------|------|
| **Rust** | core+cli: 1.87+, Tauri: 1.88+ | Rust 编译 |
| **Node.js** | 20+ | 前端编译 + Tauri CLI |
| **npm** | 随 Node.js | 依赖管理 |
| **Git** | any | 代码克隆 |

### macOS

```bash
# Xcode Command Line Tools
xcode-select --install

# Rust (如果还没有)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Node.js (推荐 nvm 管理)
brew install node
# 或
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
nvm install 20
```

### Windows

1. **Visual Studio Build Tools** — 安装 "C++ 桌面开发" 工作负载
   - 下载: https://visualstudio.microsoft.com/visual-cpp-build-tools/
   - 安装时勾选: "Desktop development with C++"

2. **Rust**
   ```powershell
   # 下载并运行 rustup-init.exe
   winget install Rustlang.Rustup
   # 或从 https://rustup.rs 下载
   ```

3. **Node.js**
   ```powershell
   winget install OpenJS.NodeJS.LTS
   # 或从 https://nodejs.org 下载
   ```

4. **WebView2 Runtime** — Windows 10/11 通常已预装
   - 下载: https://developer.microsoft.com/microsoft-edge/webview2/

## 17.3 Build Scripts

项目提供了一键构建脚本，无需了解内部构建流程。

### macOS

```bash
# 完整 release 构建 (GUI + CLI, 输出 DMG)
bash scripts/build-macos.sh

# 开发构建 (debug 模式, 更快)
bash scripts/build-macos.sh --dev

# 只构建 CLI (不需要 Node.js 前端)
bash scripts/build-macos.sh --cli-only
```

### Windows

```powershell
# 完整 release 构建 (GUI + CLI, 输出 NSIS installer)
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1

# 开发构建
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -Mode dev

# 只构建 CLI
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -CliOnly
```

## 17.4 Manual Build Steps

如果不使用构建脚本，按以下步骤手动操作。

### Step 1: Clone & Install Dependencies

```bash
git clone https://github.com/openatta/ClawEnv.git
cd ClawEnv
npm install
```

### Step 2: Build CLI

```bash
# Debug (快速, 用于开发)
cargo build -p clawcli

# Release (优化, 用于发布)
cargo build -p clawcli --release
```

验证:
```bash
./target/debug/clawcli --version
./target/debug/clawcli --json system-check
```

### Step 3: Build Tauri GUI

**重要**: Tauri 构建需要 rustc 1.88+。如果系统 rustc 低于此版本:
```bash
# macOS: 确保 ~/.cargo/bin 在 PATH 前面
export PATH="$HOME/.cargo/bin:$PATH"
rustc --version   # 应 >= 1.88
```

```bash
# 开发模式 (热重载)
npx tauri dev

# Release 构建 (生成安装包)
npx tauri build
```

### Step 4: Locate Artifacts

| 平台 | 产物路径 |
|------|---------|
| macOS DMG | `tauri/target/release/bundle/dmg/ClawEnv_*.dmg` |
| macOS App | `tauri/target/release/bundle/macos/ClawEnv.app` |
| Windows NSIS | `tauri\target\release\bundle\nsis\ClawEnv_*-setup.exe` |
| Windows MSI | `tauri\target\release\bundle\msi\ClawEnv_*.msi` |
| CLI (macOS) | `target/release/clawcli` |
| CLI (Windows) | `target\release\clawcli.exe` |

## 17.5 Running Tests

```bash
# 单元测试 (core + cli, 快速, ~5s)
cargo test -p clawenv-core -p clawcli

# 全 workspace 测试 (需要 rustc 1.88+ for Tauri)
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace

# macOS 沙盒集成测试 (创建真实 Lima VM, ~15-25min)
bash scripts/test-macos-sandbox.sh

# macOS Native 模式测试
bash scripts/test-macos-native.sh

# Windows 沙盒集成测试 (需要 WSL2)
bash scripts/test-windows-sandbox.sh
```

## 17.6 Rust Version Notes

项目使用两个 Rust 版本:

| 组件 | 最低版本 | 原因 |
|------|---------|------|
| **core + cli** | rustc 1.87+ | Homebrew 默认版本即可 |
| **tauri (GUI)** | rustc 1.88+ | `darling`, `time` crate 依赖 |

macOS 上 Homebrew 安装的 rustc 通常是 1.87.x，而 `rustup` 安装的在 `~/.cargo/bin/` 下是最新版。构建脚本会自动检测并切换。

如果遇到版本问题:
```bash
# 更新 rustup 管理的 Rust
rustup update stable

# 确认版本
~/.cargo/bin/rustc --version
```

## 17.7 Troubleshooting

### macOS: `darling` / `time` crate requires rustc 1.88

```
error: rustc 1.87.0 is not supported by darling@0.23.0
```

**解决**: 确保 `~/.cargo/bin` 在 PATH 前面:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

### Windows: `link.exe not found`

**解决**: 安装 Visual Studio Build Tools 的 C++ 工作负载。

### macOS: `npm install -g` 在沙盒中报 EACCES

全量安装流程自动使用 sudo，不受影响。如果使用 `--step claw` 分步安装遇到此问题，已在最新代码中修复。

### 客户报 "no such file or directory (OS error 2)" 创建 VM

Lima 未安装或不在 PATH 中。最新代码已修复自动安装后的 PATH 更新问题。如果仍然出现:
```bash
# 手动安装 Lima
brew install lima
# 验证
limactl --version
```
