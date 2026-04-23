# ClawEnv v2 —— 统一 Ops 架构设计文档

> **范围**：完全独立于 v1（`core/`、`cli/`、`tauri/`）的新代码树，位于 `v2/`。
> **原则**：不改 v1 任何一行；v1 现有代码可作为"成熟依赖"被 v2 通过 path 引用。
> **成果形态**：`v2/core/`（clawops-core 库）+ `v2/cli/`（clawops 可执行文件）。

---

## 1. 背景与动机

### 1.1 v1 的现状

v1 代码树（`core/`）把"怎么跑命令"、"怎么装东西"、"怎么管沙盒"、"怎么调 Claw CLI"
和"怎么对外暴露"混在 `manager/upgrade.rs`、`manager/install.rs`、`sandbox/*.rs`
等文件里。具体症状：

- `manager/upgrade.rs` 和 `manager/install.rs` 是两条独立重写的流水线
  （安装路径修过的 chown/dashboard 预构建等，升级路径又漏了一遍）。
- Hermes 升级用 `git clone` 到已存在目录 → 必失败。
- 安装器的 URL 表散落在 `install_native/{linux,macos,windows}.rs` 和
  `sandbox/{lima,wsl,podman}.rs`，无统一目录、无缓存、无连通性诊断。
- WSL2 端口转发已经实现（`wsl.rs:676` 的 `edit_port_forwards`），
  但 CLI 层没有暴露，用户看不见也调不动。
- 没有 `claw`、`sandbox`、`native`、`download` 这些"运维视角"的子命令。

### 1.2 v2 的抽象目标

把 ClawEnv 的全部能力切成五层正交命令面：

```
clawops
├── claw       # ClawOps      —— 调用 hermes/openclaw 本身的 CLI 做管理
├── sandbox    # SandboxOps   —— 管 Lima/WSL2/Podman 的生命周期 + 端口 + 诊断
├── native     # NativeOps    —— 管宿主机上的 node/git/clawenv 安装状态
├── download   # DownloadOps  —— 管软件包目录 + 缓存 + 下载
└── instance   # 组合层       —— 跨四层的综合命令（create/destroy/health）
```

每层独立可测、独立可用、互不依赖（instance 是例外，它是组合）。

### 1.3 为什么做独立 v2 而不是原地重构

| 维度 | 原地重构 | 独立 v2 |
|---|---|---|
| 稳定性风险 | 高——动 manager/install.rs 可能影响现有用户 | 零——v1 一行不改 |
| 并行开发 | 难——同一文件无法并行 | 易——v2 新增文件，不冲突 |
| 验证路径 | 直接替换，出错难回滚 | 侧线验证→灰度切流→最终替换 |
| 心智成本 | 要理解新老两套如何拼接 | 一次性按新架构思考 |

v2 的正确思维：**先把新架构完整做出来、全绿测试，再谈什么时候切流 / 是否废弃 v1**。

---

## 2. 分层架构

```
┌─────────────────────────────────────────────────────────────────┐
│  CLI 层：clawops（v2/cli）                                       │
│    clap v4 解析 → 组装 Ops 对象 → 调 Ops 方法 → 打印结果          │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│  Ops 层：v2/core/src/{claw,sandbox,native,download}_ops          │
│    每个模块暴露一个 trait（ClawOps/SandboxOps/NativeOps/...）     │
│    + 一个或多个 impl（HermesCli / LimaOps / ...）                │
│    方法返回声明式数据（CommandSpec）或执行 async 动作             │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│  Common 层：v2/core/src/common                                   │
│    CommandSpec / OutputFormat / ExecEvent / ExecResult           │
│    CommandRunner trait + LocalProcessRunner impl                 │
│    CancellationToken / ProgressSink / OpsError                   │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│  Adapter 层：v2/core/src/adapters（可选）                        │
│    SandboxBackendRunner —— 桥接到 v1 的 SandboxBackend trait      │
│    V1DownloadAdapter    —— （如需）桥接到 v1 download_with_progress│
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│  v1 复用（path = "../core"）                                      │
│    clawenv-core::sandbox::{LimaBackend, WslBackend, PodmanBackend}│
│    clawenv-core::sandbox::SandboxBackend trait                    │
│    *不* 复用 v1 的 manager/{install,upgrade}.rs（那是 v1 的业务层）│
└─────────────────────────────────────────────────────────────────┘
```

