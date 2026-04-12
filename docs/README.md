# ClawEnv — 技术规格文档 v1.3

> 为 Claw 生态（OpenClaw、ZeroClaw、AutoClaw 等 40+ 产品）提供跨平台沙盒安装、隔离与管理的统一工具
> 项目状态：开发阶段 | 文档版本：2026-04-10

---

## 文档索引

| # | 文档 | 内容 |
|---|------|------|
| 1 | [项目概述](01-overview.md) | 背景、核心问题、项目目标、可行性分析、风险评估 |
| 2 | [核心架构](02-architecture.md) | 三平台对等沙盒模型、SandboxBackend Trait |
| 3 | [技术栈](03-tech-stack.md) | Rust/Tauri/SolidJS 技术选型、Crate 依赖、前端依赖 |
| 4 | [沙盒实现](04-sandbox.md) | WSL2/Lima/Podman 三平台详细实现、浏览器集成 |
| 5 | [启动器与路由](05-launcher.md) | 启动流程状态机、安装检测、升级提示、页面路由 |
| 6 | [主 UI 设计](06-main-ui.md) | Slack 风格布局、左侧图标栏（动态 Claw 类型）、Home/ClawPage/设置页 |
| 7 | [功能规格](07-features.md) | 安装流程、升级与安全管理、用户模式、CLI 命令集 |
| 8 | [System Tray](08-system-tray.md) | 托盘图标状态、菜单、通知策略、监控机制 |
| 9 | [配置与目录](09-config.md) | config.toml 规范、源码结构、用户数据目录 |
| 10 | [安全模型](10-security.md) | 凭证安全、沙盒隔离、CVE 响应、快照策略 |
| 11 | [开发路线图](11-roadmap.md) | Phase 1–4、开发指引、环境搭建、关键约束 |
| 13 | [Claw 生态注册表](13-claw-registry.md) | 47 款 Claw 产品评测、安装方式分类、ClawEnv 支持矩阵 |
| 14 | [国产 Claw 包装关系分析](14-claw-repackaging-analysis.md) | 证据链：国内 6 大 claw 产品均为 OpenClaw 封装 |
| 15 | [Windows ARM64 交叉开发](15-cross-dev-windows.md) | SSH 远程构建/测试、环境搭建、win-remote.sh 使用 |
