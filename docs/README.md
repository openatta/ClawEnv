# ClawEnv 文档索引

ClawEnv 项目的所有设计文档 (SSOT)。按主题分组，**编号不保证连续** — 文档编号稳定，个别编号空缺是因为老的文档被重构合并了。

修改代码前应先读对应主题的 SSOT；修改 SSOT 的时候要同步落地到代码，避免文档/实现漂移。

> 用户面向的说明文档在 [`README-zh.md`](README-zh.md)（中文）和仓库根的 [`README.md`](../README.md)（英文）。这一份是给 **开发者 / 维护者** 看的。

## 架构与总览

| # | 文档 | 内容 |
|---|---|---|
| 01 | [项目概览](01-overview.md) | ClawEnv 是什么、核心价值、与 OpenClaw 的关系 |
| 02 | [架构](02-architecture.md) | 沙盒三后端对等模型、工厂函数、核心铁律 (**最重要**) |
| 03 | [技术栈](03-tech-stack.md) | Rust/Tauri/SolidJS 选型理由、依赖矩阵 |

## 实现细节

| # | 文档 | 内容 |
|---|---|---|
| 04 | [沙盒实现](04-sandbox.md) | Lima/WSL2/Podman + Native 的具体实现 + 浏览器 + noVNC |
| 05 | [打包与分发](05-packaging.md) | 镜像制作、bundle 命名历史背景（v0.2.6+ 以 18 为准） |
| 06 | [ClawEnv Lite](06-lite.md) | 面向终端用户的离线安装器 |

## Claw 产品与硬件

| # | 文档 | 内容 |
|---|---|---|
| 14 | [Claw 重打包分析](14-claw-repackaging-analysis.md) | 国内 Claw 产品 (OpenClaw / Hermes / ...) 打包关系调研 |
| 16 | [硬件集成](16-hardware-integration.md) | 智能硬件设备对接 Agent 的架构与实施 |

## 构建与跨平台

| # | 文档 | 内容 |
|---|---|---|
| 15 | [Windows 交叉开发](15-cross-dev-windows.md) | 从 macOS SSH 远控 Windows ARM64 (UTM) 构建测试 |
| 17 | [构建指南](17-build-guide.md) | 本地构建步骤（macOS / Windows） |

## 协议与契约

| # | 文档 | 内容 |
|---|---|---|
| 18 | [Bundle 格式规范](18-bundle-format.md) | export/import 的 `.tar.gz` 契约，manifest schema，wrap 结构，V1→V2 演进方案 |

## 约定

- **编号**：01-xx 先按功能域分块（01-09 架构，10-19 协议/实现）。不强制连续，合并/废弃不回收编号。
- **语言**：总体 bilingual。老文档英文为主，新文档（14 起）中文为主，少量混写。不强制翻译。
- **SSOT 纪律**：代码注释可以说 "see docs/02-architecture.md"；docs 可以引用具体代码文件+行号。单向真理 — 冲突时以代码为准，然后立即更新文档。
- **加新文档**：取下一个空编号，起描述性的 kebab-case slug，把条目加到本表。

## 文档之外的 SSOT

- [`CLAUDE.md`](../CLAUDE.md) — AI 协作规则 + 技术栈速查。修改后 Claude 的所有 session 立即生效。
- [`scripts/TEST-PLAN.md`](../scripts/TEST-PLAN.md) — 手工测试清单，发版前按这个跑。
