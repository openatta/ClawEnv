# 4. 平台沙盒实现详述

## 4.1 Windows → WSL2 + Alpine Linux

**机制说明**：WSL2 是微软提供的轻量级 Hyper-V 虚拟机，运行完整 Linux 内核。
ClawEnv 向其中导入 Alpine Linux 作为专用 distro，OpenClaw 直接运行在该 Alpine 环境内。
**不在 Alpine 内再运行 Podman 或任何容器引擎。**

### 前提检测

```rust
pub struct WslBackend {
    distro_name: String,  // "ClawEnv-Alpine"
}

impl WslBackend {
    pub async fn is_available() -> Result<bool> {
        // 检测 WSL2 版本与 Windows 版本（需 1903+ build 18362+）
        let out = Command::new("wsl").args(["--status"]).output().await?;
        Ok(out.status.success())
    }

    async fn ensure_wsl2(&self, progress: &dyn ProgressReporter) -> Result<()> {
        if !Self::is_available().await? {
            progress.report("正在启用 WSL2，完成后需要重启...", 10);
            // 提示用户以管理员权限运行：wsl --install --no-distribution
            return Err(anyhow!("需要重启以完成 WSL2 安装，请重启后重新运行 ClawEnv"));
        }
        Ok(())
    }
}
```

### Alpine 导入流程

```rust
async fn create(&self, opts: &SandboxOpts) -> Result<()> {
    // 1. 下载 Alpine Linux minirootfs
    //    来源: https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/x86_64/
    let rootfs = download_alpine_rootfs(&opts.alpine_version).await?;

    // 2. 导入为 WSL2 distro（一次性操作，幂等）
    //    wsl --import ClawEnv-Alpine <install_path> <rootfs.tar.gz> --version 2
    Command::new("wsl")
        .args([
            "--import", &self.distro_name,
            &opts.install_path.to_string_lossy(),
            &rootfs.to_string_lossy(),
            "--version", "2",
        ])
        .status().await?;

    // 3. 初始化 Alpine 环境（安装 Node.js + OpenClaw）
    self.exec("apk update && apk add --no-cache nodejs npm git curl").await?;
    // 可选：安装浏览器（用户在安装向导中选择启用时）
    // self.exec("apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont").await?;
    self.exec(&format!("npm install -g openclaw@{}", opts.claw_version)).await?;

    // 4. 配置 WSL2 资源限制
    //    写入 %USERPROFILE%\.wslconfig
    write_wslconfig(opts.memory_mb, opts.cpu_cores).await?;

    Ok(())
}
```

### WSL2 资源配置（`%USERPROFILE%\.wslconfig`）

```ini
[wsl2]
memory=512MB
processors=2
swap=256MB
# 防止 WSL2 无限制占用宿主机内存
```

### 命令执行接口

```rust
#[async_trait]
impl SandboxBackend for WslBackend {
    fn name(&self) -> &str { "WSL2 + Alpine Linux" }

    async fn exec(&self, cmd: &str) -> Result<String> {
        let out = Command::new("wsl")
            .args(["-d", &self.distro_name, "--", "ash", "-c", cmd])
            .output().await?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn snapshot_create(&self, tag: &str) -> Result<()> {
        // WSL2 快照通过导出 distro tarball 实现
        let snapshot_path = snapshot_dir().join(format!("{}.tar.gz", tag));
        Command::new("wsl")
            .args(["--export", &self.distro_name,
                   &snapshot_path.to_string_lossy()])
            .status().await?;
        Ok(())
    }

    async fn snapshot_restore(&self, tag: &str) -> Result<()> {
        let snapshot_path = snapshot_dir().join(format!("{}.tar.gz", tag));
        // 先注销当前 distro，再从快照重新导入
        Command::new("wsl").args(["--unregister", &self.distro_name]).status().await?;
        Command::new("wsl")
            .args(["--import", &self.distro_name,
                   &install_path().to_string_lossy(),
                   &snapshot_path.to_string_lossy(),
                   "--version", "2"])
            .status().await?;
        Ok(())
    }
}
```

---

## 4.2 macOS → Lima + Alpine Linux

