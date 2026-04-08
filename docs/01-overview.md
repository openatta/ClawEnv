# 1. 项目概述与可行性分析

## 1.1 背景

OpenClaw 是 2026 年增长最快的开源 AI Agent 框架（GitHub Stars 超过 25 万），基于 Node.js，
支持 WhatsApp、Telegram、Discord 等消息平台接入，具备 Shell 执行、文件操作、CDP 浏览器自动化等能力。

围绕 OpenClaw 形成了"claw 生态"，ClawEnv 目前聚焦 OpenClaw，预留对其他 claw 工具的扩展能力：

| 工具 | 语言 | 定位 | ClawEnv 支持计划 |
|---|---|---|---|
| OpenClaw | Node.js | 主流，功能最全 | Phase 1 |
| ZeroClaw | Rust | 边缘/IoT，极低资源 | Phase 4 |
| NanoClaw | Python | 轻量，可审计 | Phase 4 |
| Knolli | 闭源 SaaS | 企业级 | 暂不支持 |

## 1.2 核心问题

OpenClaw 现有安装体验存在三类问题：

**环境问题**：要求 Node.js 22+，对非技术用户门槛高；无环境隔离，直接污染主机。

**安全问题**：凭证明文存储于 `~/.openclaw/`；CVE-2026-25253（CVSS 8.8）一键 RCE 漏洞
通过 WebSocket 泄露 Gateway 鉴权 Token，已修复但需用户手动更新。

**管理问题**：无版本管理工具，升级、回滚、多实例均需手动操作；无安全公告推送机制。

## 1.3 项目目标

ClawEnv 提供：

- 美观的 GUI 安装向导（Rust + Tauri，基于系统原生 WebView）
- 三平台对等的安全隔离沙盒（Alpine Linux 统一底座）
- 完整生命周期管理：安装、升级、回滚、多实例
- 开发者工具：CLI、Skill 脚手架、本地调试
- 浏览器集成助手（Phase 2：CDT；Phase 4：指纹浏览器）

## 1.4 平台最低版本要求

| 平台 | 最低版本 | 沙盒后端 | 说明 |
|------|---------|---------|------|
| **macOS** | macOS 11 (Big Sur)+ | Lima | Tauri WebView (WKWebView) + Lima VZ |
| **Windows** | Windows 10 2004 (Build 19041)+ | WSL2 | WSL2 首次可用版本，Tauri WebView2 |
| **Ubuntu** | 22.04 LTS+ | Podman | WebKitGTK 4.1, Podman 3.0+ |
| **Fedora** | 36+ | Podman | WebKitGTK 4.1, Podman 预装 |

所有平台均支持 Native 安装模式（无沙盒，直接安装 Node.js + OpenClaw）。

---

## 2. 可行性分析

### 2.1 技术可行性

**Rust + Tauri：完全可行，当前最优选择**

Tauri v2 使用系统原生 WebView（Windows: WebView2，macOS: WKWebView，Linux: WebKitGTK），
最终二进制 3–10MB（Electron 需要 80–200MB）。Rust 后端直接调用各平台系统 API，
性能与可靠性均优于 Go 等其他选项。

**Alpine Linux 作为统一沙盒底座：理想选择**

- 基础镜像仅 ~5MB
- 使用 musl libc + busybox，极度精简
- `apk add nodejs npm` 即可运行 OpenClaw
- 安全设计理念强，与 OpenClaw 隔离需求高度契合
- 三个平台均有完善的 Alpine 支持路径

**三平台沙盒机制：均有成熟方案**

| 平台 | 沙盒机制 | 成熟度 |
|---|---|---|
| Windows | WSL2 + Alpine distro | 高，微软官方支持 |
| macOS | Lima + Alpine VM | 高，社区活跃 |
| Linux | Podman + Alpine 容器 | 高，官方支持 |

### 2.2 市场可行性

OpenClaw 正处于爆发增长阶段，大量非技术用户涌入，现有安装文档假设用户熟悉 Node.js 生态，
普通用户体验极差。安全隐患（CVE-2026-25253）使企业用户急需隔离方案。
同类工具（nvm、pyenv、mise）已验证环境管理工具的市场需求。

### 2.3 风险评估

| 风险 | 概率 | 影响 | 缓解策略 |
|---|---|---|---|
| OpenClaw API 变更 | 中 | 高 | 版本锁定 + 适配层 |
| Lima + Alpine cgroup v2 兼容性 | 中 | 中 | 使用 alpine-lima 社区模板，包含已知 fix |
| Tauri WebView 平台差异 | 低 | 低 | Tauri v2 已大幅改善跨平台一致性 |
| 指纹浏览器集成复杂度 | 高 | 低 | 明确为 Phase 4，当前预留接口不实现 |
