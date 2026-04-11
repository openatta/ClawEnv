# 国内 Claw 产品 OpenClaw 包装关系分析

> 调研日期：2026-04-11 | 状态：定期复查

## 结论

**截至 2026 年 4 月，所有国内主流 claw 产品的底层运行时均为 OpenClaw。** 没有任何一家国内厂商从零开发了独立的 AI Agent 执行引擎。差异仅在于：安装方式（桌面包/云 SaaS）、预置模型、UI 壳、IM 集成通道。

---

## 逐产品证据

### 1. AutoClaw（澳龙）— 智谱

| 项目 | 内容 |
|------|------|
| 官网 | https://autoglm.zhipuai.cn/autoclaw/ |
| 安装方式 | 桌面安装包 (.dmg/.exe)，一键安装 |
| **OpenClaw 关系** | 官方明确称 **"国内首个一键安装的本地版 OpenClaw"** |
| 证据来源 | [IT之家报道](https://www.ithome.com/0/927/423.htm)："内核与 OpenClaw 完全一致，能力无损" |
| 技术细节 | 安装包内嵌 Node.js + OpenClaw npm 包 + GLM-5 模型配置 + AutoGLM 浏览器自动化插件 |
| 独有功能 | AutoGLM Browser-Use（智谱自研浏览器自动化）、飞书集成、预置 50+ Skills |
| **ClawEnv 定位** | 无需支持 — 它自己就是一个 OpenClaw 安装器，和 ClawEnv 功能重叠 |

### 2. QClaw（鹅虾）— 腾讯

| 项目 | 内容 |
|------|------|
| 官网 | https://qclaw.qq.com/ |
| 安装方式 | 桌面安装包 (.dmg/.exe)，20 秒安装 |
| **OpenClaw 关系** | 官方称 **"对 OpenClaw 技术体系的深度封装"**，采用"三明治结构"：底层 OpenClaw + 中层腾讯模型路由 + 顶层微信/QQ 通道 |
| 证据来源 | [知乎保姆级教程](https://zhuanlan.zhihu.com/p/2018081477640889991)："首次启动 QClaw 会自动检测电脑环境并一键部署 OpenClaw" |
| 技术细节 | 首次启动自动下载 OpenClaw，内置 Kimi-2.5、GLM-5 模型 |
| 独有功能 | 微信远程控制（扫码绑定）、QQ/企微/飞书/钉钉多平台接入 |
| **ClawEnv 定位** | 无需支持 — 它自带 OpenClaw 安装和管理 |

### 3. EasyClaw — 猎豹移动

| 项目 | 内容 |
|------|------|
| 官网 | 猎豹移动官网 |
| 安装方式 | 桌面安装包，双击安装 |
| **OpenClaw 关系** | npm 上的 `easyclaw` 包 (v0.7.2) 描述为 **"Easy-to-use AI gateway watchdog — the simplest way to manage openclaw-cli"** |
| 证据来源 | `npm view easyclaw` 返回 `repository: git+https://github.com/Sobranier/openclaw-cli.git`，直接指向 openclaw-cli 仓库 |
| 技术细节 | 本质是 openclaw-cli 的 wrapper，需要先安装 openclaw 才能运行 gateway |
| ClawEnv 测试验证 | 安装通过 (21s, 3 pkgs)，但 gateway 报错 `"openclaw CLI not found. Please install openclaw first."` |
| **ClawEnv 定位** | 无需支持 — 它不是独立产品，是 OpenClaw 的辅助工具 |

### 4. ArkClaw — 字节跳动/火山引擎

| 项目 | 内容 |
|------|------|
| 官网 | https://console.volcengine.com/ark/claw |
| 安装方式 | **纯云端 SaaS**，火山方舟控制台创建，无本地安装 |
| **OpenClaw 关系** | 官方称 **"开箱即用的云上 SaaS 版 OpenClaw"** |
| 证据来源 | [博客园](https://www.cnblogs.com/javastack/p/19706462)："别再折腾 OpenClaw 部署了，字节推出 ArkClaw，一键部署 OpenClaw" |
| 技术细节 | 云端运行 OpenClaw 实例，支持 Doubao-Seed 2.0、DeepSeek 3.2、GLM 5.0 等模型 |
| 定价 | Lite 9.9元/月，Pro 49.9元/月 |
| **ClawEnv 定位** | 无需支持 — 云端运行，无本地沙盒需求 |

### 5. DuClaw — 百度智能云

| 项目 | 内容 |
|------|------|
| 官网 | https://cloud.baidu.com/product/duclaw.html |
| 安装方式 | **纯云端 SaaS**，订阅即用 |
| **OpenClaw 关系** | 官方称 **"零部署 OpenClaw 服务"**，"用户无需接触任何云控制台操作，不需了解镜像、服务器、API Key 等专业概念" |
| 证据来源 | [IT之家](https://www.ithome.com/0/927/928.htm)："百度智能云发布 DuClaw：不用自行配置 API Key，网页端直接开用 OpenClaw" |
| 技术细节 | 预置百度搜索/百科 Skills，支持 DeepSeek、Kimi-K2.5、GLM-5、MiniMax-M2.5 |
| 定价 | 17.8元/月起 |
| **ClawEnv 定位** | 无需支持 — 云端运行 |

### 6. Kimi Claw — 月之暗面

| 项目 | 内容 |
|------|------|
| 官网 | https://www.kimi.com/bot |
| 安装方式 | **纯云端 SaaS**，kimi.com 一键创建 |
| **OpenClaw 关系** | 官方称 **"云端化 OpenClaw 产品"**，"原生 OpenClaw 上线 kimi.com" |
| 证据来源 | [DataLearner](https://www.datalearner.com/en/blog/1051771167197865)："一个在云端拥有 40G 空间的 24×7 运行的 OpenClaw，基于 Kimi 模型驱动" |
| 技术细节 | 内置 Kimi K2.5 模型、5000+ ClawHub 社区插件、40GB 云存储 |
| **ClawEnv 定位** | 无需支持 — 云端运行 |

### 7. npm 上的同名包（duclaw / maxclaw / easyclaw）

| npm 包 | 版本 | 真实身份 | 证据 |
|--------|------|---------|------|
| `easyclaw` v0.7.2 | OpenClaw CLI wrapper | `npm view easyclaw repository` → `github.com/Sobranier/openclaw-cli` |
| `duclaw` v0.7.2 | OpenClaw CLI 别名 | `npm view duclaw description` → "OpenClaw CLI for DuckDuckGo AI users. Alias of openclaw-cli." |
| `maxclaw` v0.7.2 | OpenClaw CLI 扩展 | `npm view maxclaw description` → "Maximum reliability AI gateway watchdog — openclaw-cli with extended health checks" |

三个包同版本(0.7.2)、同源码仓库(openclaw-cli)，**是社区开发者对 openclaw-cli 的重新包装，与国内大厂的同名产品无关**。

---

## 总结图

```
                    ┌──────────────────────┐
                    │     OpenClaw         │
                    │  (开源 AI Agent 框架)  │
                    │   npm install -g     │
                    └──────────┬───────────┘
                               │
        ┌──────────┬───────────┼───────────┬──────────┐
        │          │           │           │          │
   桌面安装包    云端 SaaS    云服务器镜像   ClawEnv    社区分支
   ┌────┴────┐ ┌───┴───┐  ┌───┴───┐   (本项目)    ┌───┴───┐
   │AutoClaw│ │ArkClaw│  │阿里云  │              │NanoClaw│
   │QClaw   │ │DuClaw │  │腾讯云  │              │PicoClaw│
   │EasyClaw│ │Kimi   │  │华为云  │              │IronClaw│
   └────────┘ └───────┘  └───────┘              └───────┘
    (智谱/腾讯/  (字节/百度/  (预装镜像)           (独立实现,
     猎豹)      月之暗面)                         非OpenClaw)
```

## 定期复查清单

- [ ] AutoClaw 是否发布了独立 npm 包（非 OpenClaw 依赖）
- [ ] QClaw 是否开源或发布 npm 包
- [ ] Kimi Claw 是否提供本地安装选项
- [ ] NanoClaw npm 包是否添加了 CLI binary
- [ ] NVIDIA NemoClaw 官方 npm 包是否发布（当前 npm 上的是第三方占位）
- [ ] 新的独立 claw 产品是否出现（非 OpenClaw 包装）
