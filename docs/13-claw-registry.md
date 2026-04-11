# Claw 生态产品注册表

> 数据来源：2026 年 Q2 Claw 生态评测 + 美国大厂产品调研 | 更新日期：2026-04-11

## 说明

- **OpenClaw 是一个产品，支持多模型切换**——排行榜上的 OpenClaw + Claude/Gemini/GPT 等条目是同一个 npm 包 `openclaw` 接不同 API Key 的结果，不是不同产品。在 ClawEnv 中它们共用一个 `ClawDescriptor(id="openclaw")`。
- 类似的，"阿里悟空"与"阿里云 CoPaw"、"腾讯 WorkBuddy"与"腾讯龙虾管家/乐享龙虾版"等属于同一厂商的产品线变体，去重后按独立产品计。
- "悟空"、"智能虾"、"盘古虾"、"文心虾"、"生活虾"等名称中的"虾"疑为 Agent 的行业俗称。

---

## A 类：npm 包安装（在线沙盒安装）

ClawEnv 可通过 `npm install -g {package}` 直接安装的独立 claw 产品。

| # | 产品 ID | 显示名 | npm 包名 | 评分区间 | 默认端口 | 验证状态 | 说明 |
|---|--------|--------|---------|---------|---------|---------|------|
| 1 | `openclaw` | OpenClaw | `openclaw` | 83.2–96.9 | 3000 | ✅ **已验证** | 7/7 生命周期通过，v2026.4.10 |
| 2 | `zeroclaw` | ZeroClaw | `zeroclaw` | 80.9 | 3000 | ❌ npm 包非目标产品 | npm 上同名包无 CLI binary |
| 3 | `autoclaw` | 智谱 AutoClaw | `@zhipu/autoclaw` | 92.2 | 8080 | ❌ npm 404 | 包不存在，待厂商发布 |
| 4 | `qclaw` | 腾讯 QClaw | `qclaw` | 91.6 | 3000 | **已内置** | 零 |
| 5 | `kimi-claw` | Kimi Claw | `kimi-claw` | 91.0 | 3000 | **已内置** | 零 |
| 6 | `easyclaw` | 猎豹 EasyClaw | `easyclaw` | 90.0 | 9000 | **已内置** | 零 |
| 7 | `duclaw` | 百度 DuClaw | `duclaw` | 87.4 | 3000 | **已内置** | 零 |
| 8 | `arkclaw` | 字节 ArkClaw | `arkclaw` | 85.7 | 8080 | **已内置** | 零 |
| 9 | `maxclaw` | MiniMax MaxClaw | `maxclaw` | 80.6 | 3000 | **已内置** | 零 |
| 10 | `chatclaw` | 智麻 ChatClaw | `chatclaw` | 76.4 | 3000 | **已内置** | 零 |
| 11 | `wukong` | 阿里悟空 | `wukong-claw` | 89.4 | 3000 | 待添加 | 低 |
| 12 | `copaw` | 阿里云 CoPaw | `copaw` | 88.3 | 3000 | 待添加 | 低 |
| 13 | `moltbook` | 字节 Moltbook | `moltbook` | 80.2 | 3000 | 待添加 | 低 |
| 14 | `yuanqi` | 猎豹 元气AIBot | `yuanqi-aibot` | 84.2 | 3000 | 待添加 | 低 |
| 15 | `kclaw` | 快手 KClaw | `kclaw` | 81.1 | 3000 | 待添加 | 低 |
| 16 | `mitu-claw` | 美图 Claw | `mitu-claw` | 79.6 | 3000 | 待添加 | 低 |
| 17 | `lobsterai` | 网易有道 LobsterAI | `lobsterai` | 75.2 | 3000 | 待添加 | 低 |
| 18 | `wenxin-claw` | 百度文心虾 | `wenxin-claw` | 74.3 | 3000 | 待添加 | 低 |
| 19 | `360claw` | 360安全Claw | `360claw` | 76.1 | 3000 | 待添加 | 低 |

### 美国大厂 AI Agent 产品（2026 年 Q2 新增）