**机制说明**：Lima 是 macOS 上的轻量级 Linux VM 管理工具，底层使用 Apple Virtualization Framework（M 系列）
或 QEMU（Intel）。ClawEnv 通过 Lima 启动一个 Alpine Linux VM，OpenClaw 直接在该 VM 内运行。
**不在 VM 内再运行 Podman 或任何容器引擎。**

> **注意**：Lima + Alpine 有一个已知的 cgroup v2 兼容性问题（alpine-lima issue #1878），
> 默认 Alpine 以 hybrid 模式启动 cgroup，需要手动切换为 unified 模式才能支持
> 完整的进程资源控制。ClawEnv 的模板已内置此 fix。

### Lima VM 模板（`assets/lima/clawenv-alpine.yaml`）

```yaml
# ClawEnv Lima Template — Alpine Linux
# 已内置 cgroup v2 unified 模式 fix（解决 alpine-lima issue #1878）

vmType: vz          # Apple Virtualization Framework（M系列优先，Intel 回退 QEMU）
os: Linux
arch: host          # 自动匹配宿主机架构（aarch64 / x86_64）

images:
  - location: "https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/aarch64/alpine-virt-{VERSION}-aarch64.iso"
    arch: aarch64
  - location: "https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/x86_64/alpine-virt-{VERSION}-x86_64.iso"
    arch: x86_64

cpus: 2
memory: "512MiB"
disk: "8GiB"

mounts:
  - location: "~/.clawenv/workspaces/{INSTANCE_NAME}"
    writable: true
    mountPoint: "/workspace"
  # 主目录只读挂载，避免 VM 内意外修改宿主文件
  - location: "~"
    writable: false
    mountPoint: "/host-home"

hostResolver:
  enabled: true

# 初始化脚本：修复 cgroup v2 + 安装 OpenClaw
provision:
  - mode: system
    script: |
      #!/bin/ash
      set -e

      # === Fix: cgroup v2 unified 模式 ===
      # 解决 alpine-lima 默认 hybrid 模式导致的资源控制问题
      sed -i 's/rc_cgroup_mode=.*/rc_cgroup_mode=unified/' /etc/conf.d/cgroups
      rc-update add cgroups boot

      # === 安装运行时依赖 ===
      apk update
      apk add --no-cache nodejs npm git curl bash ca-certificates

      # === 可选：安装浏览器（用户在安装向导中选择启用时） ===
      # apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont

      # === 安装 OpenClaw ===
      npm install -g openclaw@{OPENCLAW_VERSION}

      # === 验证安装 ===
      openclaw --version

      echo "ClawEnv Alpine VM 初始化完成"
```

### Lima 管理接口

```rust
pub struct LimaBackend {
    vm_name: String,         // "clawenv-{instance_name}"
    template_path: PathBuf,  // ~/.clawenv/templates/clawenv-alpine.yaml
}

impl LimaBackend {
    pub async fn is_available() -> Result<bool> {
        Command::new("limactl").args(["--version"])
            .output().await
            .map(|o| o.status.success())
            .unwrap_or(false)
            .pipe(Ok)
    }

    async fn install_lima() -> Result<()> {
        // 通过 Homebrew 安装（macOS 标准方式）
        Command::new("brew").args(["install", "lima"]).status().await?;
        Ok(())
    }
}

#[async_trait]
impl SandboxBackend for LimaBackend {
    fn name(&self) -> &str { "Lima + Alpine Linux" }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        if !Self::is_available().await? {
            Self::install_lima().await?;
        }
        // 渲染模板（填充版本号、实例名等占位符）
        let rendered = render_template(&self.template_path, opts)?;
        let rendered_path = write_rendered_template(&rendered).await?;

        Command::new("limactl")
            .args(["start", "--name", &self.vm_name,
                   &rendered_path.to_string_lossy()])
            .status().await?;
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        let out = Command::new("limactl")
            .args(["shell", &self.vm_name, "--", "ash", "-c", cmd])
            .output().await?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn snapshot_create(&self, tag: &str) -> Result<()> {
        Command::new("limactl")
            .args(["snapshot", "create", &self.vm_name, "--tag", tag])
            .status().await?;
        Ok(())
    }

    async fn snapshot_restore(&self, tag: &str) -> Result<()> {
        Command::new("limactl")
            .args(["snapshot", "apply", &self.vm_name, "--tag", tag])
            .status().await?;
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        Command::new("limactl").args(["start", &self.vm_name]).status().await?;
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Command::new("limactl").args(["stop", &self.vm_name]).status().await?;
        Ok(())
    }
}
```

