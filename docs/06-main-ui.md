# 6. 主 UI 设计

## 6.1 布局概览

采用类 Slack 的经典三栏布局：**左侧窄图标栏 + 内容区**。
图标栏固定在左侧，宽度 56px，始终可见，提供全局导航。

```
┌──┬─────────────────────────────────────────────────┐
│  │                                                 │
│  │                                                 │
│H │                                                 │
│o │              内容区                              │
│m │           （根据选中的                            │
│e │            图标切换）                             │
│  │                                                 │
│──│                                                 │
│O │                                                 │
│C │                                                 │
│  │                                                 │
│  │                                                 │
│  │                                                 │
│  │                                                 │
│  │                                                 │
│──│                                                 │
│S │                                                 │
│U │                                                 │
└──┴─────────────────────────────────────────────────┘
```

## 6.2 左侧图标栏

图标栏分为上、中、下三个区域，采用纯图标 + tooltip 的形式，不显示文字：

```
┌──────┐
│      │
│ LOGO │  ← ClawEnv logo，点击回到 Home
│      │
├──────┤
│      │
│  🏠  │  ← Home（Dashboard）
│      │
│  🦞  │  ← OpenClaw（实例管理页/WebUI）
│      │
│      │  ← 未来可扩展更多图标（Phase 4: ZeroClaw 等）
│      │
│      │
│      │
│      │
├──────┤
│  ⚙️  │  ← 设置
│      │
│  👤  │  ← 用户登录（暂不实现，灰显）
│      │
└──────┘
```

### 图标定义

| 位置 | 图标 | 名称 | 功能 | 状态 |
|------|------|------|------|------|
| 上部 | Logo | — | 点击回到 Home | Phase 1 |
| 中上 | Home 图标 | Home | Dashboard 总览页 | Phase 1 |
| 中部 | OpenClaw 图标 | OpenClaw | OpenClaw 管理页（内嵌 WebView） | Phase 1 |
| 下部 | 齿轮图标 | 设置 | 应用设置页 | Phase 1 |
| 下部 | 用户头像 | 用户 | 登录/账户（暂不实现） | 灰显占位 |

### 图标交互

- **选中态**：左侧 3px 白色竖线指示条 + 图标高亮
- **悬浮态**：tooltip 显示页面名称（如 "Home"、"OpenClaw"）
- **通知徽标**：图标右上角可显示红点（如 OpenClaw 有异常、设置有待处理更新）

## 6.3 Home 页（Dashboard）

Home 是默认首页，提供一览式的状态总览：

```
┌──┬─────────────────────────────────────────────────┐
│  │  Home                                           │
│  │─────────────────────────────────────────────────│
│  │                                                 │
│H │  ┌─ 实例状态 ─────────────────────────────────┐ │
│o │  │                                             │ │
│m │  │  ● default    OpenClaw v2.1.3    运行中     │ │
│e │  │    WSL2 + Alpine   CPU 12%   MEM 156MB     │ │
│  │  │    运行时间: 3d 12h                         │ │
│  │  │                                             │ │
│──│  │    [停止]  [重启]  [查看日志]               │ │
│O │  │                                             │ │
│C │  └─────────────────────────────────────────────┘ │
│  │                                                 │
│  │  ┌─ 安全状态 ────────────────────────────┐      │
│  │  │  ✓ 所有组件版本最新                    │      │
│  │  │  ✓ 无已知 CVE                         │      │
│  │  │  上次检查: 2 小时前     [立即检查]     │      │
│──│  └────────────────────────────────────────┘      │
│S │                                                 │
│U │  ┌─ 快捷操作 ────────────────────────────┐      │
│  │  │  [打开 OpenClaw]  [浏览器集成]  [日志] │      │
│  │  └────────────────────────────────────────┘      │
└──┴─────────────────────────────────────────────────┘
```

### Home 页内容区块

| 区块 | 内容 | 数据来源 |
|------|------|---------|
| 实例状态卡片 | 每个实例的名称、版本、运行状态、资源用量 | `InstanceMonitor` + `backend.stats()` |
| 安全状态 | CVE 检查结果、版本是否最新 | `update/checker.rs` |
| 快捷操作 | 常用功能的快捷入口 | 静态链接 |
| 系统信息 | 平台、沙盒类型、ClawEnv 版本 | `platform/detector.rs` |

**开发者模式额外显示**：
- 多实例列表（而非单卡片）
- 快照管理入口
- Skill 开发快捷入口

## 6.4 OpenClaw 页

OpenClaw 页**不是 ClawEnv 自己实现的管理界面**，而是通过 Tauri 内嵌 WebView
直接加载 OpenClaw 自带的 Web 管理面板（默认 `http://127.0.0.1:3000`）。

