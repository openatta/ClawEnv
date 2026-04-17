# 16. Hardware Device Integration

智能硬件设备对接 Agent（OC / Hermes）的架构设计与实施计划。

## 16.1 Overview

硬件设备具备语音输入/输出、文本输入、浏览器等能力，需要与 ClawEnv 管理的 Agent 实例交互。
设计原则：**Agent 零改动，ClawEnv 最小改动，硬件自主决定呈现方式**。

## 16.2 Architecture

两条独立通路，职责分离：

- **Channel**：硬件 ↔ Agent Gateway，双向对话（文本 / JSON / 文件），所有数据交换走此通道
- **Bridge Notify**：Agent → MCP hw-notify → Bridge → 硬件，单向通知推送

```
┌─────────────────────────┐            ┌──────────────────────────────┐
│     Hardware Device      │            │   Host — Bridge Server       │
│                          │            │   (Axum 0.8, port 3100)      │
│  ┌─────┐  ┌─────┐       │            │                              │
│  │ STT │  │ TTS │       │            │  POST /api/hw/register       │
│  └──┬──┘  └──▲──┘       │            │  POST /api/hw/unregister     │
│     │ text   │ text      │            │  GET  /ws/hw  (WebSocket)    │
│  ┌──▼────────┴────────┐  │   WS/HTTP  │  POST /api/hw/notify         │
│  │  Channel Client     ├─┼───────────►│                              │
│  └────────────────────┘  │            └──────────┬───────────────────┘
│                          │                       │
│  ┌────────────────────┐  │   WS long   ┌────────▼───────────────────┐
│  │  Notify Receiver   ◄─┼═════════════╡  Sandbox / Native           │
│  └────────────────────┘  │  connection  │                            │
│                          │              │  Agent (OC / Hermes)        │
│  ┌────────────────────┐  │              │  ├── hw-notify MCP plugin   │
│  │  Browser / Display  │  │              │  ├── mcp-bridge             │
│  │  (hw decides render)│  │              │  └── hil-skill              │
│  └────────────────────┘  │              └────────────────────────────┘
└─────────────────────────┘
```

### Key Decisions

1. **STT / TTS 在硬件端完成** — Agent 只收发文本，无语音依赖
2. **Channel 承载全量数据** — 文本、JSON 结构化数据、文件引用，硬件自行决定如何呈现（渲染表格、打开链接、播报语音等）
3. **MCP 仅做通知推送** — Agent 主动唤醒设备时调用 `notify()` tool，不用于数据传输
4. **Bridge 作为通知中转** — MCP plugin 调 Bridge API，Bridge 通过 WebSocket 长连接或 HTTP 回调推送到硬件
5. **MCP 全模式统一安装** — Sandbox 和 Native 模式均部署 hw-notify / mcp-bridge / hil-skill，保持能力一致

## 16.3 Bridge API Extensions

在现有 Bridge Server (`core/src/bridge/server.rs`) 上新增：

### New Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/hw/register` | POST | 注册硬件设备（name, callback_url, capabilities），返回 device_id |
| `/api/hw/unregister` | POST | 注销硬件设备（device_id） |
| `/ws/hw` | GET → WS Upgrade | 硬件 WebSocket 长连接，实时接收通知 |
| `/api/hw/notify` | POST | MCP plugin 调用，广播通知到所有已连接设备 |

### BridgeState Extension

```rust
// New fields in BridgeState
hw_devices: Vec<HwDevice>,                    // Registered devices (in-memory)
hw_ws_senders: Vec<(String, WsSender)>,       // Active WS connections (device_id, sender)
```

```rust
pub struct HwDevice {
    pub id: String,           // Auto-generated UUID
    pub name: String,         // Human-readable device name
    pub callback_url: String, // HTTP callback endpoint (fallback)
    pub capabilities: Vec<String>,  // ["voice", "browser", "display", ...]
    pub registered_at: String,
}
```

### Notification Delivery Strategy

