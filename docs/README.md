# ClawEnv 文档索引（开发者）

面向开发者/维护者的设计文档 (SSOT)。

> 用户面向的说明文档在 [`README-zh.md`](README-zh.md)（中文）和仓库根的 [`README.md`](../README.md)（英文）。

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

## 本地文档（未入 git）

编号 ≥ 07 的设计稿、规格、测试手册、内部 playbook 等存放在本地 `docs/` 目录但**不追踪到 git**（通过 `.gitignore` 的 `docs/0[7-9]-*.md` / `docs/1[0-9]-*.md` / `docs/2[0-9]-*.md` 规则排除）。这些是演进中的工作笔记，按需读写，不对外发布。

想查阅某份本地文档：直接 `ls docs/` 看一眼文件名。

## 约定

- **编号**：01-06 公开，07+ 本地。不强制连续，合并/废弃不回收编号。
- **语言**：老文档英文为主，新文档（14 起）中文为主。
- **SSOT 纪律**：代码注释可引用 `docs/NN-xxx.md`；冲突时以代码为准，然后立即更新文档。
- **加新公开文档**：取下一个公开编号，把条目加到本表，**同时检查 `.gitignore` 不要误排除**。
- **加新本地文档**：取 07+ 编号，`.gitignore` 自动覆盖，直接 `git status` 看不见即可。

## 文档之外的 SSOT

- [`CLAUDE.md`](../CLAUDE.md) — AI 协作规则 + 技术栈速查。修改后 Claude 的所有 session 立即生效。
- [`scripts/TEST-PLAN.md`](../scripts/TEST-PLAN.md) — 手工测试清单，发版前按这个跑。
- [`tests/e2e/`](../tests/e2e/) — 端到端自动化测试脚本（CLI 驱动）。