```
┌──┬─────────────────────────────────────────────────┐
│  │  OpenClaw                           ● 运行中 [↗]│
│  │─────────────────────────────────────────────────│
│  │  [default (sandbox)] [staging (sandbox)] [native (native)]  ← Tab 栏│
│  │─────────────────────────────────────────────────│
│H │                                                 │
│o │  ┌─────────────────────────────────────────┐    │
│m │  │                                         │    │
│e │  │     OpenClaw 内置 Web 管理面板           │    │
│  │  │     (WebView 加载选中实例的 gateway URL) │    │
│──│  │                                         │    │
│O │  │     ┌─ Conversations ──────────┐        │    │
│C │  │     │  ...                     │        │    │
│  │  │     │  OpenClaw 原生 UI        │        │    │
│  │  │     │  消息列表/Agent 配置等    │        │    │
│  │  │     │  ...                     │        │    │
│  │  │     └──────────────────────────┘        │    │
│  │  │                                         │    │
│  │  └─────────────────────────────────────────┘    │
│──│                                                 │
│S │  状态栏: 连接正常 | Gateway: 127.0.0.1:3000     │
│U │                                                 │
└──┴─────────────────────────────────────────────────┘
```

### OpenClaw 页行为

| 场景 | 行为 |
|------|------|
| OpenClaw 运行中 | WebView 加载管理面板 URL |
| OpenClaw 已停止 | 显示占位页：「OpenClaw 未运行」+ [启动] 按钮 |
| OpenClaw 启动中 | 显示加载动画 + 状态文字，启动完成后自动加载 |
| 连接断开 | 显示重连提示，自动重试（最多 5 次，间隔 2 秒） |
| 多实例 Tab 栏 | 顶部 Tab 栏始终显示所有实例，每个 Tab 显示实例名 + 状态点（绿色=运行中 / 灰色=已停止 / 红色=异常），点击 Tab 切换 WebView 加载对应实例的 gateway URL |
| Tab 标签格式 | 每个 Tab 显示实例类型，格式为 `name (type)`，如 "default (sandbox)"、"dev (native)"，方便区分实例运行环境 |

### 人工介入面板（noVNC）

当 OpenClaw 运行中的自动化任务需要人工操作（登录、CAPTCHA、OAuth 等），
OpenClaw 页会在 WebView 上方弹出 noVNC 面板：

```
┌──┬─────────────────────────────────────────────────┐
│  │  OpenClaw    [default (sandbox)]    ● 运行中     │
│  │─────────────────────────────────────────────────│
│  │  ⚠ 需要人工操作 — 请在下方完成登录后点击"继续"    │
│  │  ┌─ noVNC 实时画面 ────────────────────────┐    │
│  │  │                                         │    │
│  │  │   🔒 请输入密码                          │    │
│  │  │   用户名: [admin        ]               │    │
│  │  │   密码:   [____________]                │    │
│  │  │   [登录]                                │    │
│  │  │                                         │    │
│  │  │   (鼠标/键盘操作实时转发到沙盒内浏览器)     │    │
│  │  └─────────────────────────────────────────┘    │
│  │  [继续自动执行]           [全屏]  [↗ 新窗口]    │
│  │                                                 │
│  │  ── OpenClaw 管理面板（下方，暂时模糊）──────     │
└──┴─────────────────────────────────────────────────┘
```

| 行为 | 说明 |
|------|------|
| 弹出时机 | 收到 `human-intervention-needed` 事件时自动弹出 |
| 面板位置 | 覆盖在 OpenClaw WebView 上方，WebView 内容模糊化处理 |
| 操作方式 | 用户在 noVNC 画面中直接用鼠标和键盘操作沙盒内的 Chromium |
| 完成操作 | 点击"继续自动执行"→ 关闭 noVNC 面板 → Chromium 切回 headless |
| 全屏模式 | 点击"全屏"将 noVNC 面板扩展为全窗口，方便复杂操作 |
| 新窗口 | 点击"↗ 新窗口"在独立窗口中打开 noVNC，不占用主界面空间 |
| System Tray | 弹出同时托盘通知用户"需要手动操作"，点击托盘通知可聚焦到面板 |

### 顶部工具栏

- 左侧：页面标题 "OpenClaw" + 实例名
- 右侧：运行状态指示灯 + `[↗]` 按钮（在系统默认浏览器中打开）

### WebView 配置

```rust
// tauri/src/ipc/openclaw_page.rs

/// 获取 OpenClaw WebUI 的 URL
/// instance_name: 指定实例名称，用于 Tab 切换时加载对应实例的管理面板；
///                传 None 时默认加载 "default" 实例。
#[tauri::command]
pub async fn get_openclaw_url(instance_name: Option<String>) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let name = instance_name.unwrap_or_else(|| "default".into());
    let instance = config.get_instance(&name)
        .ok_or(format!("实例 '{}' 不存在", name))?;
    Ok(format!("http://127.0.0.1:{}", instance.openclaw.gateway_port))
}

/// 列出所有实例的基本信息，供前端渲染 Tab 栏
#[tauri::command]
pub async fn list_instances() -> Result<Vec<InstanceTab>, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    Ok(config.instances.iter().map(|inst| InstanceTab {
        name: inst.name.clone(),
        instance_type: inst.sandbox_type.clone(), // "sandbox" | "native"
        gateway_port: inst.openclaw.gateway_port,
        status: inst.runtime_status(),            // "running" | "stopped" | "error"
    }).collect())
}
```

