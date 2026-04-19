# 代理架构

> 状态：正式 v1 | 创建：2026-04-19 | 关联：`core/src/config/proxy_resolver.rs`、`core/src/platform/download.rs`、`src/pages/SandboxPage.tsx`、`assets/mirrors.toml`

本项目所有代理相关设计的 SSOT。新增代理相关代码前必须阅读本文档。

## 1. 目标与非目标

**目标**
- 所有代理决策走同一个 resolver，单点真源
- 每个代理使用点都能清晰回答：用的什么代理？来自哪一层？什么时候会变？
- 跨平台（macOS / Windows GUI；Linux CLI）行为一致可推理
- Native vs 沙盒的边界在类型层表达，不靠运行时 if-else

**非目标**
- 不支持 PAC 脚本解析（检测到即提示改显式代理）
- 不默认启用 SOCKS（reqwest 不开 `socks` feature）
- 不支持 Linux GUI（CLAUDE.md 铁律）

## 2. 三个作用域（Scope）

所有读代理的代码都声明自己所属的作用域，resolver 按作用域返回结果。

```rust
pub enum Scope<'a> {
    /// 宿主机侧下载：git/node/lima/wsl/podman/alpine/npm
    Installer,
    /// Native claw 进程（Tauri/CLI spawn 的子进程）
    RuntimeNative,
    /// 沙盒内跑的 claw（通过 backend.exec 写 /etc/profile.d/proxy.sh）
    RuntimeSandbox {
        instance: &'a InstanceConfig,
        backend: &'a dyn SandboxBackend,
    },
}
```

**选择作用域 = 选择优先级链 + 选择输出形态。**

## 3. 优先级链

### Installer
1. shell env（开发者从终端启动，已有 `HTTPS_PROXY`）
2. `config.toml.clawenv.proxy`（用户在 Settings 显式设置）
3. OS 检测（macOS SCDynamicStore / Windows 注册表 / GNOME gsettings）
4. 无 → 直连

### RuntimeNative（**铁律：仅 OS 系统代理**）
1. OS 检测 — 唯一来源
2. 无 → 直连

Native 对 `config.toml.proxy` 和 `InstanceConfig.proxy` **一律不读**。铁律在类型层强制 —— Native 没有 VM，自然没有 `RuntimeSandbox` 作用域，不存在 per-instance 代理配置路径。

### RuntimeSandbox
1. `instance.proxy`（per-VM 覆盖，来自 SandboxPage VmCard 的代理按钮）
   - `mode = "none"`：显式直连
   - `mode = "sync-host"`：宿主代理 + URL 翻译（127.0.0.1 → host.lima.internal 等）
   - `mode = "manual"`：原样使用用户填的 URL
2. `config.toml.clawenv.proxy`（全局默认）
3. OS 检测（同样做 URL 翻译）
4. 无 → 直连

## 4. UI 归属（关键）

| UI 层 | 作用域 | 归属页面 |
|---|---|---|
| Settings 页 proxy 配置 | Installer（全局默认） | `src/pages/Settings.tsx` |
| Install 向导 StepNetwork | Installer（首次安装的一次性选择） | `src/pages/Install/StepNetwork.tsx` |
| **VM 代理按钮** | **RuntimeSandbox** | **`src/components/VmCard.tsx`（SandboxPage）** |
| Native claw | RuntimeNative | **无 UI** —— OS 层管理 |

**铁律：Native 在 UI 层没有代理入口**。ClawPage 不放代理按钮。Native 实例在 SandboxPage 不出现（没 VM），自然也没代理按钮。用户要调代理就改 macOS System Preferences / Windows Internet 选项 / Clash。

## 5. 四处状态 — 职责清晰

| 存储 | 语义 | 写入者 | 读取者 | 生命周期 |
|---|---|---|---|---|
| OS 系统代理 | 用户真实意图 | 用户（ClawEnv 外部） | `detect_os_system_proxy()` | 用户随时改 |
| GUI 进程 env | OS 代理缓存，供子进程继承 | Tauri 启动 + start hook + OS 变化 watcher | CLI / Native claw（继承） | 进程生命周期 |
| `config.toml.clawenv.proxy` | 用户对 OS 代理的全局覆盖 | Settings 页 / Install 向导 | resolver（Installer / RuntimeSandbox 回退） | 持久化 |
| `InstanceConfig.proxy` | per-VM 沙盒覆盖 | SandboxPage VmCard | resolver（仅 RuntimeSandbox） | 持久化 |

