# ClawEnv 监控边界

> 状态：草案 | 创建：2026-04-18 | 关联：`core/src/update/checker.rs`、`core/src/sandbox/*.rs`、`core/src/manager/*.rs`、`docs/22-attarun-bridge.md`

## 总纲：ClawEnv 三大职责

ClawEnv 只做三件事：**分发 / 安装 / 监控**。这份文档定义第三件事的范围，同时梳理前两件在运行期留下的"对账点"。

> **监控的铁律**：ClawEnv 监控 **基础设施与安装状态**，不监控业务语义。业务运行时的 Agent 轨迹、LLM 调用、任务产出由 Claw 自己的管理面板负责（铁律 7，WebView 嵌入）。

## 监控范围

```
┌─────────────────────────────────────────────────────────┐
│  ClawEnv 监控                                            │
│                                                           │
│  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐         │
│  │ 沙盒   │  │ Claw   │  │ 已装   │  │ Bridge │         │
│  │ 后端   │  │ 实例   │  │ APP    │  │ daemon │         │
│  └────────┘  └────────┘  └────────┘  └────────┘         │
│                                                           │
│  ┌────────┐  ┌────────┐  ┌────────┐                     │
│  │ 更新   │  │ API    │  │ 代理   │                     │
│  │ 对账   │  │ Key    │  │ 网络   │                     │
│  └────────┘  └────────┘  └────────┘                     │
└─────────────────────────────────────────────────────────┘

          ↕ 不监控，交给 Claw 管理面板
┌─────────────────────────────────────────────────────────┐
│  Agent 运行轨迹 / LLM 调用 / 业务数据 / APP 内部状态     │
└─────────────────────────────────────────────────────────┘
```

## 7 个监控域

### 1. 沙盒后端健康

对应当前 backend（WSL2 / Lima / Podman / Native），只监控一个（铁律 1）。

| 指标 | 采集方式 | 告警阈值 |
|---|---|---|
| VM / 容器运行时存活 | `limactl list` / `wsl -l -v` / `podman ps` / N/A | 进程不存在即红 |
| 后端守护进程 | 后端自身心跳命令 | 3 次连续失败告警 |
| 磁盘空间（沙盒卷） | backend 自带查询 | <10% 黄，<5% 红 |
| 网络连通性 | 从沙盒 ping 外网 / npm registry | 不通即告警 |

**代码位置**：`core/src/sandbox/*.rs`（已有存活检测），扩展统一健康查询接口 `trait HealthProbe`。

### 2. Claw 实例状态

| 指标 | 采集方式 |
|---|---|
| gateway_port 可达 | HTTP `GET /health` 或 TCP connect |
| 进程存在 | 沙盒内 `ps` / `pgrep`（通过 `ClawDescriptor.version_cmd` 间接探测） |
| 版本 | `ClawDescriptor.version_cmd` 返回值 |
| 最后启动时间 | 从 ClawEnv 侧 start 动作记录，不从 Claw 内部取 |

### 3. 已装 APP 对账（见 docs/20）

ClawEnv 侧 `config.toml` 的 `installed_apps[]` vs Claw 的 `app_list_cmd` 实际返回结果对比：
- 一致 → 绿
- ClawEnv 有 Claw 没 → 黄（"安装不完整，请重装"）
- Claw 有 ClawEnv 没 → 黄（"未知 APP，点击认领或清理"）

对账频率：进入 Apps 页时触发一次，不做常态轮询（节约资源）。

### 4. Bridge daemon（见 docs/22）

| 指标 | 来源 |
|---|---|
| daemon 进程存活 | 系统 daemon 协议（launchctl / systemctl / schtasks） |
| admin API 可达 | `GET /api/health` |
| 信令连接就绪 | `/api/status.signaling.connected` |
| 当前配对设备数 | `/api/status.paired_peers` |
| 当前活跃 WebRTC 连接数 | `/api/status.active_peers` |
| channel 同步状态 | channels.toml 内容 vs `/api/channels` 返回 |

前三项进入 tray / 顶部状态带，后三项在 Bridge 管理页展示。

### 5. 更新对账（docs/05、checker.rs 已部分）

对三类组件做"已装版本 vs 可用最新版本"对账：

| 组件 | 数据源 |
|---|---|
| ClawEnv 自身 | GitHub Releases（已实现） |
| Claw 实例 | npm registry / pypi / git tag（已实现 `check_latest_npm` 等） |
| 已装 APP | registry index.json 里每条的 version（MVP 之后） |
| Bridge | 跟随 ClawEnv，不单独对账 |