### 多实例端口分配策略

每个 OpenClaw 实例拥有独立的 `gateway_port`，避免多实例同时运行时端口冲突。
端口在创建实例时自动分配，从 3000 起递增（3000、3001、3002...），也可在
`config.toml` 中按实例手动指定：

```toml
# config.toml — 多实例端口配置示例

[[instances]]
name = "default"
sandbox_type = "sandbox"
[instances.openclaw]
gateway_port = 3000

[[instances]]
name = "staging"
sandbox_type = "sandbox"
[instances.openclaw]
gateway_port = 3001

[[instances]]
name = "native"
sandbox_type = "native"
[instances.openclaw]
gateway_port = 3002
```

分配规则：
- 自动分配时扫描已有实例端口，取最大值 +1
- 端口范围限定在 3000-3099，超出时提示用户手动指定
- 实例删除后其端口可被后续新实例复用

## 6.5 设置页

```
┌──┬─────────────────────────────────────────────────┐
│  │  设置                                           │
│  │─────────────────────────────────────────────────│
│  │                                                 │
│  │  常规                                           │
│  │  ─────────────────────────────────────────      │
│  │  用户模式      [普通用户 ▾]                      │
│  │  语言          [简体中文 ▾]                      │
│  │  主题          [跟随系统 ▾]                      │
│  │                                                 │
│  │  System Tray                                    │
│  │  ─────────────────────────────────────────      │
│  │  启用托盘常驻   [✓]                             │
│  │  启动时最小化   [ ]                             │
│  │  显示通知      [✓]                              │
│  │                                                 │
│  │  更新                                           │
│  │  ─────────────────────────────────────────      │
│  │  自动检查更新   [✓]                             │
│  │  检查间隔      [24 小时 ▾]                      │
│  │  升级前自动快照 [✓]                             │
│  │                                                 │
│  │  网络                                           │
│  │  ─────────────────────────────────────────      │
│  │  使用代理       [ ]                             │
│  │  HTTP 代理      [________________________]      │
│  │  HTTPS 代理     [________________________]      │
│  │  不走代理       [localhost,127.0.0.1     ]      │
│  │  代理认证       [ ]                             │
│  │  [测试连接]                                     │
│  │                                                 │
│  │  关于                                           │
│  │  ─────────────────────────────────────────      │
│  │  ClawEnv 版本: 1.0.0                            │
│  │  平台: macOS 14.2 (Apple M2)                    │
│  │  沙盒: Lima + Alpine Linux                      │
│  │                                                 │
└──┴─────────────────────────────────────────────────┘
```

## 6.6 用户登录页（暂不实现）

左侧图标栏最下方的用户头像图标为灰显状态，点击显示：

```
┌──────────────────────┐
│  即将推出             │
│                      │
│  用户登录功能正在     │
│  开发中，敬请期待     │
│                      │
└──────────────────────┘
```

预留用于 Phase 3+ 对接 Atta Cloud 账户体系。

## 6.7 前端路由结构

```typescript
// src/routes.ts

const routes = [
  { path: "/",          component: Home },         // Dashboard
  { path: "/openclaw",  component: OpenClawPage },  // OpenClaw WebView
  { path: "/settings",  component: Settings },       // 设置
  { path: "/user",      component: UserPlaceholder }, // 用户（占位）
];
```

## 6.8 前端源码结构更新

```
src/
├── App.tsx                         # 启动器路由（LaunchState 状态机）
├── index.tsx                       # SolidJS 入口
├── layouts/
│   └── MainLayout.tsx              # Slack 风格主布局（图标栏 + 内容区）
├── components/
│   ├── IconBar.tsx                  # 左侧图标导航栏（56px，含所有图标项）
│   ├── UpgradePrompt.tsx           # 升级提示弹窗（覆盖层）
│   └── NoVncPanel.tsx              # noVNC 人工介入面板
├── pages/
│   ├── ModeSelect.tsx              # 首次运行：模式选择（保存到 config）
│   ├── Install/
│   │   └── index.tsx               # 安装向导（7 步合并实现，连接后端 IPC）
│   ├── Home.tsx                    # Dashboard（实例卡片+健康状态+操作按钮）
│   ├── OpenClawPage.tsx            # OpenClaw 管理页（Tab 栏 + iframe WebView）
│   └── Settings.tsx                # 设置页（读写 config 持久化）
└── styles/
    └── global.css                  # TailwindCSS 入口
```
