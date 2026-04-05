use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub os: OsType,
    pub arch: Arch,
    pub os_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsType {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86_64,
    Aarch64,
}

pub fn detect_platform() -> Result<PlatformInfo> {
    let os = match std::env::consts::OS {
        "windows" => OsType::Windows,
        "macos" => OsType::Macos,
        "linux" => OsType::Linux,
        other => return Err(anyhow!("Unsupported OS: {}", other)),
    };

    let arch = match std::env::consts::ARCH {
        "x86_64" => Arch::X86_64,
        "aarch64" => Arch::Aarch64,
        other => return Err(anyhow!("Unsupported architecture: {}", other)),
    };

    Ok(PlatformInfo {
        os,
        arch,
        os_version: String::new(),
    })
}