---

## 4.3 Linux → Podman + Alpine Linux

**机制说明**：Linux 主机已有内核，不需要再虚拟化一个内核。Podman 以容器方式运行 Alpine Linux，
OpenClaw 直接在容器内运行。Podman 选择理由：无守护进程、原生 rootless、OCI 兼容。
**这里的 Podman 是沙盒本身，与 WSL2/Lima 处于同一层级，不是在另一个沙盒内运行。**

### Alpine 容器定义（`assets/podman/Containerfile`）

```dockerfile
# ClawEnv — OpenClaw Alpine 容器
# 基础镜像: Alpine Linux（~5MB，极度精简）
FROM alpine:latest

LABEL maintainer="ClawEnv"
LABEL description="OpenClaw AI Agent — Sandboxed Alpine Linux Container"

# 安装运行时依赖（无缓存，保持镜像最小）
RUN apk update && \
    apk add --no-cache \
      nodejs \
      npm \
      git \
      curl \
      bash \
      ca-certificates && \
    rm -rf /var/cache/apk/*

# 可选：安装浏览器 + 人工介入组件（通过 ARG 控制）
ARG INSTALL_BROWSER=false
RUN if [ "$INSTALL_BROWSER" = "true" ]; then \
      apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont; \
    fi

# 安装 OpenClaw（版本通过 ARG 注入，确保可重现性）
ARG OPENCLAW_VERSION=latest
RUN npm install -g openclaw@${OPENCLAW_VERSION} && \
    openclaw --version

# 创建非特权用户（安全原则：不以 root 运行）
RUN adduser -D -s /bin/ash clawuser && \
    mkdir -p /home/clawuser/.openclaw && \
    chown -R clawuser:clawuser /home/clawuser

USER clawuser
WORKDIR /home/clawuser

# 工作目录与配置目录挂载点
VOLUME ["/workspace", "/home/clawuser/.openclaw"]

EXPOSE 3000

CMD ["openclaw", "start"]
```

### Podman 管理接口

```rust
pub struct PodmanBackend {
    container_name: String,   // "clawenv-{instance_name}"
    image_tag: String,        // "clawenv/openclaw:{version}"
}

impl PodmanBackend {
    pub async fn is_available() -> Result<bool> {
        Command::new("podman").args(["--version"])
            .output().await
            .map(|o| o.status.success())
            .unwrap_or(false)
            .pipe(Ok)
    }
}

#[async_trait]
impl SandboxBackend for PodmanBackend {
    fn name(&self) -> &str { "Podman + Alpine Linux" }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        // 构建镜像
        Command::new("podman")
            .args([
                "build",
                "--build-arg", &format!("OPENCLAW_VERSION={}", opts.claw_version),
                "-t", &self.image_tag,
                "-f", &containerfile_path().to_string_lossy(),
                ".",
            ])
            .status().await?;
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        Command::new("podman")
            .args([
                "run", "-d",
                "--name", &self.container_name,
                "--userns=keep-id",   // rootless：容器内 UID 映射到宿主机当前用户
                "-v", &format!("{}:/workspace:Z",
                    workspace_path(&self.container_name).display()),
                "-v", &format!("{}:/home/clawuser/.openclaw:Z",
                    openclaw_config_path().display()),
                "-p", "127.0.0.1:3000:3000",  // 只绑定本地，不暴露外网
                "--restart", "unless-stopped",
                &self.image_tag,
            ])
            .status().await?;
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        let out = Command::new("podman")
            .args(["exec", &self.container_name, "ash", "-c", cmd])
            .output().await?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn stop(&self) -> Result<()> {
        Command::new("podman").args(["stop", &self.container_name]).status().await?;
        Ok(())
    }

    async fn snapshot_create(&self, tag: &str) -> Result<()> {
        // Podman 快照通过 commit 容器为镜像实现
        Command::new("podman")
            .args(["commit", &self.container_name,
                   &format!("{}:snap-{}", self.image_tag, tag)])
            .status().await?;
        Ok(())
    }

    async fn snapshot_restore(&self, tag: &str) -> Result<()> {
        // 停止并删除当前容器，从快照镜像重新启动
        self.stop().await.ok();
        Command::new("podman").args(["rm", &self.container_name]).status().await?;
        // 临时修改 image_tag 为快照标签后重新 start
        let snapshot_image = format!("{}:snap-{}", self.image_tag, tag);
        Command::new("podman")
            .args(["run", "-d", "--name", &self.container_name,
                   "--userns=keep-id", &snapshot_image])
            .status().await?;
        Ok(())
    }
}
```