**关键纪律**：
- v2 的 Ops 层**只依赖 Common 层和 Adapter 层**，不直接 use 任何 v1 类型。
- v1 类型只在 Adapter 层出现（实现了"跨边界"的胶水）。
- 这样未来可以无缝替换 Adapter（比如 v2 自己写 LimaBackend），不影响 Ops 层。

---

## 3. Common 层（v2/core/src/common）

### 3.1 `CommandSpec`

```rust
pub struct CommandSpec {
    pub binary: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<String>,
    pub cwd: Option<String>,
    pub timeout: Option<Duration>,
    pub output_format: OutputFormat,
}
pub enum OutputFormat { Plain, JsonLines, JsonFinal }
```

纯数据、可 Clone、可打印、可断言。所有 `ClawCli` / `SandboxOps` 的命令生成函数
返回它或它的列表。

### 3.2 `CommandRunner`

```rust
#[async_trait]
pub trait CommandRunner: Send + Sync {
    fn name(&self) -> &str;
    async fn exec(&self, spec: CommandSpec, cancel: CancellationToken)
        -> Result<ExecResult, CommandError>;
    fn exec_streaming(&self, spec: CommandSpec, cancel: CancellationToken)
        -> mpsc::Receiver<ExecEvent>;
}
```

两个实现：
- `LocalProcessRunner` —— tokio::process，支持 stdin/timeout/cancel/流式/JSON。
- `SandboxBackendRunner`（Adapter）—— 包装 `clawenv_core::sandbox::SandboxBackend`。

### 3.3 `CancellationToken`

自建（不引 tokio-util）：`Arc<{AtomicBool, Notify}>`。UI 按钮和 runner 共享。

### 3.4 `ProgressSink`

```rust
#[derive(Clone)]
pub struct ProgressSink(mpsc::Sender<ProgressEvent>);

pub struct ProgressEvent {
    pub percent: Option<u8>,
    pub stage: String,
    pub message: String,
}
```

所有 Ops 方法接受 `ProgressSink`——它是通用的进度上报通道，
UI / CLI / 日志任选其一消费。

### 3.5 `OpsError`

```rust
pub enum OpsError {
    Command(CommandError),         // 来自 CommandRunner
    Download(DownloadError),       // 下载/校验失败
    Parse(anyhow::Error),           // 输出解析失败
    Unsupported { what: String, reason: String },
    NotFound { what: String },
    Io(std::io::Error),
    Other(anyhow::Error),
}
```

统一的顶层错误，方便 CLI 分类打印。

---

## 4. ClawOps 层

### 4.1 已有基础

v1 已经实现了完整的 `claw_ops`（见 `docs/25-claw-ops-stage-a.md`）。v2 把它
**整体迁移**进 `v2/core/src/claw_ops/`，做两个改动：
- 使用 v2 的 `common::CommandSpec` 而非 v1 的类型（实际上签名一致，换 use 就行）。
- 接上 v2 的 `ProgressSink`（原 claw_ops 不直接用 ProgressSink，因为是声明式层，
  这次引入后 `update`/`doctor` 方法仍然返回 CommandSpec，执行才用 ProgressSink）。

### 4.2 trait