**VM 的 `/etc/profile.d/proxy.sh` 不是状态**，是 resolver 的输出物，在每次 VM 启动时按 resolver 结果重写。这意味着 export/import 时不会把旧代理带过去。

## 6. 核心 API

```rust
// core/src/config/proxy_resolver.rs

pub struct ProxyTriple {
    pub http: String,
    pub https: String,
    pub no_proxy: String,
    pub source: ProxySource,
}

#[derive(Debug, Clone, Copy)]
pub enum ProxySource {
    PerVm,         // InstanceConfig.proxy
    GlobalConfig,  // config.toml.clawenv.proxy
    OsSystem,      // scutil / registry / gsettings
    ShellEnv,      // parent process env
    None,          // direct
}

pub enum Scope<'a> {
    Installer,
    RuntimeNative,
    RuntimeSandbox {
        instance: &'a InstanceConfig,
        backend: &'a dyn SandboxBackend,
    },
}

impl<'a> Scope<'a> {
    /// Async because RuntimeSandbox may query VM (WSL resolv.conf).
    pub async fn resolve(&self, cfg: &ConfigManager) -> Option<ProxyTriple>;
}

// Apply:
pub fn apply_env(triple: &ProxyTriple);
pub fn apply_child_cmd(triple: &ProxyTriple, cmd: &mut Command);
pub async fn apply_to_sandbox(triple: &ProxyTriple, backend: &dyn SandboxBackend) -> Result<()>;
pub async fn clear_sandbox(backend: &dyn SandboxBackend) -> Result<()>;

// Detection only (no policy):
pub fn detect_os_system_proxy() -> Option<OsProxyInfo>;
pub async fn detect_os_system_proxy_async() -> Option<OsProxyInfo>;

// Watcher (OS proxy change):
pub fn spawn_os_proxy_watcher<F>(on_change: F) -> WatcherHandle
where F: Fn(Option<OsProxyInfo>) + Send + 'static;
```

**设计决策**：`resolve` 返回 `Option<ProxyTriple>`。`None` = 直连。`Some` 带 `source` 字段方便日志和 UI 展示。调用方**不再有 if-else 选代理来源**，只问 resolver。

## 7. Mirror 基础设施

`assets/mirrors.toml`：所有下载素材的版本 + URL 列表 + sha256 的单一来源。

```toml
[dugite]
tag              = "2.53.0-3"
upstream_version = "2.53.0"
commit           = "f49d009"
filename_tpl     = "dugite-native-v{upstream_version}-{commit}-{platform}.tar.gz"
urls = [
    "https://github.com/desktop/dugite-native/releases/download/v{tag}/{filename}",
    "https://ghfast.top/https://github.com/desktop/dugite-native/releases/download/v{tag}/{filename}",
    "https://mirror.ghproxy.com/https://github.com/desktop/dugite-native/releases/download/v{tag}/{filename}",
    "https://gh-proxy.com/https://github.com/desktop/dugite-native/releases/download/v{tag}/{filename}",
]
[dugite.sha256]
macos-arm64  = "..."
# ...

[node]
version = "v22.16.0"
# urls with {version}, {platform}, {ext} placeholders
# 4 sources: upstream + npmmirror + huaweicloud + tsinghua

[mingit]
# Windows only

[lima]
# Lima binary for macOS

[wsl-distro]
# Alpine WSL distro

[alpine-minirootfs]
# Podman base image on Linux
```

Loader：`core/src/config/mirrors_asset.rs` 提供 `AssetMirrors::load()` + `build_urls(asset, platform)` + `expected_sha256(asset, platform)`。所有 downloader 统一使用。

## 8. 下载 helper

`core/src/platform/download.rs::download_with_progress`：
- Connect timeout 15s
- 每 chunk stall 检测 60s
- 进度节流 1MiB 或 500ms
- 镜像 URL 列表遍历 + sha256 校验
- 签名统一，由 `AssetMirrors` 提供 URL 列表

消费者：
- Git（`install_native/mod.rs`）
- Node.js（`install_native/{macos,windows,linux}.rs` — Linux CLI 也用）
- Lima（`sandbox/lima.rs`）
- WSL distro（`sandbox/wsl.rs`）
- Podman image（`sandbox/podman.rs`）
- update checker（`update/checker.rs`，短超时版本）

