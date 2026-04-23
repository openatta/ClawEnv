//! Error hierarchy. `OpsError` is the top-level type every Ops method returns.

use thiserror::Error;

/// Command-runner errors (spawn, timeout, cancel, IO, parse).
#[derive(Error, Debug)]
pub enum CommandError {
    #[error("failed to spawn `{binary}`: {source}")]
    SpawnFailed {
        binary: String,
        #[source]
        source: std::io::Error,
    },

    #[error("command timed out after {0:?}")]
    TimedOut(std::time::Duration),

    #[error("command was cancelled")]
    Cancelled,

    #[error("command exited with code {exit_code}")]
    NonZeroExit {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },

    #[error("stdout is not valid JSON: {source}")]
    JsonParse {
        #[source]
        source: serde_json::Error,
        stdout: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("runner error: {0}")]
    Runner(String),
}

/// Download-specific errors.
#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("artifact `{name}` not found in catalog")]
    NotInCatalog { name: String },

    #[error("artifact `{name}` version `{version}` not available for {os}/{arch}")]
    NoMatchingVersion { name: String, version: String, os: String, arch: String },

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("stall: no data for {seconds}s from {url}")]
    Stalled { url: String, seconds: u64 },

    #[error("throughput floor: only {bytes} bytes in {seconds}s from {url}")]
    SlowThroughput { url: String, bytes: u64, seconds: u64 },

    #[error("checksum mismatch: expected {expected}, got {got}")]
    ChecksumMismatch { expected: String, got: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Top-level error type for every Ops method.
#[derive(Error, Debug)]
pub enum OpsError {
    #[error(transparent)]
    Command(#[from] CommandError),

    #[error(transparent)]
    Download(#[from] DownloadError),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("unsupported: {what} — {reason}")]
    Unsupported { what: String, reason: String },

    #[error("not found: {what}")]
    NotFound { what: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl OpsError {
    pub fn unsupported(what: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Unsupported { what: what.into(), reason: reason.into() }
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::NotFound { what: what.into() }
    }
    pub fn parse(msg: impl Into<String>) -> Self {
        Self::Parse(msg.into())
    }
}