```rust
#[async_trait]
pub trait ClawCli: Send + Sync {
    fn id(&self) -> &str;
    fn binary(&self) -> &str;
    fn supports_native(&self) -> bool;

    fn update(&self, opts: UpdateOpts) -> CommandSpec;
    fn doctor(&self, opts: DoctorOpts) -> CommandSpec;
    fn config_get(&self, key: &str) -> CommandSpec;
    fn config_set(&self, key: &str, value: &str) -> CommandSpec;
    fn config_list(&self) -> CommandSpec;
    fn logs(&self, opts: LogsOpts) -> CommandSpec;
    fn status(&self) -> CommandSpec;
    fn version(&self) -> CommandSpec;
    fn help(&self, subcommand: Option<&str>) -> CommandSpec;
}
```

### 4.3 实现

- `HermesCli` —— 对应 `hermes update/doctor/config/...`（见 v1 文档的命令核实列表）。
- `OpenClawCli` —— 对应 `openclaw update --json/--channel/...`。

两者覆盖度、flag 映射、timeout 设置与 v1 相同。

### 4.4 仓库注册

```rust
pub struct ClawRegistry;
impl ClawRegistry {
    pub fn cli_for(id: &str) -> Option<Box<dyn ClawCli>> {
        match id {
            "hermes" => Some(Box::new(HermesCli)),
            "openclaw" => Some(Box::new(OpenClawCli)),
            _ => None,
        }
    }
    pub fn all() -> Vec<Box<dyn ClawCli>> { ... }
}
```

---

## 5. SandboxOps 层

### 5.1 trait

```rust
#[async_trait]
pub trait SandboxOps: Send + Sync {
    fn backend_kind(&self) -> BackendKind;
    fn capabilities(&self) -> SandboxCaps;

    // 生命周期
    async fn status(&self) -> Result<SandboxStatus, OpsError>;
    async fn start(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>;
    async fn stop(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>;
    async fn restart(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>;

    // 端口
    async fn list_ports(&self) -> Result<Vec<PortRule>, OpsError>;
    async fn add_port(&self, host: u16, guest: u16) -> Result<(), OpsError>;
    async fn remove_port(&self, host: u16) -> Result<(), OpsError>;

    // 诊断 / 修复
    async fn doctor(&self) -> Result<SandboxDoctorReport, OpsError>;
    async fn repair(&self, issue_ids: &[String], progress: ProgressSink)
        -> Result<(), OpsError>;

    // 监控
    async fn stats(&self) -> Result<ResourceStats, OpsError>;
    async fn dump_logs(&self, tail: Option<u32>) -> Result<String, OpsError>;
}
```

### 5.2 三个实现

每个实现内部持有 `Arc<dyn clawenv_core::sandbox::SandboxBackend>`：

```rust
pub struct LimaOps { backend: Arc<LimaBackend> }
pub struct WslOps  { backend: Arc<WslBackend> }
pub struct PodmanOps { backend: Arc<PodmanBackend> }
```

多数方法是薄包装：
- `status()` → 查 `limactl list --json` / `wsl -l -v` / `podman ps`
- `stats()` → 调 `backend.stats()` 直接返回
- `add/remove_port()` → 读 `list_ports()` → 合成新列表 → 调 `backend.edit_port_forwards()`

真正的新代码：`list_ports()`、`doctor()`、`repair()`。

### 5.3 doctor 的 issue 分类

统一的 `DoctorIssue` 结构：

```rust
pub struct DoctorIssue {
    pub id: String,              // 如 "vm-not-running"
    pub severity: Severity,      // Info / Warning / Error
    pub message: String,
    pub repair_hint: Option<String>,
    pub auto_repairable: bool,
}
```

阶段 A 实现的 issue（所有后端共用）：
- `vm-not-running` —— VM 进程不存在
- `vm-stopped` —— VM 存在但停了
- `port-conflict` —— 某 host 端口被其他进程占（用 `lsof -i` / `netstat -ano`）
- `dns-broken` —— `backend.exec("nslookup github.com")` 失败
- `disk-low` —— `df -m` 显示空闲 < 500MB
- `memory-oversubscribed` —— 配置内存 > 宿主可用

