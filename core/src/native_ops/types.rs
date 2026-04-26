use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::sandbox_ops::Severity;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub version: String,
    pub path: PathBuf,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeStatus {
    pub clawenv_home: PathBuf,
    pub home_exists: bool,
    pub node: Option<ComponentInfo>,
    pub git: Option<ComponentInfo>,
    pub total_disk_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub version: Option<String>,
    pub path: Option<PathBuf>,
    pub healthy: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDoctorIssue {
    pub id: String,
    pub severity: Severity,
    pub message: String,
    pub repair_hint: Option<String>,
    pub auto_repairable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDoctorReport {
    pub issues: Vec<NativeDoctorIssue>,
    pub checked_at: String,
}

impl NativeDoctorReport {
    pub fn healthy(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }
}

#[derive(Debug, Clone)]
pub enum VersionSpec {
    Latest,
    Exact(String),
}
