# API Key 管理设计

> 状态：草案 | 创建：2026-04-18 | 关联：`core/src/config/keychain.rs`、`src/pages/Install/StepApiKey.tsx`、`core/src/claw/descriptor.rs`

## 背景与问题

当前 API Key 仅在 **安装向导** 中输入（`StepApiKey.tsx`），一旦跳过或填错，安装后没有任何入口可以修改、测试、替换 key。对想先装壳、后配 key 的用户完全不友好。

进一步的诉求：
- 一个 key 应能绑到多个实例（节约额度）
- 希望看到每个实例/每个 key 的用量
- 未来可能直接在 ClawEnv 内买 credits

## 现状可复用资产（别重造轮子）

| 能力 | 位置 | 备注 |
|---|---|---|
| 密钥存储 | `core/src/config/keychain.rs` | `keyring` crate，服务名 `clawenv`，命名约定 `apikey-{instance}` |
| 注入到实例 | `ClawDescriptor.config_apikey_cmd` | 模板如 `openclaw config set apiKey '{key}'` |
| 输入 UI | `src/pages/Install/StepApiKey.tsx` | 支持输入、测试、保存，整组件可抽离复用 |
| CLI 侧 | `clawcli --api_key=...` | 安装时参数已通 |

**结论：基础设施齐备，本质上是补管理入口 + 改命名约定。**

## 分期设计

### P0 — Settings 页 API Keys Tab（2 周，强烈推荐）

**目标**：安装后随时可以增/删/测试/切换 API Key。

- 前端：新增 `src/pages/Settings/ApiKeysTab.tsx`；复用 StepApiKey 的测试/保存逻辑抽成 `components/ApiKeyEditor.tsx`
- CLI 扩展（依旧只 JSON 输出，遵守铁律 8）：
  ```
  clawcli apikey list --json
  clawcli apikey set --instance <name> --provider <openai|anthropic|...>
  clawcli apikey unset --instance <name>
  clawcli apikey test --instance <name>
  ```
- Tauri 侧：在 `cli_bridge` 增加对应 spawn 封装，遵守铁律 2（异步 emit 进度）
- 键的 provider 维度：命名从 `apikey-{instance}` 升级为 `apikey-{instance}-{provider}`，支持一个实例挂多家 LLM 的 key

**不改的事**：P0 阶段每个实例仍然独占一套 key（不共享），迁移成本最小。

### P1 — Key Pool 模型（1 周）

**动机**：同一把 OpenAI key，想给 3 个 openclaw 实例共用。

改动：
- `keyring` 命名：`apikey-pool-{uuid}`，元数据（nickname/provider/created_at）存 `config.toml`
- `InstanceConfig` 新增字段 `api_key_refs: Vec<KeyRef>`（解绑 1:1）
- 迁移：启动时扫旧 `apikey-{instance}` 并自动升级成 pool 条目 + ref，旧条目保留一个版本周期后删
- UI：Keys tab 里是 key 列表，每条可展开"绑定到哪些实例"

### P2 — 用量统计（2 周，取巧方案）

**不自己埋点**（LLM 厂商 billing API 粒度差、延迟高、各家不统一）。

推荐路径：
- 在 key 上加一个 `gateway: Option<GatewayKind>` 字段，支持 `litellm` / `openrouter` / `direct`
- Gateway 模式：ClawEnv 在沙盒内起一个 LiteLLM/OpenRouter 边车容器，实例的 `baseURL` 指向 `http://gateway:4000`，key 统一由 gateway 管
- 统计：直接打开 gateway 自带 dashboard（LiteLLM 有现成的 Web UI），ClawEnv 只负责"一键打开仪表板"按钮
- 代价：多一个边车进程；好处：统计/限额/fallback 全免费拿

### P3 — 采购（暂缓）

- 若 Atta Cloud 自己卖 credits：接 Stripe，订单存 Cloud 后端，ClawEnv 只是前端壳
- 若不卖：提供到 OpenAI/Anthropic billing 页面的深链接即可
- **除非 Cloud 侧决定卖 credits，否则不做**

## 架构调整总览

| 模块 | 变化 |
|---|---|
| `core/src/config/keychain.rs` | P1 改命名约定 + 新增 `list_keys / rename_key / bind / unbind` API |
| `core/src/config/models.rs` | P1 `InstanceConfig.api_key_refs`；新增 `KeyPoolEntry { id, nickname, provider, gateway }` |
| `core/src/claw/descriptor.rs` | 无改动（注入模板机制仍然用 `config_apikey_cmd`） |
| `cli/src/` | 新增 `apikey` 子命令组，JSON 输出 |
| `src/pages/Settings/` | 新增 Settings 页和 ApiKeys tab（若 Settings 页还没有则一并建） |
| `assets/claw-registry.toml` | 无改动 |

## 不做的事（刻意裁掉）

- ❌ 自建 key 代理（安全责任太重）
- ❌ 自己扒 OpenAI billing（频繁变，且需要 login cookie）
- ❌ 跨设备同步 key（Keychain 已经做本地保护，跨机器同步让用户自己用 iCloud/1Password）
- ❌ Key 轮换/自动过期（企业功能，社区版不需要）

## 验收标准（P0）

1. 安装时未填 key → 安装后 Settings 里能补
2. `clawcli apikey list --json` 输出可被 Tauri 侧稳定解析
3. 切换 key 后实例内部 `config_apikey_cmd` 正确执行，openclaw 能用新 key 响应
4. 误输入错 key → 测试按钮能识别并给出明确错误
5. Keychain 中不残留旧 key；卸载实例时 key 同步清理
