//! Structured error codes for CLI → GUI error handling.
//!
//! Frontend can match on `code` to show localized messages or take
//! specific recovery actions (e.g., redirect to proxy settings on NetworkTimeout).

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Config file not found or cannot be created
    ConfigNotFound,
    /// Config file is corrupted (backed up and recreated)
    ConfigCorrupted,
    /// Instance name not found in config
    InstanceNotFound,
    /// Instance name already exists
    InstanceExists,
    /// Sandbox backend (Lima/WSL2/Podman) not available
    SandboxNotAvailable,
    /// Sandbox VM failed to start or is unreachable
    SandboxUnreachable,
    /// Network request timed out or failed (DNS, connection refused, proxy error)
    NetworkError,
    /// Network timeout specifically during npm/apk package download
    NetworkTimeout,
    /// Operation stalled — no output for idle timeout period
    OperationStalled,
    /// npm install or claw installation failed
    InstallFailed,
    /// Gateway process failed to start
    GatewayFailed,
    /// Invalid argument or parameter
    InvalidArgument,
    /// Operation not supported on this platform or backend
    NotSupported,
    /// API key format invalid or verification failed
    InvalidApiKey,
    /// Keychain access failed
    KeychainError,
    /// Provision script crashed (process exited without writing done-file)
    ProvisionCrashed,
    /// Port conflict — requested port already in use by another instance
    PortConflict,
    /// Export/import operation failed (tar, file I/O)
    ExportFailed,
    /// Import file validation failed (wrong platform, corrupt archive)
    ImportInvalid,
    /// Sandbox resource edit failed (CPU/memory/disk)
    ResourceEditFailed,
    /// Bridge server failed to start or is not reachable
    BridgeFailed,
    /// Permission denied by bridge permission rules or user
    PermissionDenied,
    /// Upgrade check or upgrade operation failed
    UpgradeFailed,
    /// Generic / uncategorized error
    Internal,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
