# ClawEnv

[English](../README.md)

> Claw 生态（OpenClaw、NanoClaw 等）的跨平台沙盒安装器、启动器与管理器。

ClawEnv 在你的系统上创建安全隔离的 Alpine Linux 沙盒——基于 **Lima**（macOS）、**WSL2**（Windows）或 **Podman**（Linux）——让 AI Agent 安全运行而不影响宿主系统。

## 功能特性

- **多 Claw 支持** — 通过可插拔的 [ClawDescriptor](../assets/claw-registry.toml) 注册表安装管理任何 Claw 产品。新增产品零代码改动。
- **三平台对等** — macOS、Windows、Linux 体验完全一致。同一沙盒模型、同一 UI、同一 CLI。
- **一键安装** — 引导式向导处理沙盒创建、包安装、API Key 存储和网关启动。
- **动态 UI** — SolidJS 前端，实例驱动的图标栏、Claw 类型选择器、逐实例管理页面。
- **镜像源配置** — 一键 `preset = "china"` 切换国内 Alpine/npm/Node.js 源。支持自定义源。
- **离线安装包** — 通过预打包的 Node.js + node_modules 包离线安装。
- **系统托盘** — 后台运行，健康监控、实例控制、通知推送。

## 架构

```
┌──────────────────────────────────────────────────┐
│                   宿主操作系统                      │
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

## 快速开始

### 前提条件

| 平台 | 需要 |
|------|------|
| macOS | Lima（自动安装） |
| Windows | WSL2（自动弹出 UAC 提示安装） |
| Linux | Podman |

### 安装与运行

```bash
# 克隆
git clone https://github.com/openatta/ClawEnv.git
cd ClawEnv

# 安装前端依赖
npm install

# 开发模式
cargo tauri dev

# 生产构建
cargo tauri build
```

### CLI 命令

```bash
# 在沙盒中安装 OpenClaw
clawenv install --claw-type openclaw --name default

# 安装其他 claw 产品
clawenv install --claw-type nanoclaw --name secure-agent

# 列出实例
clawenv list

# 启动/停止
clawenv start --name default
clawenv stop --name default
```

## 技术栈

| 层级 | 技术 |
|------|------|
| 后端 | Rust 2021 edition |
| GUI | Tauri v2（系统原生 WebView） |
| 前端 | SolidJS + TailwindCSS v4 + TypeScript |
| CLI | clap v4（derive 模式） |
| 沙盒 | Alpine Linux — Lima / WSL2 / Podman |
| 配置 | TOML（`~/.clawenv/config.toml`） |

## 项目结构

```
core/            # 核心逻辑（平台无关，无 UI 依赖）
  src/claw/      #   ClawDescriptor + ClawRegistry
  src/sandbox/   #   WSL2 / Lima / Podman 后端实现
  src/manager/   #   安装 / 升级 / 实例管理
  src/config/    #   配置模型、镜像源、代理、钥匙串
tauri/           # Tauri GUI 应用（系统托盘、IPC 处理器）
cli/             # CLI（开发者模式）
src/             # 前端 SolidJS
  components/    #   IconBar、UpgradePrompt、Terminal
  pages/         #   Home、ClawPage、SandboxPage、Settings、Install
  layouts/       #   MainLayout
assets/          # Lima 模板、Containerfile、claw-registry.toml
scripts/         # 测试框架、打包脚本、Windows 远程助手
docs/            # 规格文档（16 个文件）
```

## 测试

```bash
# L1+L2: 单元测试 + Mock 流程测试（< 1 秒）
cargo test -p clawenv-core

# L3: 真实沙盒生命周期测试
bash scripts/test-claw-lifecycle.sh openclaw

# 并行测试运行器
bash scripts/test-claw-runner.sh --parallel 2

# Windows 远程测试
bash scripts/win-remote.sh test
```

详见 [scripts/README.md](../scripts/README.md) 完整测试指南。

## Claw 注册表

ClawEnv 支持 [`assets/claw-registry.toml`](../assets/claw-registry.toml) 中定义的任何 Claw 产品。当前已验证：

| 产品 | 状态 | 说明 |
|------|------|------|
| **OpenClaw** | ✅ 已验证 | 完整生命周期测试通过，v2026.4.10 |
| **NanoClaw** | 已注册 | 安全增强版替代方案 |

详见 [docs/13-claw-registry.md](13-claw-registry.md) 完整生态分析（47 款产品）。

## 文档索引

| # | 文档 | 内容 |
|---|------|------|
| 1 | [项目概述](01-overview.md) | 背景、目标、可行性 |
| 2 | [核心架构](02-architecture.md) | 三平台对等沙盒模型 |
| 3 | [技术栈](03-tech-stack.md) | Rust/Tauri/SolidJS 选型 |
| 4 | [沙盒实现](04-sandbox.md) | WSL2/Lima/Podman 详细实现 |
| 5 | [启动器](05-launcher.md) | 启动流程状态机 |
| 6 | [主 UI](06-main-ui.md) | Slack 风格布局、动态 IconBar |
| 13 | [Claw 注册表](13-claw-registry.md) | 47 款产品、验证矩阵 |
| 14 | [国产包装分析](14-claw-repackaging-analysis.md) | 国内产品均为 OpenClaw 封装 |
| 15 | [Windows 交叉开发](15-cross-dev-windows.md) | SSH 远程构建/测试指南 |

## 许可证

MIT