---

## 4.4 浏览器集成（沙盒内 Chromium）

### 设计决策

浏览器（Chromium headless）安装在沙盒内部，与 OpenClaw 同处一个隔离环境。
不使用宿主机浏览器，避免打穿沙盒安全边界。

**为什么选 Chromium**：
- OpenClaw 默认使用 CDP 协议，Chromium 原生支持，零适配成本
- Puppeteer / Playwright 一等公民支持
- Alpine 官方仓库提供 `chromium` 包，维护稳定

**体积影响**：

| 组件 | 体积 | 是否必装 |
|------|------|---------|
| Alpine + Node.js + OpenClaw | ~150MB | 是 |
| Chromium headless | +100-120MB | 可选 |
| Xvfb + VNC + noVNC（人工介入） | +5-8MB | 随 Chromium 一起装 |
| **总计（含浏览器）** | **~260-280MB** | |

浏览器为可选组件，在安装向导中由用户选择是否启用。

### 沙盒内浏览器安装

```sh
# 在 Alpine 沙盒内安装 Chromium + 人工介入组件
apk add --no-cache \
    chromium \
    xvfb-run \
    x11vnc \
    novnc \
    websockify \
    ttf-freefont    # 中文字体支持
```

### 人工介入：noVNC 方案

当 OpenClaw 遇到需要人工操作的场景（登录、CAPTCHA、OAuth 授权等），
通过 noVNC 将沙盒内浏览器画面安全地转发到 ClawEnv 主界面。

**安全特性**：浏览器始终在沙盒内运行，只有画面像素流通过 WebSocket 传出，
宿主机上的 cookie、密码管理器、文件系统均不受影响。

**架构**：

```
沙盒内部                                    宿主机
┌────────────────────────────┐          ┌─────────────┐
│  Xvfb (:99)               │          │  ClawEnv    │
│    └── Chromium            │  画面流   │  Tauri App  │
│         (有头模式)          │◄────────►│             │
│  x11vnc → websockify:6080  │ WebSocket│  noVNC 面板  │
└────────────────────────────┘          └─────────────┘
```

**工作流程**：

1. 正常运行时：Chromium 以 headless 模式执行自动化任务
2. OpenClaw 检测到需要人工（登录页/CAPTCHA）→ 发出 `human-intervention-needed` 事件
3. ClawEnv 启动 Xvfb + x11vnc + websockify（如未运行），Chromium 切换到有头模式
4. System Tray 通知用户："需要您手动操作"
5. 用户点击 → ClawEnv 在 OpenClaw 页中打开 noVNC 面板
6. 用户在 noVNC 中完成操作（输密码/点验证码）
7. 完成后点击"返回 Agent 视图"
8. Chromium 切回 headless，继续自动执行

**noVNC 启动脚本**（沙盒内）：

```sh
#!/bin/ash
# clawenv-vnc-start.sh — 人工介入时启动 VNC 链路
set -e

export DISPLAY=:99

# 启动虚拟帧缓冲（如未运行）
if ! pgrep -f "Xvfb :99" > /dev/null; then
    Xvfb :99 -screen 0 1280x720x24 &
    sleep 1
fi

# 启动 VNC server（仅监听本地）
x11vnc -display :99 -nopw -listen 127.0.0.1 -port 5900 -shared -forever &

# 启动 WebSocket 代理（noVNC 前端通过此连接）
websockify --web=/usr/share/novnc 6080 127.0.0.1:5900 &

echo "noVNC ready at ws://127.0.0.1:6080"
```

### BrowserBackend Trait