Lima 专属：`cgroup-v2-not-unified`（查 `/etc/conf.d/cgroups`）
WSL 专属：`wsl-version-too-old`、`portproxy-stale`
Podman 专属：`userns-keep-id-missing`、`volume-z-missing`

### 5.4 端口规则统一表示

```rust
pub struct PortRule {
    pub host: u16,
    pub guest: u16,
    pub backend_native_id: Option<String>,  // Lima 的 yaml entry index 等
}
```

三个 backend 各自把 native 格式映射到统一的 PortRule 列表：
- Lima：读 `~/.lima/<inst>/lima.yaml` 的 `portForwards` 数组
- WSL2：执行 `netsh interface portproxy show v4tov4` 解析
- Podman：执行 `podman port <container>` 解析（或从 inspect JSON）

---

## 6. NativeOps 层

### 6.1 trait

```rust
#[async_trait]
pub trait NativeOps: Send + Sync {
    // 只读（阶段 A 完整实现）
    async fn status(&self) -> Result<NativeStatus, OpsError>;
    async fn list_components(&self) -> Result<Vec<Component>, OpsError>;
    async fn doctor(&self) -> Result<NativeDoctorReport, OpsError>;

    // 写（阶段 A 返回 Unsupported，阶段 C 接通）
    async fn repair(&self, issue_ids: &[String], progress: ProgressSink)
        -> Result<(), OpsError>;
    async fn upgrade_node(&self, target: VersionSpec, progress: ProgressSink)
        -> Result<(), OpsError>;
    async fn upgrade_git(&self, target: VersionSpec, progress: ProgressSink)
        -> Result<(), OpsError>;
    async fn reinstall_component(&self, name: &str, progress: ProgressSink)
        -> Result<(), OpsError>;
}
```

### 6.2 types

```rust
pub struct NativeStatus {
    pub clawenv_home: PathBuf,     // ~/.clawenv
    pub home_exists: bool,
    pub node: Option<ComponentInfo>,
    pub git: Option<ComponentInfo>,
    pub total_disk_bytes: u64,
}

pub struct Component {
    pub name: String,
    pub version: Option<String>,
    pub path: Option<PathBuf>,
    pub healthy: bool,
    pub size_bytes: u64,
}

pub struct ComponentInfo {
    pub version: String,
    pub path: PathBuf,
    pub healthy: bool,
}

pub enum VersionSpec {
    Latest,
    Exact(String),
}
```

### 6.3 实现

`DefaultNativeOps` 调用 `LocalProcessRunner` 跑 `~/.clawenv/node/bin/node --version`
（或 Windows `node.exe --version`），返回结构化结果。`doctor()` 组合多个检查。

阶段 A 使用 `clawenv_core` 里已经 pub 的 `clawenv_node_dir()`、`clawenv_git_dir()`、
`has_node()`、`has_git()` 等函数做路径计算和存在性检查，**纯只读**。

---

## 7. DownloadOps 层

### 7.1 trait

```rust
#[async_trait]
pub trait DownloadOps: Send + Sync {
    // 目录
    fn catalog(&self) -> &DownloadCatalog;
    fn list_artifacts(&self) -> Vec<&ArtifactSpec>;
    fn find(&self, name: &str, version: Option<&str>) -> Option<&ArtifactSpec>;

    // 缓存
    fn cache_root(&self) -> &Path;
    async fn list_cached(&self) -> Result<Vec<CachedItem>, OpsError>;
    async fn verify_cached(&self, item: &CachedItem) -> Result<bool, OpsError>;
    async fn prune_cache(&self, keep_per_artifact: usize)
        -> Result<PruneReport, OpsError>;

    // 下载
    async fn fetch(
        &self, name: &str, version: Option<&str>,
        progress: ProgressSink, cancel: CancellationToken,
    ) -> Result<PathBuf, OpsError>;
    async fn fetch_to(
        &self, name: &str, version: Option<&str>, dest: &Path,
        progress: ProgressSink, cancel: CancellationToken,
    ) -> Result<FetchReport, OpsError>;

    // 诊断
    async fn doctor(&self) -> Result<DownloadDoctorReport, OpsError>;
    async fn check_connectivity(&self) -> Result<ConnectivityReport, OpsError>;
}
```