## 9. 生命周期

### Install

```
Tauri 启动：
  1. apply_env(Scope::Installer.resolve(cfg))   // env 注入
Install 向导 StepNetwork：
  展示 detect_os_system_proxy()，用户确认/改写 → 保存 config.toml 全局
IPC install_openclaw：
  将 proxy_json 透传为 env 注入 CLI 子进程
CLI install：
  apply_env(Scope::Installer.resolve(cfg))      // 二次注入
  download_with_progress(...)                   // 自动读 env

创建 VM（关键：provision 三拍子）：
  (1) provision_preamble 拼入 Scope::Installer 的 proxy export 行
      → Lima YAML / WSL script 里 apk update/add 第一次跑就能走代理
      → Podman: opts.{http,https,no}_proxy 作为 --build-arg 传给 podman build
  backend.create(opts)                          // VM 首次 boot + apk 装包
  backend.start()                               // 确认运行
  (2) apply_to_sandbox(Scope::RuntimeSandbox.resolve(cfg, &instance, &backend), &backend)
      → 把 /etc/profile.d/proxy.sh 写为持久化代理配置
      → 和 provision-time 的 export 相同值，后续 VM shell / claw 进程都能看到
  mirrors::apply_mirrors(backend, ...)          // 如果配了镜像
  backend.exec("apk add ...")                   // 后续依赖装包，走持久化 proxy
  backend.exec("npm install openclaw ...")
```

**provision 三拍子的必要性**：
1. **provision 期间的 apk/npm** 跑在 VM 第一次 boot 阶段，走 `provision_preamble` 的 inline export（机制 1）
2. **持久化 proxy** 由 post-boot `apply_to_sandbox` 写 `/etc/profile.d/proxy.sh`（机制 2），供后续所有 shell/claw 继承
3. **导出时 scrub** 把 `/etc/profile.d/proxy.sh` 清掉（机制 3），bundle 永远 proxy-clean

机制 1 和 2 的代理值**永远相同**（都从 `Scope::Installer` / `Scope::RuntimeSandbox` 读），所以 VM 内看到的代理在 provision 和 post-boot 之间没有跳变。

### Start

```
Tauri start_instance IPC：
  refresh_os_proxy_env()                        // OS 变化 fallback
  spawn CLI
CLI start：
  if sandbox:
    backend.start()
    apply_to_sandbox(Scope::RuntimeSandbox.resolve(cfg, &instance, &backend), &backend)
  if native:
    ManagedShell::spawn_detached(...)            // 继承 env
```

### Export（仅 sandbox）

```
tar 之前：
  clear_sandbox(&backend)                      // 清 proxy.sh + npm config
manifest 写入 proxy_was_configured = true
```

### Import

```
解包完成 → VM 是 proxy-clean 的
读 manifest.proxy_was_configured
若 true：Import 向导走"代理配置"步骤
  模式：inherit / sync-host / manual / none
用户选完 → 写 InstanceConfig.proxy
第一次 start 时 resolver 生效
```

## 10. OS 代理变化监听

每平台实现统一接口 `spawn_os_proxy_watcher`：
- macOS：`SCDynamicStoreSetNotificationKeys` 订阅 `State:/Network/Global/Proxies`，在独立 `std::thread` 跑 CFRunLoop，变化通过 `tokio::sync::mpsc` 桥接回异步上下文
- Windows：优先 `WinHttpRegisterProxyChangeNotification`；若 feature 不稳则降级为 30s 轮询
- Linux：`gsettings monitor` 子进程 stdout 行解析（GUI 不支持，仅 CLI 诊断用）

**前端集成**：Tauri 收到变化 → `app.emit("os-proxy-changed", info)` → Home / SandboxPage 顶栏显示实时 proxy 状态指示器：

```
🟢 System proxy: http://127.0.0.1:7890 (macOS System Preferences)
```

## 11. VM 内连通性测试

```rust
#[tauri::command]
pub async fn test_instance_network(
    name: String,
    targets: Vec<String>,  // 预设 key 或自定义 URL
) -> Result<Vec<ConnTestResult>>;
```

