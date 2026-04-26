mod lima;
mod native;
mod wsl;
mod podman;
mod exec_helper;
#[cfg(test)]
pub mod mock;

pub use lima::{LimaBackend, init_lima_env, lima_home, limactl_bin};
// Re-export with a stable, module-prefixed name so migration code in
// manager/instance.rs doesn't reach into `sandbox::lima::*` privates.
pub(crate) use lima::ensure_dashboard_port_forward as ensure_dashboard_port_forward_yaml;
pub use native::NativeBackend;
pub use wsl::WslBackend;
pub use podman::{PodmanBackend, init_podman_env, podman_data_home, podman_runtime_home};

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

    /// 检测并安装前提条件（WSL2 / limactl / podman）.
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
    /// Proxy URL triple for provision-time use. Lima / WSL use
    /// `proxy_script` (inline export statements); Podman builds use these
    /// separate fields passed as `--build-arg HTTP_PROXY=...`. Populated
    /// from `proxy_resolver::Scope::Installer` at install.rs entry.
    #[serde(default)]
    pub http_proxy: String,
    #[serde(default)]
    pub https_proxy: String,
    #[serde(default)]
    pub no_proxy: String,
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

    /// Stable wire-format string matching the serde `kebab-case` rename rule.
    /// Keeping this as a hand-written match (rather than going through
    /// `serde_json`) avoids a fallible serialize at the call-site — the
    /// export/import code that stamps this into the bundle manifest needs it
    /// to be infallible, and a match doesn't drift because a `#[test]` below
    /// asserts it stays in sync with the derived serde.
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            Self::Wsl2Alpine => "wsl2-alpine",
            Self::LimaAlpine => "lima-alpine",
            Self::PodmanAlpine => "podman-alpine",
            Self::Native => "native",
        }
    }

    /// Inverse of `as_wire_str`. Import side uses this to map a manifest's
    /// `sandbox_type` string back to the enum (so we can route to the right
    /// backend importer).
    pub fn parse_wire(s: &str) -> Option<Self> {
        match s {
            "wsl2-alpine" => Some(Self::Wsl2Alpine),
            "lima-alpine" => Some(Self::LimaAlpine),
            "podman-alpine" => Some(Self::PodmanAlpine),
            "native" => Some(Self::Native),
            _ => None,
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

#[cfg(test)]
mod sandbox_type_tests {
    use super::SandboxType;

    /// Guard against `as_wire_str` drifting from the `#[serde(rename_all =
    /// "kebab-case")]` derivation — those two must stay identical since
    /// manifests written with one are read back through the other (via
    /// `InstanceConfig`'s serde roundtrip in config.toml).
    #[test]
    fn wire_str_matches_serde() {
        for v in [SandboxType::Wsl2Alpine, SandboxType::LimaAlpine,
                  SandboxType::PodmanAlpine, SandboxType::Native] {
            let serde_str = serde_json::to_value(v).unwrap()
                .as_str().unwrap().to_string();
            assert_eq!(v.as_wire_str(), serde_str,
                "as_wire_str() out of sync with serde for {v:?}");
            assert_eq!(SandboxType::parse_wire(&serde_str), Some(v));
        }
    }
}