### 7.2 catalog

`v2/assets/download-catalog.toml`（嵌入到二进制）：

```toml
[[artifact]]
name = "node"
version = "22.12.0"
os = "macos"
arch = "arm64"
url = "https://nodejs.org/dist/v22.12.0/node-v22.12.0-darwin-arm64.tar.gz"
sha256 = "..."
kind = "tarball"
size_hint = 42000000

[[artifact]]
name = "node"
version = "22.12.0"
os = "linux"
arch = "x86_64"
url = "..."
sha256 = "..."
kind = "tarball"
```

阶段 A 只放 2–3 个 PoC 条目，阶段 C 再把 v1 散落的 URL 表搬过来。

### 7.3 fetch 实现

自包含：`CatalogBackedDownloadOps` 内部直接用 `reqwest`，复刻 v1
`download_with_progress` 的三重死亡检测（归功于 v1 的成熟设计，直接移植算法）：

- CONNECT_TIMEOUT = 15s
- CHUNK_STALL = 60s
- MIN_BYTES_BY_DEADLINE = 256 KB in 30s

SHA256 校验、指数退避重试（最多 2 次）完全一致。

### 7.4 缓存布局

```
~/.clawenv/cache/artifacts/
├── node/
│   ├── 22.12.0-macos-arm64.tar.gz
│   ├── 22.12.0-macos-arm64.tar.gz.sha256   (缓存的校验和，用于快速验证)
│   └── 22.11.0-macos-arm64.tar.gz
├── git/
└── lima/
```

`fetch` 命中缓存时只校验 sha 再返回路径，不走网络。

### 7.5 connectivity 诊断

`check_connectivity()` 对每个 artifact 的 URL 做 HEAD 请求，报告：
- 每个 URL 的 HTTP 状态码 / TLS 握手耗时 / 是否需要代理
- 宿主机的 `HTTP_PROXY` / `HTTPS_PROXY` env 是否设置
- GitHub / npmjs / nodejs.org 三个核心域名的 DNS 解析

---

## 8. CLI 层（clawops）

### 8.1 命令树

```
clawops [--json] [--quiet] [--instance NAME]
├── claw
│   ├── list                              # ClawRegistry::all()
│   ├── update <claw>     [--yes --json --channel CH --tag T --dry-run]
│   ├── doctor <claw>     [--fix --json]
│   ├── config <claw> get <key>
│   ├── config <claw> set <key> <value>
│   ├── config <claw> list
│   ├── logs <claw>       [--tail N --follow --level L]
│   ├── status <claw>
│   └── version <claw>
├── sandbox
│   ├── status            [--backend lima|wsl|podman]
│   ├── start | stop | restart
│   ├── port list
│   ├── port add <host> <guest>
│   ├── port remove <host>
│   ├── doctor            [--json]
│   ├── repair            <issue-id>... [--auto]
│   ├── stats
│   └── logs              [--tail N]
├── native
│   ├── status
│   ├── components
│   ├── doctor            [--json]
│   ├── repair            <issue-id>...     # 阶段 A: NotImplemented
│   └── upgrade node | git [--to VERSION]   # 阶段 A: NotImplemented
├── download
│   ├── list              [--os X --arch Y]
│   ├── cache list
│   ├── cache prune       [--keep N]
│   ├── cache verify
│   ├── fetch <name>      [--version V --to PATH]
│   ├── check-connectivity
│   └── doctor
└── instance
    ├── list
    └── health            [--instance NAME]   # 组合 claw+sandbox+native+download
```