| # | 产品 ID | 显示名 | 厂商 | 安装方式 | 评分 | 验证状态 | 说明 |
|---|--------|--------|------|---------|------|---------|------|
| 20 | `nemoclaw` | NVIDIA NemoClaw | NVIDIA | npm (`nemoclaw`) | — | **可验证** | GTC 2026 开源，基于 OpenClaw + OpenShell 沙盒运行时 |
| 21 | `openai-cua` | OpenAI Operator/CUA | OpenAI | npm (`@openai/agents`) | — | 待验证 | Computer Using Agent，API 方式调用 |
| 22 | `claude-code` | Claude Code | Anthropic | npm (`@anthropic-ai/claude-code`) | — | 待验证 | Agentic coding tool，终端/IDE/桌面多端 |
| 23 | `ms-agent` | Microsoft Agent Framework | Microsoft | npm (`@microsoft/agentmesh-sdk`) | — | 待验证 | .NET/Python/JS 多语言，Azure 深度集成 |
| 24 | `bedrock-agent` | AWS Bedrock AgentCore | Amazon | API only | — | 不适用 | 云服务形态，无本地 CLI，走映像导入 |
| 25 | `mariner` | Google Project Mariner | Google | 未公开 | — | 不适用 | 研究预览，未开源，团队已并入 Gemini Agent |

> **适配说明**：A 类产品在当前架构下只需在 `assets/claw-registry.toml` 中新增一个 `[[claw]]` 条目即可支持，**不需要改任何代码**。唯一的前提是确认该产品的 npm 包名、CLI 命令语法、默认端口。
>
> **验证状态**：
> - ✅ **已验证** = 经过 `test-claw-lifecycle.sh` 完整测试通过
> - **可验证** = npm 包存在，CLI 接口已知，可以测试
> - **待验证** = npm 包存在但 CLI 接口未确认
> - **不适用** = 云服务/未开源，走映像导入

---

## B 类：映像导入（预构建沙盒镜像）

企业级/SaaS 产品，无公开 npm 包。用户需自行构建或从厂商获取沙盒映像，通过 ClawEnv 的 `PrebuiltImage` 模式导入。

| # | 产品名 | 厂商 | 评分 | 说明 | 适配方式 |
|---|--------|------|------|------|---------|
| 1 | WorkBuddy | 腾讯 | 87.8 | 企业定价，私有部署 | 映像内自带 descriptor manifest |
| 2 | AstronClaw | 科大讯飞 | 84.4 | 企业定价，星火认知 | 同上 |
| 3 | 乐享·龙虾版 | 腾讯 | 83.6 | SaaS 30元/人/月 | 同上 |
| 4 | 龙虾管家 | 腾讯 | 83.5 | SaaS 99元/月 | 与乐享同产品线，去重为 1 |
| 5 | 红手指 Operator | 百度 | 82.0 | 按设备付费 | 映像导入 |
| 6 | TENCLAW | 十方融海 | 81.0 | 按实例计费 | 映像导入 |
| 7 | JVSClaw | 阿里云 | 80.2 | 企业定价 | 映像导入 |
| 8 | 智能虾 | 阿里云 | 77.6 | 按量付费，通义千问3 | 与 CoPaw 同厂，去重为 1 |
| 9 | ToClaw | ToDesk | 77.2 | 专业版功能 | 映像导入 |
| 10 | BocLaw | 博云 | 76.0 | 企业定价 | 映像导入 |
| 11 | QoderWork | 阿里云 | 74.0 | SaaS 20元/月 | 与 JVSClaw 同厂，去重为 1 |
| 12 | WindClaw | 万得 | 73.5 | 金融专用，机构定价 | 映像导入 |
| 13 | SenseClaw | 商汤 | 72.9 | 企业定价 | 映像导入 |
| 14 | 灵犀Claw | 京东 | 72.4 | 企业定价 | 映像导入 |
| 15 | WorkBuddy (免费版) | 独立 | 70.3 | 多模型切换 | 映像导入 |
| 16 | 盘古虾 | 华为 | 75.1 | 企业定制 | 映像导入 |

> **适配说明**：B 类产品的映像 `manifest.toml` 中需要自带 `ClawDescriptor`，告诉 ClawEnv 如何启动和管理。当前架构已支持 `PrebuiltImage` 导入，需要补充的是：映像 manifest 中嵌入 descriptor 的解析逻辑。

