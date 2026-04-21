mod detector;
pub mod download;
pub mod managed_shell;
pub mod network;
pub mod preflight;
pub mod process;

pub use detector::{detect_platform, PlatformInfo, OsType, Arch};

/// Escape a string for safe use inside single-quoted shell arguments.
/// Wraps the result in single quotes: `'value'`.
/// Safe for sh, bash, zsh, ash (Alpine).
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Escape a string for safe use in PowerShell single-quoted strings.
/// PowerShell single quotes only need `''` to escape a literal `'`.
pub fn powershell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