### 8.2 全局 flags

- `--json` —— 所有输出换成结构化 JSON（机器可读，供 UI / 脚本消费）
- `--quiet` —— 只打错误
- `--instance NAME` —— 选择 v1 config.toml 里的实例（查 v1 的 config 模块）

### 8.3 子命令约定

每个子命令的 handler：
1. 解析本地参数
2. 构造对应的 Ops 对象（CLI 初始化代码专门干这个）
3. 调用 Ops 方法，收集结果
4. 根据 `--json` 格式化输出

---

## 9. 测试策略

### 9.1 分层

| 层级 | 位置 | 依赖 | 速度 |
|---|---|---|---|
| 单元测试 | 各模块 `#[cfg(test)] mod tests` | 无外部 | 毫秒 |
| 集成测试 | `v2/core/tests/*.rs` | LocalProcessRunner + fixture 脚本 + 本地 HTTP server | 秒 |
| CLI 测试 | `v2/cli/tests/*.rs` | `assert_cmd` 直接 spawn `clawops` 二进制 | 秒 |
| E2E | （未来）`v2/tests/e2e/` | 真 VM + 真 Hermes/OpenClaw | 分钟（CI nightly） |

### 9.2 单元测试覆盖

- Common 层：CommandSpec 构造、CancellationToken 行为、OpsError 分类
- ClawOps：每个方法的 CommandSpec 生成（~30 tests）
- SandboxOps：每个 backend 的 status/port 解析（mock stdout）
- NativeOps：各种损坏场景（tempfile 模拟 `~/.clawenv/`）
- DownloadOps：catalog TOML 解析、缓存扫描、URL-to-path 映射

### 9.3 集成测试

- `runner_local.rs` —— 端到端跑 fake claw shell 脚本，验证 timeout/cancel/stream/stdin/JSON
- `download_fixture_server.rs` —— 起 axum HTTP server 提供 tarball fixture，
  验证 fetch + checksum + stall detection + retry
- `sandbox_ops_mock.rs` —— 提供 MockSandboxBackend，验证 SandboxOps 各方法正确调用

### 9.4 CLI 测试

- `cli_smoke.rs` —— 对每个子命令跑一次 `--help`，验证不 panic
- `cli_json_output.rs` —— 验证 `--json` 输出是合法 JSON 且结构稳定

### 9.5 覆盖率目标

- Common 层：≥ 95%（它是基础设施）
- Ops 层：≥ 80%（核心业务）
- CLI 层：≥ 70%（组合层，集成测试为主）

---

## 10. 依赖策略

### 10.1 必装 crate

与 v1 保持一致，不引入新哲学：
- `tokio`（process, sync, time, io-util）
- `async-trait`
- `serde` / `serde_json` / `toml`
- `reqwest`（rustls-tls）
- `sha2` / `hex`
- `anyhow` / `thiserror`
- `clap`（v4, derive）—— CLI 专属
- `chrono`
- `tracing` / `tracing-subscriber`

### 10.2 dev 依赖

- `mockall`
- `tempfile`
- `axum`（本地 HTTP fixture server）
- `assert_cmd` / `predicates`（CLI 集成测试）

### 10.3 对 v1 的依赖

`v2/core/Cargo.toml` 加：
```toml
clawenv-core = { path = "../../core" }
```

只在 `v2/core/src/adapters/` 使用。用法：
- `clawenv_core::sandbox::{SandboxBackend, LimaBackend, WslBackend, PodmanBackend}`
- 不再引入任何其他 v1 模块。

---

## 11. 目录结构