1. **WebSocket 优先** — 设备在线（WS 已连接）时通过 WS 实时推送
2. **HTTP 回调 fallback** — WS 断开但设备已注册 callback_url 时，POST 到回调地址
3. **消息格式**：
```json
{
  "type": "notify",
  "device_id": "target-device-id or *",
  "message": "Analysis complete, 23 leads found",
  "level": "info | alert | action",
  "from_instance": "default",
  "timestamp": "2026-04-16T10:30:00Z"
}
```

### Infrastructure Readiness

| Condition | Status |
|-----------|--------|
| Axum 0.8 built-in WS support (`axum::extract::ws`) | Ready, no new deps |
| SharedState `Arc<RwLock<BridgeState>>` | Ready, extend directly |
| Tokio Notify pattern (HIL/Approval) | Reusable for broadcast |
| Bridge runs in both sandbox + native modes | Confirmed |
| Bridge port fixed at 3100 (global) | Single endpoint for hw |

## 16.4 hw-notify MCP Plugin

极简 MCP plugin，一个 tool，两个版本（Node.js + Python）。

### Tool Definition

```
notify(message: string, level?: "info"|"alert"|"action", device_id?: string)
```

- `message`：通知内容
- `level`：紧急程度（默认 `info`）
- `device_id`：目标设备（默认 `*` 广播所有设备）

### Implementation Pattern

复用 mcp-bridge 的已有模式：
- Sandbox：stdio MCP Server，HTTP 转发到 Bridge `/api/hw/notify`
- Native：同样模式，bridge_url 为 `http://127.0.0.1:3100`
- Bridge URL 自动检测：env `CLAWENV_HOST_IP` → Podman alias → Lima default → localhost

### File Layout

```
assets/mcp/
├── mcp-bridge.mjs      (existing)
├── mcp-bridge.py        (existing)
├── hil-skill.mjs        (existing)
├── hil-skill.py         (existing)
├── hw-notify.mjs        (NEW — Node.js version, ~40 lines)
└── hw-notify.py         (NEW — Python version, ~40 lines)
```

## 16.5 Unified MCP Deployment

当前 Native 模式不部署任何 MCP plugin，调整为全模式一致：

### Deployment Paths

| Mode | Path | Registration |
|------|------|-------------|
| Sandbox | `/workspace/{plugin}/index.mjs` or `bridge.py` | `{agent} mcp set {name} '{json}'` inside sandbox |
| Native | `~/.clawenv/native/mcp/{plugin}/index.mjs` | `{agent} mcp set {name} '{json}'` on host |

### Plugins Deployed (All Modes)

| Plugin | OC (Node.js) | Hermes (Python) |
|--------|-------------|----------------|
| mcp-bridge (clawenv) | `index.mjs` | `bridge.py` |
| hil-skill (clawenv-hil) | `index.mjs` | `skill.py` |
| hw-notify (hw-notify) | `index.mjs` | `notify.py` |

### Install Flow Changes

**`core/src/manager/install.rs`** (sandbox): 在现有 MCP 部署段 (~line 371) 追加 hw-notify 部署和注册。

**`core/src/manager/install_native/mod.rs`** (native): 新增完整 MCP 部署段，将三个 plugin 写入 `~/.clawenv/native/mcp/` 并注册到 Agent。

## 16.6 Agent Channel Access

### OpenClaw (OC)

**现成可用**。OC Gateway 提供 HTTP + WebSocket API：
- 硬件设备实现 WS client 连 `ws://host:gateway_port`
- Gateway 已有 webchat 模式，消息格式为 JSON
- 需确认：`webchat_enabled: true` 配置、WS 消息 schema、认证方式

### Hermes Agent — Plan B Confirmed

**Hermes 自带 HTTP API Server**（OpenAI-compatible），方案 B 可行：

| Feature | Detail |
|---------|--------|
| Server | FastAPI + Uvicorn（`web` extra 提供） |
| Command | `hermes gateway` 启动 API Server |
| Default Port | `8642` |
| Protocol | OpenAI-compatible HTTP API |
| WebSocket | 支持 streaming + tool progress events |
| Session | `X-Hermes-Session-Id` header 维持对话上下文 |
| Auth | `API_SERVER_KEY` 环境变量 |
| Config | `~/.hermes/.env` 中 `API_SERVER_ENABLED=true` |

