mod lima;
mod native;
mod wsl;
mod podman;
mod exec_helper;

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
    async fn install_package(&self, packages: &str) -> Result<()> {
        self.exec(&format!("sudo apk add --no-cache {packages} 2>&1 || apk add --no-cache {packages} 2>&1")).await?;
        Ok(())
    }

    /// 安装 npm 全局包
    async fn npm_install_global(&self, package: &str, tx: &mpsc::Sender<String>) -> Result<()> {
        self.exec_with_progress(
            &format!("sudo npm install -g {package} 2>&1 || npm install -g {package} 2>&1"),
            tx,
        ).await?;
        Ok(())
    }

    /// 创建快照
    async fn snapshot_create(&self, tag: &str) -> Result<()>;

    /// 还原快照
    async fn snapshot_restore(&self, tag: &str) -> Result<()>;

    /// 列出快照
    async fn snapshot_list(&self) -> Result<Vec<SnapshotInfo>>;

    /// 获取资源使用情况
    async fn stats(&self) -> Result<ResourceStats>;

    /// 导入预构建镜像
    async fn import_image(&self, path: &std::path::Path) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxOpts {
    pub instance_name: String,
    pub claw_version: String,
    pub alpine_version: String,
    pub memory_mb: u32,
    pub cpu_cores: u32,
    pub install_browser: bool,
    pub install_mode: InstallMode,
    /// Proxy env lines for provision script (empty string if no proxy)
    #[serde(default)]
    pub proxy_script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstallMode {
    /// 在线构建：下载 Alpine base + 逐步安装
    OnlineBuild,
    /// 预构建镜像：下载或本地导入
    PrebuiltImage { source: ImageSource },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageSource {
    /// 从 GitHub Releases 下载
    Remote { url: String, checksum_sha256: String },
    /// 从本地文件导入
    LocalFile { path: std::path::PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub tag: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub size_bytes: u64,
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
    pub fn from_os() -> Self {
        match detect_platform().map(|p| p.os) {
            Ok(OsType::Windows) => Self::Wsl2Alpine,
            Ok(OsType::Macos) => Self::LimaAlpine,
            Ok(OsType::Linux) | Err(_) => Self::PodmanAlpine,
        }
    }
}

/// 工厂函数：根据当前平台自动选择后端
pub fn detect_backend() -> Result<Box<dyn SandboxBackend>> {
    let platform = detect_platform()?;
    match platform.os {
        OsType::Windows => Ok(Box::new(WslBackend::new("default"))),
        OsType::Macos => Ok(Box::new(LimaBackend::new("default"))),
        OsType::Linux => Ok(Box::new(PodmanBackend::with_defaults("default"))),
    }
}

/// 创建 native 模式后端（开发者专用）
pub fn native_backend(instance_name: &str) -> NativeBackend {
    NativeBackend::new(instance_name)
}