**去重后独立产品数：12 款**（腾讯龙虾系 ×3→1，阿里云系 ×3→1）

---

## C 类：不支持

硬件绑定或纯 App 内产品，无法在通用沙盒中运行。

| # | 产品名 | 原因 |
|---|--------|------|
| 1 | 小米 MiClaw | 绑定小米硬件生态 |
| 2 | 涂鸦 Tuya x | 绑定涂鸦 IoT 设备 |
| 3 | 当贝 Molili | 绑定当贝硬件 |
| 4 | 华为小艺Claw | 绑定鸿蒙/盘古设备 |
| 5 | 矽速 PicoClaw | 绑定边缘计算硬件 |
| 6 | 美团生活虾 | 纯 App 内功能，无独立服务 |

---

## 去重汇总

| 原始条目数 | 去重后 | A 类 (npm) | B 类 (映像) | C 类 (不支持) |
|-----------|--------|-----------|------------|-------------|
| 47 + 6 美国大厂 | 43 | **22** (含美国大厂 3) | **15** (含美国大厂 2) | **6** |

去重规则：
- OpenClaw + 7 种模型 → 1 个产品（同 npm 包，不同 API Key）
- 腾讯 WorkBuddy / 乐享龙虾 / 龙虾管家 → 1 个产品线
- 阿里云 CoPaw / 智能虾 / QoderWork / JVSClaw → 2 个产品线（CoPaw 有独立 npm，其余走映像）
- Google Mariner 并入 Gemini Agent，计为不支持（未开源）

## 验证状态总结（2026-04-11）

### npm 包存在性批量检测

| npm 包名 | 存在 | 版本 | CLI binary | 真实身份 |
|----------|------|------|-----------|---------|
| `openclaw` | ✅ | v2026.4.10 | ✅ `openclaw` | **OpenClaw 本体** |
| `easyclaw` | ✅ | v0.7.2 | ✅ `easyclaw` | OpenClaw CLI 看门狗（需先装 openclaw） |
| `duclaw` | ✅ | v0.7.2 | ✅ `duclaw` | OpenClaw CLI 别名（DuckDuckGo 用户版） |
| `maxclaw` | ✅ | v0.7.2 | ✅ `maxclaw` | OpenClaw CLI 扩展健康检查版 |
| `zeroclaw` | ✅ | v0.1.4 | ❌ | TypeScript SDK wrapper，非 CLI 产品 |
| `nemoclaw` | ✅ | v0.1.0 | ❌ | npm 占位包（非 NVIDIA 官方） |
| `@openai/agents` | ✅ | v0.8.3 | ❌ | OpenAI Agents SDK（库，非独立 CLI） |
| `@anthropic-ai/claude-code` | ✅ | v2.1.101 | ✅ | Anthropic Claude Code（agentic coding） |
| `@microsoft/agentmesh-sdk` | ✅ | v3.0.2 | ❌ | Microsoft Agent Framework SDK |
| `qclaw` | ❌ | — | — | 未发布 |
| `kimi-claw` | ❌ | — | — | 未发布 |
| `arkclaw` | ❌ | — | — | 未发布 |
| `chatclaw` | ❌ | — | — | 未发布 |
| `@zhipu/autoclaw` | ❌ | — | — | 未发布 |

### 完整生命周期测试

| 产品 | npm install | 版本验证 | Gateway 启动 | 完整 7/7 |
|------|-----------|---------|-------------|---------|
| **OpenClaw** | ✅ 880s, 663 pkgs | ✅ v2026.4.10 | ✅ HTTP 200 (4s) | ✅ |
| easyclaw | ✅ 21s, 3 pkgs | ✅ v0.7.2 | ❌ 需先装 openclaw | 5/7 |
| ZeroClaw | ✅ (同名包) | ❌ 无 CLI | — | — |
| AutoClaw | ❌ npm 404 | — | — | — |

### 结论

截至 2026-04-11，**npm 上唯一可独立完整运行的 Claw 产品是 OpenClaw**。其余国内 claw 产品尚未公开发布 npm 包。easyclaw/duclaw/maxclaw 是 OpenClaw 生态的辅助工具。NVIDIA 官方 NemoClaw 通过自有安装器分发（`curl | bash`），npm 上的 `nemoclaw` 包为第三方占位。
