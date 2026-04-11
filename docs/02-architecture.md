# 2. 核心架构：三平台对等沙盒模型

## 2.1 架构概览

ClawEnv 的核心设计原则：**WSL2、Lima、Podman 是同一层级的三种对等实现，
分别对应三个操作系统的最优隔离方案，没有任何一个平台需要在沙盒内部再套另一个沙盒。**

```
┌─────────────────────────────────────────────────────────────────┐
│                         主机操作系统                             │
│                                                                 │
│   Windows 10/11           macOS 12+            Linux            │
│   ─────────────           ─────────            ─────            │
│                                                                 │
│   ┌────────────┐       ┌────────────┐       ┌────────────┐      │
│   │   WSL2     │       │    Lima    │       │   Podman   │      │
│   │  (轻量VM)  │       │  (轻量VM)  │       │  (容器)    │      │
│   │            │       │            │       │            │      │
│   │ ┌────────┐ │       │ ┌────────┐ │       │ ┌────────┐ │      │
│   │ │ Alpine │ │       │ │ Alpine │ │       │ │ Alpine │ │      │
│   │ │ Linux  │ │       │ │ Linux  │ │       │ │ Linux  │ │      │
│   │ │        │ │       │ │        │ │       │ │        │ │      │
│   │ │ Claw ☆ │ │       │ │ Claw ☆ │ │       │ │ Claw ☆ │ │      │
│   │ └────────┘ │       │ └────────┘ │       │ └────────┘ │      │
│   └────────────┘       └────────────┘       └────────────┘      │
│                                                                 │
│         ▲                     ▲                    ▲            │
│         └─────────────────────┴────────────────────┘            │
│                               │                                 │
│                    ┌──────────┴──────────┐                      │
│                    │     ClawEnv         │                      │
│                    │  (Rust + Tauri)     │                      │
│                    │                     │                      │
│                    │  GUI  ◄──IPC──► CLI │                      │
│                    └─────────────────────┘                      │
└─────────────────────────────────────────────────────────────────┘
```

## 2.2 为什么三者处于同一层级

三种机制解决的是同一个问题：**在没有原生隔离能力（或原生隔离不够安全）的情况下，
为任意 Claw 产品（OpenClaw、ZeroClaw、AutoClaw 等）提供一个独立的 Alpine Linux 运行环境**。

| | WSL2 | Lima | Podman |
|---|---|---|---|
| **本质** | 轻量级 Hyper-V VM | 轻量级 QEMU/VZ VM | Linux 容器（namespaces + cgroups） |
| **隔离级别** | VM 级 | VM 级 | 容器级 |
| **为什么在这个平台用它** | Windows 无原生 Linux 内核，需要 VM | macOS 无原生 Linux 内核，需要 VM | Linux 已有内核，容器隔离已足够 |
| **Alpine 的角色** | WSL2 内导入的 distro | Lima VM 内运行的 OS | Podman 容器的 base image |
| **Claw 产品运行位置** | Alpine distro 内 | Alpine VM 内 | Alpine 容器内 |

**Linux 选 Podman 而非 Docker 的理由**：
Podman 无守护进程（daemonless），空闲时 CPU/内存占用为零；原生 rootless，无需 root 权限；
OCI 完全兼容，Alpine 官方 wiki 有专页支持。

**WSL2 内不需要再跑 Podman**：WSL2 本身已提供 VM 级别的隔离，Alpine distro 直接在其中运行
OpenClaw 即可。在 WSL2 内再套 Podman 属于不必要的双重虚拟化，增加复杂度而无收益。

**Lima 内不需要再跑 Podman**：同理，Lima VM 本身已是隔离边界。

## 2.4 Native 模式（开发者专用）

除三种沙盒后端外，ClawEnv 还提供 **Native 模式**——直接在宿主操作系统上安装并运行 OpenClaw，
不经过任何沙盒（WSL2 / Lima / Podman）隔离层。

| 项目 | 说明 |
|---|---|
| **可见性** | 仅在 `user_mode = "developer"` 时可选，普通用户界面不展示此选项 |
| **使用场景** | 开发者需要直接访问宿主文件系统、调试 OpenClaw 源码、或本机已有 Node.js 环境希望避免虚拟化开销 |
| **代价** | 无进程/文件系统隔离、无 VM/容器级快照与回滚（仅 tar 备份）、API Key 等凭证直接存储在本地文件系统 |
| **Trait 兼容性** | `NativeBackend` 同样实现 `SandboxBackend` trait，`exec` 直接调用宿主 `Command`，`start`/`stop` 为空操作 |

> **不建议普通用户使用 Native 模式。** 沙盒隔离是 ClawEnv 安全模型的核心，
> Native 模式绕过了全部隔离机制，仅供开发与调试用途。

## 2.3 SandboxBackend Trait 设计

三个后端在 Rust 代码层面完全对等，实现同一个 trait：

```rust
/// 沙盒后端抽象——三个平台的对等接口
/// WSL2 / Lima / Podman 各自实现此 trait，没有层级关系
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// 后端名称，用于日志与展示
    fn name(&self) -> &str;

    /// 检测此后端在当前系统是否可用
    async fn is_available() -> Result<bool> where Self: Sized;

    /// 检测并安装前提条件（WSL2 / limactl / podman）
    async fn ensure_prerequisites(&self, progress: &dyn ProgressReporter) -> Result<()>;

    /// 创建并初始化沙盒（含 Alpine Linux 环境）
    async fn create(&self, opts: &SandboxOpts) -> Result<()>;

    /// 启动沙盒
    async fn start(&self) -> Result<()>;

    /// 停止沙盒
    async fn stop(&self) -> Result<()>;

    /// 销毁沙盒（不可逆）
    async fn destroy(&self) -> Result<()>;

    /// 在沙盒内执行命令，返回 stdout
    async fn exec(&self, cmd: &str) -> Result<String>;

    /// 在沙盒内以流式方式执行命令（用于安装进度等场景）
    async fn exec_stream(&self, cmd: &str, tx: mpsc::Sender<String>) -> Result<ExitStatus>;

    /// 获取资源使用情况（内存/CPU）
    async fn stats(&self) -> Result<ResourceStats>;
}

/// 工厂函数：根据当前平台自动选择后端
pub async fn detect_backend() -> Result<Box<dyn SandboxBackend>> {
    match std::env::consts::OS {
        "windows" => Ok(Box::new(WslBackend::new())),
        "macos"   => Ok(Box::new(LimaBackend::new())),
        "linux"   => Ok(Box::new(PodmanBackend::new())),
        other     => Err(anyhow!("不支持的操作系统: {}", other)),
    }
}
```
