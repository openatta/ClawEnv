# ClawEnv

[English](../README.md)

> OpenClaw AI Agent 的跨平台沙盒安装器、启动器与管理器。

ClawEnv 在你的系统上创建安全隔离的 Alpine Linux 沙盒 —— 基于 **Lima**（macOS）、**WSL2**（Windows）或 **Podman**（Linux）—— 让 AI Agent 安全运行，不影响宿主系统。

## 功能特性

- **一键安装**：GUI 安装向导，含系统检查、代理检测、进度追踪
- **沙盒隔离**：每个实例运行在独立的 Alpine Linux VM/容器中
- **Native 模式**：可选的宿主机直装模式（无沙盒），面向开发者
- **多实例**：运行多个 OpenClaw 实例，端口自动分配（20 端口块）
- **系统托盘**：后台健康监控、通知、快速启停
- **浏览器终端**：每个沙盒 VM 配备 ttyd + xterm.js 终端
- **浏览器 HIL**：Agent 需要人工干预时（验证码、2FA），通过 noVNC 远程操作
- **MCP Bridge**：Agent 通过权限控制的桥接访问宿主文件/命令
- **执行审批**：Agent 执行命令时弹窗确认
- **自动更新检查**：定期检查新版本，提示升级
- **开机自启**：可选的登录启动（默认关闭）

## 快速开始

```bash
# macOS / Linux
cargo tauri build
open target/release/bundle/macos/ClawEnv.app

# Windows（需 Rust + Node.js）
cargo tauri build
# 运行 target\release\bundle\nsis\ClawEnv_*-setup.exe
```

## 架构

**CLI 为核心**：所有业务逻辑在 `clawenv-cli` 中，GUI 是薄壳。

## 端口分配（每实例 20 端口块）

| 偏移 | 服务 | 实例 1 | 实例 2 |
|------|------|--------|--------|
| +0 | Gateway | 3000 | 3020 |
| +1 | 终端 (ttyd) | 3001 | 3021 |
| +2 | MCP Bridge | 3002 | 3022 |
| +3 | CDP (浏览器) | 3003 | 3023 |
| +4 | VNC (noVNC) | 3004 | 3024 |

## 文档

- [项目概述](01-overview.md)
- [核心架构](02-architecture.md)
- [技术栈](03-tech-stack.md)
- [沙盒实现](04-sandbox.md)
