# APP 包设计

> 状态：草案 v2 | 创建：2026-04-18 | 关联：`core/src/claw/descriptor.rs`、`core/src/manager/install_native/`

## 定位：ClawEnv 是薄分发器

这份文档的前提是 ClawEnv **只做安装器**，不做任务编排、不做记忆、不做用户配置表单、不做调度。这些都是 Claw（openclaw / attaos / hermes / ...）的事。

**APP 的角色**：告诉某个 Claw "干这件事该怎么干"。APP 包内部的 skills / flows / prompts / 配置模板 —— 全是 Claw 的语言，ClawEnv 不解析、不 care。

所以 ClawEnv 对 APP 只做三件事：

```
download → verify(sha256) → 调用 Claw 的 install hook
```

之后 Claw 自己加载、展示、配置、执行。

## 职责切分

| 层 | 谁懂 |
|---|---|
| **manifest**（安装元信息 + 橱窗元信息） | ClawEnv |
| **payload/**（skills/flows/prompts/...任意结构） | 目标 Claw |

payload 对 ClawEnv 是黑盒 — 它只把这个目录路径透传给 Claw 的 `app_install_cmd`，之后发生什么 ClawEnv 不插手。

## 包结构

```
myapp-0.1.0.tar.zst
├── clawenv-app.toml          # manifest（ClawEnv 懂）
├── icon.png                   # 橱窗图标（可选，规定尺寸）
├── screenshots/               # 橱窗截图（可选）
├── README.md                  # 详情页正文（Markdown，可选）
└── payload/                   # Claw 的地盘，ClawEnv 不看
    └── (任意结构，由目标 Claw 定义)
```

## Manifest schema

manifest 分两部分：**安装元信息**（ClawEnv 用来装）和 **橱窗元信息**（前端 Marketplace 用来展示）。

```toml
schema_version = 1

# ========== 安装元信息（ClawEnv 用）==========
id             = "com.atta.lead-scraper"       # 反域名命名，全局唯一
version        = "0.1.0"                        # semver
compat_claw    = ["openclaw>=0.5", "attaos>=0.3"]  # 兼容哪些 Claw（版本区间）

[dist]
sha256    = "abc123..."                        # tar.zst 内容 sha256，ClawEnv 校验
signature = "..."                               # V2 才启用（minisign/sigstore）

# ========== 橱窗元信息（类 App Store 上架信息）==========
[showcase]
name           = "Lead Scraper"                 # 显示名
tagline        = "从 LinkedIn 抓潜客写入 CRM"    # 一句话
description    = "README.md"                    # 指向包内详情正文
icon           = "icon.png"                     # 指向包内图标
screenshots    = ["screenshots/1.png", "screenshots/2.png"]
demo_video     = "https://youtube.com/..."      # 外链，可选
category       = ["sales", "automation"]
tags           = ["linkedin", "crm", "outreach"]

[showcase.author]
name     = "Atta"
homepage = "https://atta.space"
email    = "support@atta.space"

[showcase.pricing]
model = "free"   # free / one-time / subscription —— V3 才真正接支付

[showcase.locale."zh-CN"]                       # 多语言覆盖（可选）
tagline     = "从 LinkedIn 抓潜在客户并写入 CRM"
```

### 关键原则

- **安装元信息是契约**：字段变更需要 `schema_version++`，reader 见到不认的版本就拒
- **橱窗元信息是展示**：schema 可以宽松演进，ClawEnv 识别不了的字段就忽略
- **payload 格式不进 manifest**：不要让 ClawEnv 知道"这个包里有 3 个 skill、2 个 flow"，那是 Claw 的事

## 注册中心 Index

Registry 是一个 JSON endpoint，返回所有可装 APP 的 **橱窗元信息摘要 + 下载地址**。ClawEnv 侧缓存到 `~/.clawenv/apps-index.json`，按 ETag 增量更新。

```json
{
  "schema_version": 1,
  "updated_at": "2026-04-18T00:00:00Z",
  "apps": [
    {
      "id": "com.atta.lead-scraper",
      "version": "0.1.0",
      "compat_claw": ["openclaw>=0.5"],
      "download_url": "https://cdn.atta.space/apps/com.atta.lead-scraper-0.1.0.tar.zst",
      "sha256": "abc123...",
      "showcase": {
        "name": "Lead Scraper",
        "tagline": "从 LinkedIn 抓潜客写入 CRM",
        "icon_url": "https://cdn.atta.space/apps/com.atta.lead-scraper/icon.png",
        "screenshot_urls": [...],
        "category": ["sales"],
        "author": { "name": "Atta" }
      }
    }
  ]
}
```

前端浏览市场时只拉 index（轻量），用户点"查看详情"再按需拉 README 全文，点"安装"才下载 tar.zst。

**契约对齐**：Registry 由 AttaSpace/Cloud 子项目承载，manifest/index schema 开工前必须双向对齐。

## ClawDescriptor 扩展

在现有 descriptor 上增加三个钩子（对标 `config_apikey_cmd`）：

```toml
[claw.openclaw]
app_install_cmd   = "openclaw plugin install --from /workspace/apps/{app_id}/payload"
app_uninstall_cmd = "openclaw plugin uninstall {app_id}"
app_list_cmd      = "openclaw plugin list --json"   # 可选：让 ClawEnv 能显示"这个实例装了哪些 APP"
```

- 若某 Claw 不支持 APP 机制，三字段为空 → 前端"安装到该实例"按钮置灰
- 三字段都是"shell 命令模板"，ClawEnv 复用现有 `shell_quote()` 机制（铁律 9），不引入新的执行框架

## 安装流程

```
1. 用户在 Marketplace 页点"安装" → 选择目标实例
2. CLI: 校验 compat_claw 与目标实例的 claw_type/version
3. CLI: 下载 tar.zst（带代理与进度）
4. CLI: 校验 sha256（不符即拒）
5. CLI: 解压到 /workspace/apps/{app_id}/
6. CLI: 执行 ClawDescriptor.app_install_cmd（把 payload 路径传给 Claw）
7. CLI: 记录到 config.toml 的 installed_apps[]
8. Tauri emit('app:installed', {...})
```

卸载反向执行。更新 = 卸载 + 安装（先不搞增量）。

## ClawEnv 侧数据模型

持久化到 `~/.clawenv/config.toml`：

```toml
[[installed_apps]]
id           = "com.atta.lead-scraper"
version      = "0.1.0"
instance     = "my-openclaw"
installed_at = "2026-04-18T10:00:00Z"
install_path = "/workspace/apps/com.atta.lead-scraper"
```

ClawEnv 只记这些。APP 的运行状态、配置、日志都是 Claw 管。

## 分期路线

### MVP（2-3 周）

- 支持下载、sha256 校验、解压、调用 install hook
- `clawcli app install/uninstall/list --json`
- 静态 `assets/app-registry.toml`（bundled），不搞 remote
- 前端 Apps 页：已装列表 + 卸载
- **不做签名、不做搜索、不做多语言**
- 目标：跑通 "点击→下载→装进 Claw→能用" 骨架

### V1（3-4 周）

- Remote registry（从 Cloud 拉 index.json + ETag 缓存）
- 完整包格式：tar.zst + manifest + 橱窗资源
- 前端 Marketplace 页：分类、搜索、详情、截图轮播、README 渲染
- `compat_claw` semver 解析

### V2（2-3 周）

- 签名验证：minisign 起步
- 开发者工具链：`clawcli app pack`、`clawcli app publish`
- 版本更新提示（registry 上版本高于本地）

### V3（长期）

- 付费 APP（接 Stripe，分成）
- 评分 / 评论
- 私有 registry（企业内部市场）

## 架构调整清单

| 位置 | 动作 |
|---|---|
| `core/src/app/` | **新模块**：`manifest.rs` / `download.rs` / `installer.rs` / `registry.rs` / `showcase.rs` |
| `core/src/claw/descriptor.rs` | 增 `app_install_cmd` / `app_uninstall_cmd` / `app_list_cmd` 三字段 |
| `core/src/config/models.rs` | 增 `InstalledApp { id, version, instance, installed_at, install_path }` |
| `core/src/manager/install_native/` | 不动，`app/download.rs` 复用其 sha256 框架与代理 |
| `cli/src/` | 新增 `app` 子命令组，JSON 输出 |
| `assets/` | MVP：`app-registry.toml`；V1 起迁移到远程 |
| `src/pages/Apps/` | **新增**：已装 APP 管理（跨实例） |
| `src/pages/Marketplace/` | **V1 新增**：浏览/搜索/详情/安装 |
| `src/App.tsx` | `LaunchState` 增加对应路由（铁律 5） |

## 不做的事（刻意裁掉 — 都是 Claw 的责任）

- ❌ 解析 payload 内容 — ClawEnv 只是快递员
- ❌ 渲染 APP 的配置表单 — Claw 自己的管理 WebView 做
- ❌ 给 APP 提供运行时 API / 权限模型 — Claw 的沙盒边界已经是终态
- ❌ cron 调度 / trigger — Claw 或 Claw 的 flow 引擎做
- ❌ 日志聚合 / 运行历史面板 — Claw 管理页做，ClawEnv 入口打开 WebView 即可（铁律 7）
- ❌ APP 间依赖解析 — MVP/V1 不搞；若要搞就靠 Claw 的插件系统，ClawEnv 不介入

## 风险

1. **schema 对齐**：开工前必须与 AttaSpace/Cloud 的 registry schema 双向对齐，否则两侧各自演进后期返工
2. **恶意 APP**：V2 签名前只能靠白名单 registry；早期不接受第三方上传
3. **Claw 插件机制不统一**：openclaw / attaos / hermes 的 `app_install_cmd` 契约各不相同 —— ClawEnv 只要给每个 Claw 填对 descriptor 就行，不做统一抽象
4. **CDN 成本**：包 + 截图 + 图标走流量，若不用 Cloud 的对象存储而自建要算账

## 验收标准（MVP）

1. `clawcli app install com.atta.lead-scraper --instance my-openclaw --json` 下载、校验、解压、触发 install hook 全链路通
2. sha256 不匹配 → 拒绝，错误信息清晰
3. `compat_claw` 不满足 → 拒绝，告诉用户目标实例版本不够
4. `clawcli app uninstall` 清理 `/workspace/apps/{id}` 并触发 Claw 侧卸载 hook
5. 前端 Apps 页显示本地已装 APP，可卸载
6. 图标与截图能在前端正确显示（路径解析正确）
