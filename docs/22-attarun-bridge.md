# AttaRun Bridge 集成设计

> 状态：草案 | 创建：2026-04-18 | 关联：AttaRun 项目（app / cloud / bridge 三部分）、`core/src/claw/descriptor.rs`、`core/src/config/models.rs`

## 1. 角色与边界

AttaRun 由三部分组成，ClawEnv **只和 bridge 打交道**：

- **AttaRun.app**：手机端，不在 ClawEnv 讨论范围
- **AttaRun.cloud**：信令 / 会合服务器（STUN/TURN、设备注册），不在 ClawEnv 讨论范围
- **AttaRun.bridge**：本地常驻进程，通过 WebRTC（P2P + E2E）把本地 Claw 暴露给手机端

### 1.1 数据流

```
手机 APP ──WebRTC──┐           ┌──WebRTC──> AttaRun.bridge ──localhost──> Claw 1 gateway
                   ├─ Cloud ────┤                             └─────────> Claw 2 gateway
                   │ (仅信令)   │                             └─────────> Claw N gateway
                  STUN/TURN
                                              (宿主机，独立 daemon，ClawEnv 管理)
```

- **数据面**：手机 ↔ bridge 直连 P2P，E2E 加密，Cloud 不参与
- **控制面**：Cloud 只做信令会合（SDP offer/answer 中转、ICE 候选交换）
- **本地**：bridge 访问 `localhost:<gateway_port>`，不穿透沙盒

### 1.2 ClawEnv 对 bridge 的职责（清单）

| 类别 | 事项 |
|---|---|
| 分发 | bridge 二进制编进 ClawEnv release bundle（同包同版本） |
| 安装 | 首次启动 ClawEnv 时把 bridge 注册为系统 daemon |
| 配置 | 维护路由表（channel 列表），按 `InstanceConfig` 同步下发 |
| 凭证 | 首次启动生成 Ed25519 keypair，私钥存 Keychain，公钥发 Cloud 换 device_id |
| 生命周期 | 通过 bridge admin API 与系统 daemon 协议做 start/stop/restart/status |
| 升级 | 跟 ClawEnv 同节奏（同包） |
| 监控 | daemon 存活、信令连接、channel 同步状态、peer 数、流量 |
| UI | 左侧导航留一个 Bridge 按钮 → 独立管理页 |

### 1.3 ClawEnv 不做的事

- ❌ WebRTC 协议实现、ICE / STUN / TURN 逻辑 — 全在 bridge 内部
- ❌ 鉴权、配对协议、设备信任链 — bridge + Cloud
- ❌ 解析 E2E 流量（加密层之下 ClawEnv 也看不到）
- ❌ 手机端的一切

## 2. 架构铁律（提议新增）

> **Bridge 是独立 daemon，和沙盒实例平级。ClawEnv 管理但不托管其进程生命周期。**

含义：
- ClawEnv tray 关闭不影响 bridge，手机端仍可访问
- ClawEnv 通过**系统 daemon 协议**（launchd / systemd / Windows Service）和 **admin API** 两个通道管 bridge，不通过父子进程
- 升级 bridge = 停止 daemon → 替换二进制 → 重启 daemon

## 3. 守护进程注册

| 平台 | 机制 | 单元文件位置 |
|---|---|---|
| macOS | launchd LaunchAgent（用户级） | `~/Library/LaunchAgents/space.atta.clawenv.bridge.plist` |
| Linux | systemd --user | `~/.config/systemd/user/clawenv-bridge.service` |
| Windows | Task Scheduler（登录时触发，更简单）或 Windows Service | Task: `ClawEnvBridge` |

**注册时机**：首次启动 ClawEnv（不是懒加载）。理由：默认开启暴露策略，用户期望装完即可访问。

**启动参数**：bridge 从环境变量读入：
- `CLAWENV_BRIDGE_CONFIG=~/.clawenv/bridge/config.toml`
- `CLAWENV_BRIDGE_STATE=~/.clawenv/bridge/state/`
- `CLAWENV_BRIDGE_LOG=~/.clawenv/logs/bridge.log`
- `CLAWENV_BRIDGE_ADMIN_TOKEN_KEYCHAIN=clawenv/bridge-admin-token`（Keychain 服务名）

## 4. 端口模型

### 4.1 每个实例两个端口

| 端口 | 作用 | 谁用 |
|---|---|---|
| `gateway_port` | Claw 本身的 HTTP 管理/业务面 | 宿主 localhost / Tauri WebView 直接访问 |
| `bridge_port` | bridge 内部为这个实例分配的本地监听端口 | 仅 bridge 使用，用作 channel 到 gateway 的反向代理 endpoint |

### 4.2 端口分配

