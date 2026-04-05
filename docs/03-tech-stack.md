# 3. 技术栈决策

## 3.1 核心技术栈

```
后端语言:    Rust (2021 edition)
GUI 框架:    Tauri v2
前端框架:    SolidJS（轻量响应式，与 Tauri IPC 配合好）
前端样式:    TailwindCSS v4
CLI 框架:    clap v4（derive 模式）
配置格式:    TOML
沙盒底座:    Alpine Linux（三平台统一）
沙盒机制:    WSL2（Windows）/ Lima（macOS）/ Podman（Linux）
```

## 3.2 Rust Crate 依赖清单

```toml
[workspace]
members = ["core", "tauri", "cli"]

# core/Cargo.toml（平台无关的核心逻辑）
[dependencies]
# 异步运行时
tokio         = { version = "1",    features = ["full"] }
async-trait   = "0.1"

# 序列化
serde         = { version = "1",    features = ["derive"] }
toml          = "0.8"
serde_json    = "1"
serde_yaml    = "0.9"

# 网络（检查版本/CVE）
reqwest       = { version = "0.12", features = ["json", "rustls-tls"] }

# 版本号比较
semver        = "1"

# 系统 Keychain（API Key 安全存储）
keyring       = "2"

# 日志
tracing           = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# 错误处理
anyhow    = "1"
thiserror = "1"

# 系统路径
dirs = "5"

# 时间
chrono = { version = "0.4", features = ["serde"] }

# tauri/Cargo.toml（GUI 层）
[dependencies]
clawenv-core = { path = "../core" }
tauri          = { version = "2", features = ["shell-open", "notification"] }
tauri-plugin-shell        = "2"
tauri-plugin-fs           = "2"
tauri-plugin-notification = "2"

# cli/Cargo.toml（CLI 层）
[dependencies]
clawenv-core = { path = "../core" }
clap           = { version = "4", features = ["derive", "color"] }
indicatif      = "0.17"   # 进度条
console        = "0.15"   # 终端颜色
dialoguer      = "0.11"   # 交互式提示
```

## 3.3 前端依赖

```json
{
  "dependencies": {
    "solid-js": "^1.9",
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-shell": "^2",
    "@tauri-apps/plugin-fs": "^2",
    "@tauri-apps/plugin-notification": "^2"
  },
  "devDependencies": {
    "vite": "^6",
    "vite-plugin-solid": "^2",
    "@tailwindcss/vite": "^4",
    "typescript": "^5"
  }
}
```
