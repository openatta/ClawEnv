# 9. 配置格式与目录结构

## 9.1 主配置文件：`~/.clawenv/config.toml`

```toml
# ClawEnv 主配置
[clawenv]
version    = "1.0.0"
user_mode  = "general"   # "general" | "developer"
language   = "zh-CN"
theme      = "system"    # "light" | "dark" | "system"

[clawenv.updates]
auto_check          = true
check_interval_hours = 24
auto_snapshot_before_upgrade = true
snapshot_retention_count     = 5

[clawenv.security]
# API Key 统一存储在系统 Keychain，此处不存明文
keychain_backend = "system"

[clawenv.tray]
enabled              = true       # 是否启用系统托盘常驻
start_minimized      = false      # 启动时最小化到托盘（不显示主窗口）
show_notifications   = true       # 是否显示系统通知
monitor_interval_sec = 5          # 实例状态轮询间隔（秒）

[clawenv.proxy]
enabled     = false
http_proxy  = ""                     # e.g. "http://proxy.example.com:8080"
https_proxy = ""                     # 留空则跟随 http_proxy
no_proxy    = "localhost,127.0.0.1"  # 不走代理的地址
# 代理认证凭证存储在系统 Keychain，此处不存明文
auth_required = false
auth_user     = ""                   # 用户名（明文，非敏感）
# auth_password 存储在 Keychain: clawenv/proxy-password

# ─── 实例列表 ───

[[instances]]
name         = "default"
claw_type    = "openclaw"
version      = "2.1.3"

# sandbox_type 对应三个对等后端之一
# "wsl2-alpine" | "lima-alpine" | "podman-alpine" | "native"
# 注意: "native" 为开发者专用模式（无沙盒隔离，直接在宿主机运行），
#       仅在 user_mode = "developer" 时可用，不建议普通用户使用
sandbox_type = "wsl2-alpine"

# sandbox_id 在各后端中的标识符
# WSL2:  distro 名称 ("ClawEnv-Alpine")
# Lima:  VM 名称    ("clawenv-default")
# Podman: 容器名称  ("clawenv-default")
sandbox_id   = "ClawEnv-Alpine"

created_at       = "2026-04-01T10:00:00Z"
last_upgraded_at = "2026-04-03T09:30:00Z"

[instances.openclaw]
default_model   = "claude-sonnet-4-20250514"
heartbeat_model = "claude-haiku-4-5-20251001"
gateway_port    = 3000
webchat_enabled = true

[instances.openclaw.channels]
telegram_enabled = true
whatsapp_enabled = false
discord_enabled  = false

[instances.browser]
enabled  = true
mode     = "cdp-extension"   # "cdp-extension" | "headless-chromium" | "fingerprint"
cdp_port = 9222
profile  = "clawenv"

[instances.resources]
memory_limit_mb = 512
cpu_cores       = 2
workspace_path  = "~/.clawenv/workspaces/default"
```

---

## 9.2 项目源码结构

```
clawenv/
├── Cargo.toml                      # Workspace 根
├── Cargo.lock
├── package.json                    # 前端依赖
├── tsconfig.json                   # TypeScript 配置
├── vite.config.ts                  # Vite 构建配置
├── index.html                      # HTML 入口
│
├── core/                           # 核心逻辑（无 UI 依赖）
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── launcher.rs             # 启动检测状态机
│       ├── tests.rs                # 单元测试
│       ├── monitor.rs              # 实例状态监控（驱动托盘刷新）
│       ├── platform/
│       │   ├── mod.rs
│       │   └── detector.rs         # OS / 架构 / 环境检测
│       ├── sandbox/
│       │   ├── mod.rs              # SandboxBackend trait
│       │   ├── wsl.rs              # WslBackend（Windows）
│       │   ├── lima.rs             # LimaBackend（macOS）
│       │   ├── podman.rs           # PodmanBackend（Linux）
│       │   └── native.rs           # NativeBackend（开发者模式可选）
│       ├── manager/
│       │   ├── mod.rs              # Manager 模块入口
│       │   ├── install.rs          # 安装流程
│       │   ├── instance.rs         # 实例 CRUD
│       │   └── upgrade.rs          # 升级 / 回滚
│       ├── config/
│       │   ├── mod.rs              # Config 模块入口
│       │   ├── manager.rs          # ConfigManager
│       │   ├── keychain.rs         # 系统 Keychain 集成
│       │   ├── models.rs           # 配置数据结构（BrowserConfig, ChannelsConfig 等）
│       │   └── proxy.rs            # 代理配置与沙盒注入
│       ├── update/
│       │   ├── mod.rs              # Update 模块入口
│       │   └── checker.rs          # GitHub Releases API + 版本检查
│       └── browser/
│           ├── mod.rs              # BrowserBackend trait
│           └── chromium.rs         # 沙盒内 Chromium 管理
│
├── tauri/                          # Tauri GUI 应用
│   ├── Cargo.toml
│   ├── build.rs                    # Tauri 构建脚本
│   ├── tauri.conf.json             # Tauri 应用配置
│   └── src/
│       ├── main.rs                 # Tauri 入口
│       ├── tray.rs                 # System Tray（图标/菜单/事件）
│       └── ipc/
│           └── mod.rs              # Tauri IPC 命令（统一模块，暴露给前端）
│
├── cli/                            # 纯 CLI（开发者模式）
│   ├── Cargo.toml
│   └── src/
│       └── main.rs                 # clap 命令树
│
├── src/                            # 前端（SolidJS）
│   ├── App.tsx                     # 启动器路由（LaunchState 状态机）
│   ├── index.tsx                   # SolidJS 入口
│   ├── layouts/
│   │   └── MainLayout.tsx          # Slack 风格主布局（图标栏 + 内容区）
│   ├── components/
│   │   ├── IconBar.tsx             # 左侧图标导航栏（56px，含所有图标项）
│   │   ├── UpgradePrompt.tsx       # 升级提示弹窗（覆盖层）
│   │   └── NoVncPanel.tsx          # noVNC 人工介入面板
│   ├── pages/
│   │   ├── ModeSelect.tsx          # 首次运行：模式选择（保存到 config）
│   │   ├── Install/
│   │   │   └── index.tsx           # 安装向导（7 步合并实现，连接后端 IPC）
│   │   ├── Home.tsx                # Dashboard（实例卡片+健康状态+操作按钮）
│   │   ├── OpenClawPage.tsx        # OpenClaw 管理页（Tab 栏 + iframe WebView）
│   │   └── Settings.tsx            # 设置页（读写 config 持久化）
│   └── styles/
│       └── global.css              # TailwindCSS 入口
│
└── assets/                         # 平台模板
    ├── lima/
    │   └── clawenv-alpine.yaml     # Lima VM 模板（含 cgroup v2 fix）
    └── podman/
        └── Containerfile           # Alpine + OpenClaw 容器定义
```

## 9.3 用户数据目录

```
~/.clawenv/
├── config.toml                     # 主配置（不含 API Key 明文）
├── clawenv.log                     # 滚动日志
├── instances/
│   └── default/
│       └── instance.toml           # 实例元数据
├── snapshots/
│   └── default/
│       ├── manifest.toml
│       └── *.tar.gz                # WSL2 快照（或 Podman image 引用）
├── workspaces/
│   └── default/                    # 挂载到沙盒的工作目录
├── templates/                      # 从 assets/ 复制并实例化
└── skills/                         # 本地 Skill 开发目录
```