- `InstanceConfig` 新增 `bridge_port: u16` 字段
- 安装实例时由 ClawEnv 在 `49152-65535` 动态范围里选空闲端口
- 即使 bridge channel 对该实例关闭，端口仍预留（便于后续启用时不需要重分配）

### 4.3 为什么要 bridge_port 而不是让 bridge 直连 gateway

理论上 bridge 可以直接访问 `localhost:gateway_port`。加一层 bridge_port 的好处：
- **生命周期隔离**：Claw 实例重启 gateway 端口可能变，bridge_port 稳定
- **统一治理**：将来要加本地限流 / ACL / 流量审计时有注入点
- **调试**：`nc localhost <bridge_port>` 可独立测试 bridge → Claw 这段，和 WebRTC 解耦

如果实现复杂度高于收益，后期可以退化成直连（API 层面保留字段即可）。

## 5. Channel 模型

### 5.1 什么是 channel

一个 channel 绑定一个 Claw 实例：

```
channel := {
  id: "ch_<instance_name>",          # 路由标识，手机端通过这个选实例
  instance: "my-openclaw",
  target: "localhost:<bridge_port>", # bridge 内部转发到这里
  enabled: true,                     # 是否对手机端可见
  allow_gateway: true,               # 暴露 gateway（业务面）
  allow_dashboard: false,            # 是否也暴露 dashboard（默认否，防泄露管理面）
}
```

### 5.2 默认策略

- 新装实例 → ClawEnv 自动加 channel，`enabled=true`，`allow_gateway=true`，`allow_dashboard=false`
- 用户可在 Bridge 管理页按实例开关 enabled 或调整暴露范围

### 5.3 配置下发

ClawEnv 写 `~/.clawenv/bridge/channels.toml`，bridge 监听文件变更（inotify/FSEvents/ReadDirectoryChangesW）或收到 `POST /api/channels/reload` 后重载。

```toml
[[channel]]
id              = "ch_my-openclaw"
instance        = "my-openclaw"
target          = "127.0.0.1:50123"
enabled         = true
allow_gateway   = true
allow_dashboard = false
```

## 6. Pairing（设备配对）

### 6.1 首次启动流程

```
1. ClawEnv 首启：检查 Keychain 有没有 bridge keypair
2. 无 → 生成 Ed25519 私钥 + 公钥
3. 私钥写 Keychain (service=clawenv, key=bridge-device-privkey)
4. 向 Cloud POST /devices/register { pubkey } → 拿到 device_id
5. device_id 写 ~/.clawenv/bridge/identity.toml（非敏感，可明文）
6. bridge daemon 启动时从 Keychain + identity.toml 读身份
```

### 6.2 手机端配对

- 用户在 Bridge 管理页点"配对新设备" → ClawEnv 调 `POST /api/pair/start`
- bridge 向 Cloud 申请一次性配对码（6 位数字 + 2 分钟 TTL）
- 前端展示配对码 + QR（二维码内容：`attarun://pair?code=<code>&device=<device_id>`）
- 手机 APP 扫码 / 输入码 → 通过 Cloud 完成身份绑定
- bridge 拿到 peer pubkey → 加入信任列表
- 前端轮询 `GET /api/peers` 看到新设备出现即完成

### 6.3 撤销设备

- 管理页列表旁有"吊销"按钮 → `DELETE /api/peers/{peer_id}`
- bridge 从信任列表剔除 + 发 Cloud 吊销该 peer（让 Cloud 拒绝其未来的信令请求）

## 7. Admin API

### 7.1 绑定与鉴权

- bind `127.0.0.1:<admin_port>`（admin_port 在 `49152-65535` 随机选，落 `~/.clawenv/bridge/admin.port`）
- 鉴权：HTTP header `X-Bridge-Token: <token>`，token 首次启动生成后写 Keychain（service=clawenv, key=bridge-admin-token）
- ClawEnv 前端通过 Tauri 侧的 CLI bridge 调用，不直接从 WebView 发（避免 token 泄露）

### 7.2 端点一览（MVP）

| Method | Path | 作用 |
|---|---|---|
| GET | `/api/health` | 存活探针（不需要 token） |
| GET | `/api/status` | 整体状态：信令连接、上线设备数、流量汇总 |
| GET | `/api/channels` | 路由表 |
| POST | `/api/channels/reload` | 触发 channels.toml 热重载 |
| GET | `/api/peers` | 活跃 peer 列表（device_id、昵称、最后活跃时间、流量） |
| DELETE | `/api/peers/{id}` | 踢除 peer |
| POST | `/api/pair/start` | 生成配对码 |
| POST | `/api/pair/cancel` | 取消配对 |
| GET | `/api/logs/tail?lines=200` | 尾部日志（辅助诊断，非生产长期连接） |