**结论：方案 B 确认可行**，Hermes 原生具备 HTTP API Server，无需自建 Gateway 代理。

**已知风险**：Hermes `[web]` extra 存在 uv.lock marker collapse bug（NousResearch/hermes-agent Issue #9569），
`fastapi` 和 `uvicorn` 依赖可能未正确解析。安装时需要 workaround：
```bash
pip install --break-system-packages fastapi "uvicorn[standard]" 2>/dev/null || true
```

**ClawEnv 需要的改动**：

1. `assets/claw-registry.toml` — Hermes 的 `gateway_cmd` 从空改为 `"gateway --port {port}"`（如果 Hermes 支持 `--port` 参数，否则通过环境变量注入端口）
2. `core/src/manager/install.rs` — Hermes 安装后配置 `.env` 文件启用 API Server + 安装 fastapi/uvicorn workaround
3. 端口适配 — Hermes 默认 8642，ClawEnv 分配的是 gateway_port (3000+)，需要对齐

**硬件接入方式**：与 OC 一致，WS/HTTP client 连 Agent Gateway，发送 OpenAI-format messages。硬件端可以用同一套 client 代码对接两种 Agent。

## 16.7 Combination Matrix

| | Sandbox | Native |
|---|---|---|
| **OC** | Channel: Gateway WS ✅ / MCP: hw-notify ✅ | Channel: Gateway WS ✅ / MCP: hw-notify ✅ |
| **Hermes** | Channel: `hermes gateway` ✅ / MCP: hw-notify ✅ | N/A (`supports_native = false`) |

三种有效组合，所有能力对等。

## 16.8 Implementation Phases

### Phase 0 — Bridge API Extension

| # | File | Change |
|---|------|--------|
| 0.1 | `core/src/bridge/server.rs` | Add 4 endpoints: register, unregister, ws/hw, notify |
| 0.2 | `core/src/bridge/server.rs` | Extend `BridgeState` with `hw_devices` + `hw_ws_senders` |
| 0.3 | `core/src/bridge/mod.rs` | Add `HwDevice` struct |

~200 lines. No dependencies to add (Axum 0.8 WS built-in).

### Phase 1 — hw-notify MCP Plugin

| # | File | Change |
|---|------|--------|
| 1.1 | `assets/mcp/hw-notify.mjs` | NEW — Node.js stdio MCP, single `notify()` tool |
| 1.2 | `assets/mcp/hw-notify.py` | NEW — Python version |

~80 lines total.

### Phase 2 — Unified MCP Install (Sandbox + Native)

| # | File | Change |
|---|------|--------|
| 2.1 | `core/src/manager/install.rs` | Add hw-notify deploy + register (sandbox) |
| 2.2 | `core/src/manager/install_native/mod.rs` | Add full MCP deploy (all 3 plugins) + register (native) |

~120 lines.

### Phase 3 — Hermes Gateway Activation

| # | File | Change |
|---|------|--------|
| 3.1 | `assets/claw-registry.toml` | Set Hermes `gateway_cmd` |
| 3.2 | `core/src/manager/install.rs` | Hermes post-install: configure `.env` for API Server |
| 3.3 | `core/src/manager/instance.rs` | Hermes startup: ensure API Server env vars injected |

~60 lines.

### Execution Order

```
Phase 0 → Phase 1 → Phase 2 → 联调验证 (OC Sandbox) → Phase 3 → 联调验证 (Hermes)
```

## 16.9 Open Items

1. **OC Gateway WS protocol** — 确认 JSON schema、认证方式
2. **Hermes `--port` flag** — 确认 `hermes gateway` 是否支持端口参数，还是只能通过 `.env` 配置
3. **Hermes web extra uv.lock bug** — NousResearch/hermes-agent Issue #9569，`[web]` extra 依赖可能未正确解析，安装时需要 workaround
4. **WS 心跳 / 断线重连** — Bridge WS 端需要 ping/pong 保活，硬件端需要自动重连
5. **多设备广播 vs 定向推送** — `device_id: "*"` 广播 vs 指定设备，是否需要设备分组
