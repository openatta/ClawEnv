# ClawEnv

[English](../README.md)

> OpenClaw AI Agent 的跨平台沙盒安装器、启动器与管理器。

ClawEnv 在你的系统上创建安全隔离的 Alpine Linux 沙盒 —— 基于 **Lima**（macOS）、**WSL2**（Windows）或 **Podman**（Linux）—— 让 AI Agent 安全运行，不影响宿主系统。

## 为什么选择 ClawEnv？

- **默认安全** —— AI Agent 在隔离沙盒（Alpine Linux VM/容器）中运行，不会触及你的宿主文件和系统，除非你明确授权
- **零依赖** —— ClawEnv 自动下载管理自己的 Node.js 和 Git，无需 Homebrew、无需系统安装器、无需管理员权限
- **导入 / 导出** —— 将整个环境（沙盒镜像或本地 Bundle）打包为单个 `.tar.gz` 文件，一键迁移到其他机器
- **权限可控桥接** —— Agent 仅通过可配置的允许/拒绝列表访问宿主文件和命令，每次执行弹窗确认
- **人机协作 (HIL)** —— 当 Agent 遇到验证码或 2FA 时，浏览器切换到交互模式（noVNC），你可以介入处理后继续自动化
- **多实例并行** —— 同时运行多个 OpenClaw 实例，各自独立的 20 端口块、配置和生命周期

## 下载

| 平台 | 下载 |
|------|------|
| macOS（Apple Silicon） | [ClawEnv_0.2.0_aarch64.dmg](https://github.com/openatta/ClawEnv/releases/tag/v0.2.0) |
| Windows（ARM64） | [ClawEnv_0.2.0_arm64-setup.exe](https://github.com/openatta/ClawEnv/releases/tag/v0.2.0) |

## 功能特性

- **一键安装** —— GUI 安装向导，系统检查、代理检测、进度追踪
- **沙盒隔离** —— 每个实例运行在独立的 Alpine Linux VM/容器中
- **Native 模式** —— 可选宿主机直装模式（无 VM 开销），面向开发者
- **导入 / 导出** —— 沙盒镜像和本地 Bundle，带文件校验
- **系统托盘** —— 后台健康监控、通知、退出选项
- **浏览器终端** —— 每个沙盒 VM 配备 ttyd + xterm.js
- **浏览器 HIL** —— 通过 noVNC 远程操作处理验证码/2FA
- **MCP Bridge** —— 权限控制的宿主文件/命令访问
- **执行审批** —— Agent 命令需用户确认
- **诊断工具** —— 检查实例/配置一致性，自动修复
- **开机自启** —— 可选登录启动
- **双语 UI** —— 中英文

## 架构

```
GUI (SolidJS + Tauri) ──IPC──► clawcli --json <命令>
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

**CLI 为核心**：所有业务逻辑在 `clawcli` 中，GUI 是薄壳。

## 文档

- [项目概述](01-overview.md)
- [核心架构](02-architecture.md)
- [技术栈](03-tech-stack.md)
- [沙盒实现](04-sandbox.md)
- [打包与发布](05-packaging.md)
- [智能硬件对接](16-hardware-integration.md)
- [构建指南](17-build-guide.md)