预设目标分组：
- **international**：github, npm, openai, anthropic
- **china**：deepseek, dashscope.aliyun, hunyuan.tencent, npmmirror
- **custom**：用户填

实现：`backend.exec("curl -sS -m 8 -o /dev/null -w '%{http_code}|%{time_total}' <url> || echo FAIL")`。

UI：VmCard 点"代理"按钮打开 ProxyModal，底部有"测试连通性"区，一眼看出"代理通国外/国内/哪个都不通"。

## 12. 认证

```rust
pub struct InstanceProxyConfig {
    pub mode: String,
    pub http_proxy: String,
    pub https_proxy: String,
    pub no_proxy: String,
    pub auth_required: bool,
    pub auth_user: String,
    // 密码存 Keychain，key = "clawenv-proxy-<instance-name>"
}
```

Keychain API：
```rust
pub fn set_instance_proxy_password(instance: &str, password: &str) -> Result<()>;
pub fn get_instance_proxy_password(instance: &str) -> Result<String>;
pub fn delete_instance_proxy_password(instance: &str) -> Result<()>;
```

Resolver 在返回 triple 时把 `user:password@` 拼进 URL（复用现有 `proxy_url_with_auth`）。

**导出**：密码不跟 bundle 走（Keychain 本机存储）。导入方看到 `auth_required=true` 但没密码 → ProxyModal 提示重填。

## 13. 日志与诊断

统一 tracing target：`clawenv::proxy`：

```
DEBUG clawenv::proxy: resolve scope=Installer source=ShellEnv http=http://127.0.0.1:7890
DEBUG clawenv::proxy: resolve scope=RuntimeNative source=OsSystem http=http://127.0.0.1:7890
INFO  clawenv::proxy: sandbox proxy applied instance=default mode=sync-host
  raw=http://127.0.0.1:7890 rewritten=http://host.lima.internal:7890
```

### 诊断 CLI

```bash
clawcli proxy diagnose [--instance NAME]
```

输出所有 scope 的解析结果 + 连通性测试，一条命令支持用户问题定位。

## 14. 测试

### 单元测试（core）
- `Scope::resolve` 每个 scope × 每层优先级组合（6 + 1 + 8 = 15 cases）
- `rewrite_proxy_url_for_sandbox` 四后端 × 三 URL 形态
- `proxy_url_with_auth`
- `AssetMirrors::build_urls` 模板渲染
- `download_with_progress` 用 httpmock：成功 / fallback / checksum 不匹配 / stall

### 集成测试
- `InstanceProxyConfig` round-trip
- Keychain set/get/delete
- `apply_to_sandbox` 用 mock backend 验证写入内容

### 平台特定
- Windows 注册表：临时键隔离测试
- Linux gsettings：skip（CI 太重）
- macOS SCDynamicStore：skip（难 mock），依赖手测

## 15. 已知限制

- **PAC**：不解析。检测到提示用户改显式代理
- **SOCKS**：不支持。提示切 HTTP 桥
- **Clash TUN 模式**：OS 代理字段为空 → resolver 返回 None → 直连（TUN 在网络层转发，符合预期）
- **Windows 代理变化**：WinHTTP 通知 feature 稳定性不佳时降级为轮询（30s 粒度，用户可接受）
- **Linux GUI**：不支持（CLAUDE.md 铁律），Linux CLI 用户靠 shell env + `clawcli proxy diagnose`

## 16. 未来扩展点（非当前范围）

- per-domain 代理规则（Clash rules 风格）
- 多代理池 + 健康检查自动 failover
- 一个 VM 跑多个 claw 时的代理继承关系（当前 1 VM : 1 claw，未触发）

## 17. 目录索引

- `core/src/config/proxy_resolver.rs` — 本章所有 API 的实现
- `core/src/config/proxy.rs` — apply 动作（env / child / sandbox / keychain）
- `core/src/config/mirrors_asset.rs` — AssetMirrors loader
- `core/src/platform/download.rs` — download_with_progress
- `core/src/platform/proxy_watcher.rs` — OS 代理变化监听
- `assets/mirrors.toml` — 镜像数据
- `src/pages/SandboxPage.tsx` + `src/components/VmCard.tsx` — VM 代理 UI
- `src/pages/Install/StepNetwork.tsx` — 安装向导代理选择
- `src/pages/Settings.tsx` — 全局代理配置
