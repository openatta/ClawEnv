mod lima;
mod native;
mod wsl;
mod podman;
mod exec_helper;
#[cfg(test)]
pub mod mock;

pub use lima::LimaBackend;
pub use native::NativeBackend;
pub use wsl::WslBackend;
pub use podman::PodmanBackend;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::platform::{OsType, detect_platform};

/// 沙盒后端抽象——三个平台的对等接口
/// WSL2 / Lima / Podman 各自实现此 trait，没有层级关系
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// 后端名称，用于日志与展示
    fn name(&self) -> &str;

    /// 检测此后端在当前系统是否可用
    async fn is_available(&self) -> Result<bool>;

    /// 检测并安装前提条件（WSL2 / limactl / podman）
    async fn ensure_prerequisites(&self) -> Result<()>;

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

    /// 在沙盒内以流式方式执行命令（stderr 逐行发送到 channel）
    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String>;

    /// 安装系统包（平台抽象：Lima=sudo apk add, WSL2=apk add, Podman=podman exec apk add）
    /// `packages` comes from trusted TOML descriptors. Validated to prevent injection.
    async fn install_package(&self, packages: &str) -> Result<()> {
        // Validate: package names must be alphanumeric, hyphens, dots, spaces only
        if !packages.chars().all(|c| c.is_ascii_alphanumeric() || "-_. ".contains(c)) {
            anyhow::bail!("Invalid package names: {packages}");
        }
        self.exec(&format!("sudo apk add --no-cache {packages} 2>&1 || apk add --no-cache {packages} 2>&1")).await?;
        Ok(())
    }

    /// 安装 npm 全局包
    /// `package` comes from trusted TOML descriptors. Validated to prevent injection.
    async fn npm_install_global(&self, package: &str, tx: &mpsc::Sender<String>) -> Result<()> {
        // Validate: npm package@version must be alphanumeric, hyphens, dots, @, / only
        if !package.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@/".contains(c)) {
            anyhow::bail!("Invalid npm package name: {package}");
        }
        self.exec_with_progress(
            &format!("sudo npm install -g {package} 2>&1 || npm install -g {package} 2>&1"),
            tx,
        ).await?;
        Ok(())
    }

    /// 获取资源使用情况
    async fn stats(&self) -> Result<ResourceStats>;

    /// 导入预构建镜像
    async fn import_image(&self, path: &std::path::Path) -> Result<()>;

    // ---- Optional management operations (default: not supported) ----

    /// Rename the sandbox instance. Returns new sandbox_id.
    async fn rename(&self, _new_name: &str) -> Result<String> {
        anyhow::bail!("Rename not supported by this backend")
    }

    /// Edit resource limits (CPU cores, memory MB, disk GB). Requires restart.
    async fn edit_resources(&self, _cpus: Option<u32>, _memory_mb: Option<u32>, _disk_gb: Option<u32>) -> Result<()> {
        anyhow::bail!("Resource editing not supported by this backend")
    }

    /// Edit port forwarding rules. Requires restart.
    async fn edit_port_forwards(&self, _forwards: &[(u16, u16)]) -> Result<()> {
        anyhow::bail!("Port forward editing not supported by this backend")
    }

    /// Capability flags
    fn supports_rename(&self) -> bool { false }
    fn supports_resource_edit(&self) -> bool { false }
    fn supports_port_edit(&self) -> bool { false }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxOpts {
    pub instance_name: String,
    /// Claw type ID (e.g., "openclaw", "zeroclaw") — used to look up the ClawDescriptor
    #[serde(default = "default_claw_type")]
    pub claw_type: String,
    pub claw_version: String,
    pub alpine_version: String,
    pub memory_mb: u32,
    pub cpu_cores: u32,
    pub install_browser: bool,
    pub install_mode: InstallMode,
    /// Proxy env lines for provision script (empty string if no proxy)
    #[serde(default)]
    pub proxy_script: String,
    /// Gateway port (for Lima portForwards)
    #[serde(default = "default_gateway_port")]
    pub gateway_port: u16,
    /// Custom Alpine mirror base URL (empty = default CDN)
    #[serde(default)]
    pub alpine_mirror: String,
    /// Custom npm registry URL (empty = default npmjs.org)
    #[serde(default)]
    pub npm_registry: String,
}

fn default_claw_type() -> String { "openclaw".into() }
fn default_gateway_port() -> u16 { 3000 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstallMode {
    /// 在线构建：下载 Alpine base + 逐步安装
    OnlineBuild,
    /// 预构建镜像：下载或本地导入（沙盒用）
    PrebuiltImage { source: ImageSource },
    /// Native 离线安装包：内含 Node.js + node_modules，解压即用
    NativeBundle { path: std::path::PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageSource {
    /// 从 GitHub Releases 下载
    Remote { url: String, checksum_sha256: String },
    /// 从本地文件导入
    LocalFile { path: std::path::PathBuf },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceStats {
    pub cpu_percent: f32,
    pub memory_used_mb: u64,
    pub memory_limit_mb: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxType {
    Wsl2Alpine,
    LimaAlpine,
    PodmanAlpine,
    Native,
}

impl SandboxType {
    /// User-friendly display name (platform-neutral).
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Wsl2Alpine => "Sandbox (WSL2)",
            Self::LimaAlpine => "Sandbox (Lima)",
            Self::PodmanAlpine => "Sandbox (Podman)",
            Self::Native => "Native",
        }
    }

    pub fn from_os() -> Self {
        match detect_platform().map(|p| p.os) {
            Ok(OsType::Windows) => Self::Wsl2Alpine,
            Ok(OsType::Macos) => Self::LimaAlpine,
            Ok(OsType::Linux) | Err(_) => Self::PodmanAlpine,
        }
    }
}

/// 工厂函数：根据当前平台自动选择后端（默认实例名 "default"）
pub fn detect_backend() -> Result<Box<dyn SandboxBackend>> {
    detect_backend_for("default")
}

/// 工厂函数：根据当前平台自动选择后端，使用指定实例名
pub fn detect_backend_for(instance_name: &str) -> Result<Box<dyn SandboxBackend>> {
    let platform = detect_platform()?;
    match platform.os {
        OsType::Windows => Ok(Box::new(WslBackend::new(instance_name))),
        OsType::Macos => Ok(Box::new(LimaBackend::new(instance_name))),
        OsType::Linux => Ok(Box::new(PodmanBackend::with_defaults(instance_name))),
    }
}

/// 创建 native 模式后端（开发者专用）
pub fn native_backend(instance_name: &str) -> NativeBackend {
    NativeBackend::new(instance_name)
}