所有响应 JSON；错误格式统一 `{ "error": { "code": "...", "message": "..." } }`。

## 8. ClawEnv 左侧导航与管理页

### 8.1 导航项

左侧新增一个 Bridge 条目（沙盒/实例/Apps/**Bridge**/Settings），icon 用一个"连接/塔"形图标。

### 8.2 管理页（SolidJS 原生，不走 WebView）

页面分区：

- **顶部状态带**：信令连接 ✓ / ✗、daemon 存活 ✓ / ✗、已配对设备数、当前活跃连接数
- **Channel 列表**：每行一个 Claw 实例；开关 enabled、勾选 allow_dashboard；点行展开看流量
- **配对设备**：列表 + "配对新设备"按钮（显示二维码）；每行有"吊销"
- **日志面板**：最近 200 行 bridge 日志（轮询 `/api/logs/tail`）
- **Bridge 控制**：右上角下拉 → 重启 daemon / 查看详细状态

### 8.3 为什么不用 WebView 嵌 bridge 内置 UI

- 铁律 7（Claw 管理页是 WebView）是针对 Claw 的，bridge 不是 Claw
- bridge 保持最小化 — 只有 JSON API，不带 Web 资源，体积、攻击面更小
- 管理页内容简单（状态 + 列表 + 二维码），SolidJS 直接画效率更高
- 避免额外一层技术栈

## 9. 监控（与 docs/21 交叉）

ClawEnv 需要在统一的监控视图（tray 气泡、Bridge 页顶部状态带）里覆盖：

- daemon 存活（launchd/systemd/task scheduler 查询）
- `/api/health` 是否 200
- 信令连接是否就绪（来自 `/api/status`）
- channel 数量 vs `InstanceConfig` 记录数量对账（不一致即告警）
- peer 数 / 字节数（展示，不告警）

## 10. 升级流程

1. ClawEnv 主程序更新（`app-update.json` 通知 → 用户同意）
2. 新版下载 → 含新版 bridge 二进制
3. 安装阶段：停止 bridge daemon → 替换 `~/.clawenv/bridge/bridgen` → 启动 daemon
4. daemon 读取既有 config + keypair，无需重新配对
5. schema 变更（config/identity 格式）需要向后兼容至少一个大版本

## 11. 架构调整清单

| 位置 | 改动 |
|---|---|
| `core/src/bridge/`（新） | `daemon.rs` / `admin_client.rs` / `channels.rs` / `identity.rs` |
| `core/src/config/models.rs` | `InstanceConfig` 增 `bridge_port: u16`；新增 `BridgeChannel` 结构 |
| `core/src/claw/descriptor.rs` | 无改动（bridge 与 Claw 解耦） |
| `core/src/config/keychain.rs` | 新增 key：`bridge-device-privkey`、`bridge-admin-token` |
| `cli/src/` | 新增 `bridge` 子命令组（status / pair / reload / restart） |
| `assets/` | bridge 二进制三平台预编译随 release bundle 一起装（`assets/bridge/{macos,linux,windows}/`） |
| `src/pages/Bridge/` | 新增管理页（状态 / channels / peers / 配对 / 日志） |
| `src/App.tsx` | `LaunchState` 加 Bridge 路由（铁律 5） |
| 系统集成 | 新增 LaunchAgent plist / systemd unit / Task XML 三平台模板 |

## 12. 分期

### MVP（3-4 周）

- bridge 二进制打包与三平台 daemon 注册
- channels.toml 下发 + reload
- admin API：health / status / channels / peers / pair（核心五个）
- 首次 keypair 生成 + pairing
- 前端 Bridge 页基础版（状态 + channels 列表 + 配对）
- 目标：手机能扫码连上，能访问 gateway

### V1（2-3 周）

- 吊销设备、流量统计、日志面板
- 配置变更的文件监听（实现热重载）
- 监控与 tray 集成
- 升级流程验证

### V2（长期）

- 多设备组管理、访问策略（按 channel 粒度）
- 分享链接（给朋友临时访问）
- 带宽限制 / 流量告警

## 13. 验收标准（MVP）

1. 首次启动 ClawEnv → bridge 自动注册为 daemon 并起来（重启电脑后自启）
2. 关闭 ClawEnv tray → bridge 仍在线，手机端仍能访问
3. 新装 Claw 实例 → channel 自动出现在 `/api/channels`，手机端看到对应条目
4. 手机端扫码配对成功 → 能 HTTP 请求到 Claw gateway（延迟、错误码、payload 透明透传）
5. 升级 ClawEnv → bridge 被重启但 keypair / 已配对设备保留
6. `clawcli bridge status --json` 输出结构化状态，Tauri 前端稳定解析