对账结果进入 Settings 页和 tray 菜单。

### 6. API Key 健康（见 docs/19）

- 存在性：Keychain 里有无对应条目
- 可用性：用户点击"测试"时发一次轻调用验证
- 不做常态探测（LLM 厂商不喜欢）

### 7. 代理 / 网络

- `core/src/config/proxy.rs` 的配置是否生效（启动时做一次 `GET api.openai.com` 之类）
- 代理密码是否还能从 Keychain 读出（迁移 / 权限丢失检测）

## 用户可见的监控入口

### System Tray（铁律 6）

常驻托盘。展示当前**最严重**的状态：

- 全绿 → 常规图标
- 有黄 → 黄色点
- 有红 → 红色点 + 浮层文案

点开菜单：沙盒状态、实例快速开关、Bridge 在线状态、更新提醒。

### 顶部状态带（主窗口 / Bridge 管理页）

一条窄 banner 显示当前监控域的最严重状态，可折叠展开看全部 7 域细节。

### 各功能页的卡片

- 实例页：每个实例卡片自带状态点（1 + 2 + 6 的融合）
- Apps 页：安装对账结果（3）
- Bridge 页：4 的详细指标
- Settings 页：5、6、7 的详情

## 关键实现原则

### 拉模式为主，推模式为辅

- 绝大多数指标 ClawEnv 主动 query（避免组件逆向依赖 ClawEnv）
- 异步事件用 Tauri `Emitter::emit`（铁律 2）仅用于 ClawEnv 自己触发的动作（安装进度、下载完成）
- 不要求 Claw 或 APP 主动上报

### 采集频率分级

| 频率 | 指标 |
|---|---|
| 用户触发 | 安装对账、API Key 测试、代理测试 |
| 进入页面时 | 版本对账、APP 对账 |
| 5s 轮询 | 沙盒健康、实例 gateway 可达、Bridge admin health |
| 30s 轮询 | Bridge peer 数、流量统计 |
| 启动时一次 | 代理可用性、更新检查（已实现） |

### 告警抑制

- 用户主动执行 stop / pause 操作后，对应域进入"静默"状态，不告警
- 连续失败 3 次才触发 UI 告警，避免抖动

### 不自建指标存储

- 所有监控数据**不落盘**（除了 bridge 自己的日志文件）
- 历史趋势、长期观测不在 ClawEnv 范围 — 需要的用户接入自己的 Prometheus / Grafana，ClawEnv 可选暴露 `/metrics`（V2 考虑）

## 架构调整

| 位置 | 动作 |
|---|---|
| `core/src/monitor/`（新） | 汇总入口：`HealthProbe` trait、各域的实现、统一状态聚合 `SystemStatus` |
| `core/src/sandbox/*.rs` | 补齐 `impl HealthProbe` |
| `core/src/manager/*.rs` | 补齐 instance / app 对账逻辑 |
| `core/src/bridge/` | admin_client 暴露监控查询接口 |
| `cli/src/` | 新增 `status --json` 返回聚合状态 |
| `src/components/StatusDot.tsx` | 通用状态点 / 状态带组件 |
| `src/lib/monitor.ts` | 前端侧轮询调度器，按频率分级调度 |

## 不做的事（刻意裁掉）

- ❌ 解析 Claw 内部日志的业务语义 — 该信息经 Claw 自己的管理 WebView 展现
- ❌ LLM 调用链路追踪 — 上游 LiteLLM / OpenRouter gateway 有（docs/19 P2）
- ❌ 自建时序数据库 / dashboard — 不做轻易膨胀的基础设施
- ❌ 告警推送（邮件 / 短信 / webhook）— 用户要就接 bridge + 手机 APP 的推送通道

## 与相关文档的交叉引用

- **docs/19 API Key 管理**：监控域 #6
- **docs/20 APP 包**：监控域 #3
- **docs/22 AttaRun Bridge**：监控域 #4

## 验收标准

1. Tray 图标能在不开主窗口时正确反映最严重状态（路径：关闭 sandbox → tray 变红）
2. 7 个监控域都能返回结构化状态（`clawcli status --json` 一次性拿全）
3. 所有域在异常恢复后能自动回归绿色（不需要重启 ClawEnv）
4. 沙盒后端切换（比如 Lima → Podman）时监控逻辑自动跟随（铁律 1 的对等性不破）