```
v2/
├── Cargo.toml                           # [workspace] members=["core","cli"]
├── docs/
│   └── DESIGN.md                        # 本文件
├── assets/
│   └── download-catalog.toml            # DownloadOps PoC 条目
├── core/
│   ├── Cargo.toml                       # clawops-core
│   └── src/
│       ├── lib.rs
│       ├── common/
│       │   ├── mod.rs
│       │   ├── cancel.rs
│       │   ├── command.rs
│       │   ├── error.rs
│       │   ├── event.rs
│       │   ├── progress.rs
│       │   └── runner.rs
│       ├── runners/
│       │   ├── mod.rs
│       │   └── local.rs
│       ├── adapters/
│       │   ├── mod.rs
│       │   └── sandbox_backend.rs       # SandboxBackendRunner
│       ├── claw_ops/
│       │   ├── mod.rs
│       │   ├── claw_cli.rs              # trait + Opts
│       │   ├── hermes.rs
│       │   ├── openclaw.rs
│       │   └── registry.rs
│       ├── sandbox_ops/
│       │   ├── mod.rs
│       │   ├── types.rs
│       │   ├── ops.rs                   # trait
│       │   ├── lima.rs
│       │   ├── wsl.rs
│       │   └── podman.rs
│       ├── native_ops/
│       │   ├── mod.rs
│       │   ├── types.rs
│       │   ├── ops.rs
│       │   └── default.rs
│       └── download_ops/
│           ├── mod.rs
│           ├── types.rs
│           ├── catalog.rs
│           ├── ops.rs
│           └── default.rs
├── cli/
│   ├── Cargo.toml                       # clawops (binary)
│   └── src/
│       ├── main.rs
│       ├── cmd/
│       │   ├── mod.rs
│       │   ├── claw.rs
│       │   ├── sandbox.rs
│       │   ├── native.rs
│       │   ├── download.rs
│       │   └── instance.rs
│       └── shared.rs                    # JSON output helper, progress printer
└── tests/                               # workspace-level integration (optional)
```

---

## 12. 分阶段交付

### 阶段 A（本次交付）

- [x] 设计文档（本文件）
- [ ] v2 workspace 骨架
- [ ] common 层 + LocalProcessRunner 完整实现 + 测试
- [ ] ClawOps 完整（Hermes + OpenClaw）+ 测试
- [ ] SandboxOps trait + 三后端只读 impl + doctor PoC（~3 个 issue）
- [ ] NativeOps trait + 只读 impl（status/components/doctor）
- [ ] DownloadOps trait + catalog TOML + fetch + 缓存 + 测试
- [ ] clawops CLI 五个顶层子命令树 + `--help` / `--json` 支持
- [ ] 全套单元 + 集成测试，`cargo test` 全绿

### 阶段 B（后续）

- SandboxOps 写操作（repair 具体实现）
- NativeOps 写操作（repair / upgrade_node / upgrade_git）
- Catalog 扩展：把 v1 散落的 URL 表搬进来
- CLI 输出美化（表格、色彩）

### 阶段 C（后续）

- v1 业务层（manager/upgrade.rs 等）改为调用 v2 的 Ops
- Tauri IPC 替换为 spawn clawops 子进程 + `--json`

### 阶段 D（长期）

- v1 核心业务层废弃，v2 成为主干

---

## 13. 验收标准

- `cd v2 && cargo build` 零错误
- `cd v2 && cargo test` 全绿
- `cd v2 && cargo clippy -- -D warnings` 无告警
- `cd v2/cli && cargo run -- --help` 输出五个子命令
- `cd v2/cli && cargo run -- claw list` 列出 hermes + openclaw
- `cd v2/cli && cargo run -- download list` 列出 catalog 条目
- `cd v2/cli && cargo run -- native status` 返回当前宿主机 node/git 状态
- `cd v2/cli && cargo run -- sandbox status` 返回当前后端 VM 状态
- `cd v2/cli && cargo run -- native status --json` 输出合法 JSON
- 在根目录 `cargo build --workspace` 仍然只编译 v1（v2 未被拉入）
- `git status` 只看到 `v2/` 下的新增文件，v1 零改动