```rust
/// 浏览器后端 trait（独立于 SandboxBackend）
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    /// 启动 headless 浏览器
    async fn start_headless(&self, cdp_port: u16) -> Result<()>;
    /// 切换到有头模式（人工介入）并启动 noVNC
    async fn start_interactive(&self, vnc_ws_port: u16) -> Result<String>; // 返回 noVNC URL
    /// 切回 headless 模式
    async fn resume_headless(&self) -> Result<()>;
    /// 停止浏览器
    async fn stop(&self) -> Result<()>;
    /// 获取状态
    async fn status(&self) -> Result<BrowserStatus>;
}

#[derive(Debug, Clone)]
pub enum BrowserStatus {
    Stopped,
    Headless { cdp_port: u16 },
    Interactive { novnc_url: String },
}

// Phase 1 实现
pub struct ChromiumBackend {
    sandbox: Arc<dyn SandboxBackend>,
}

// Phase 4 预留
pub struct FingerprintBrowserBackend;
impl BrowserBackend for FingerprintBrowserBackend {
    async fn start_headless(&self, _: u16) -> Result<()> {
        Err(anyhow!("指纹浏览器支持计划于 Phase 4 实现"))
    }
    // ...
}
```

---

## 4.5 Native 模式（无沙盒直装）

> **WARNING: Native 模式没有任何安全隔离。**
> OpenClaw 将以当前用户权限直接运行在宿主操作系统上，可完全访问本机文件系统、网络和进程。
> 仅供开发者调试使用，不建议普通用户启用。

**机制说明**：Native 模式跳过所有沙盒层（WSL2 / Lima / Podman），将 OpenClaw 直接安装到
本机目录 `~/.clawenv/native/{instance_name}` 下，通过宿主机的 Node.js 运行时执行。
此模式仅在 `user_mode = "developer"` 时可见。

```rust
pub struct NativeBackend {
    install_dir: PathBuf, // ~/.clawenv/native/{instance_name}
}

#[async_trait]
impl SandboxBackend for NativeBackend {
    fn name(&self) -> &str { "Native (无沙盒)" }

    async fn is_available() -> Result<bool> where Self: Sized {
        // 检测宿主机是否已安装 Node.js 和 npm
        let node = Command::new("node").args(["--version"]).output().await;
        let npm  = Command::new("npm").args(["--version"]).output().await;
        Ok(node.map(|o| o.status.success()).unwrap_or(false)
            && npm.map(|o| o.status.success()).unwrap_or(false))
    }

    async fn ensure_prerequisites(&self, _progress: &dyn ProgressReporter) -> Result<()> {
        if !Self::is_available().await? {
            return Err(anyhow!("Native 模式需要宿主机已安装 Node.js 和 npm"));
        }
        Ok(())
    }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        // 在宿主机目录下直接 npm install openclaw
        std::fs::create_dir_all(&self.install_dir)?;
        Command::new("npm")
            .args(["install", "-g", &format!("openclaw@{}", opts.claw_version)])
            .status().await?;
        Ok(())
    }

    // start / stop 对沙盒本身是空操作（无 VM/容器生命周期）
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self)  -> Result<()> { Ok(()) }

    async fn destroy(&self) -> Result<()> {
        std::fs::remove_dir_all(&self.install_dir)?;
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        // 直接在宿主机执行命令
        let out = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&self.install_dir)
            .output().await?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn snapshot_create(&self, tag: &str) -> Result<()> {
        // 快照通过 tar 打包 install_dir 实现（无 VM/容器级快照）
        let snapshot_path = snapshot_dir().join(format!("{}.tar.gz", tag));
        Command::new("tar")
            .args(["-czf", &snapshot_path.to_string_lossy(),
                   "-C", &self.install_dir.to_string_lossy(), "."])
            .status().await?;
        Ok(())
    }

    async fn snapshot_restore(&self, tag: &str) -> Result<()> {
        let snapshot_path = snapshot_dir().join(format!("{}.tar.gz", tag));
        std::fs::remove_dir_all(&self.install_dir)?;
        std::fs::create_dir_all(&self.install_dir)?;
        Command::new("tar")
            .args(["-xzf", &snapshot_path.to_string_lossy(),
                   "-C", &self.install_dir.to_string_lossy()])
            .status().await?;
        Ok(())
    }
}
```
