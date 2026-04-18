use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgePermissions {
    #[serde(default)]
    pub file_read: Vec<String>,
    #[serde(default)]
    pub file_write: Vec<String>,
    #[serde(default)]
    pub file_deny: Vec<String>,
    #[serde(default)]
    pub exec_allow: Vec<String>,
    #[serde(default)]
    pub exec_deny: Vec<String>,
    #[serde(default = "default_require_approval")]
    pub require_approval: Vec<String>,
    #[serde(default = "default_auto_approve")]
    pub auto_approve: Vec<String>,
    /// Shell mode: allow agents to execute arbitrary shell scripts
    #[serde(default)]
    pub shell_enabled: bool,
    /// Shell program: bash (macOS/Linux), powershell (Windows)
    #[serde(default = "default_shell_program")]
    pub shell_program: String,
    /// Require user approval for each shell command
    #[serde(default = "default_true")]
    pub shell_require_approval: bool,
}

fn default_true() -> bool { true }

fn default_shell_program() -> String {
    #[cfg(target_os = "windows")]
    { "powershell".into() }
    #[cfg(not(target_os = "windows"))]
    { "bash".into() }
}

fn default_require_approval() -> Vec<String> {
    vec!["file_write".into(), "exec".into()]
}

fn default_auto_approve() -> Vec<String> {
    vec!["file_read".into()]
}

impl Default for BridgePermissions {
    fn default() -> Self {
        Self {
            file_read: vec!["~/Documents/**".into(), "~/Projects/**".into()],
            file_write: vec!["~/.clawenv/workspaces/**".into()],
            file_deny: vec!["~/.ssh/**".into(), "~/.aws/**".into(), "~/.env*".into()],
            exec_allow: vec!["git".into(), "code".into(), "python3".into(), "npm".into()],
            exec_deny: vec!["rm -rf".into(), "sudo".into(), "ssh".into()],
            require_approval: default_require_approval(),
            auto_approve: default_auto_approve(),
            shell_enabled: false,
            shell_program: default_shell_program(),
            shell_require_approval: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionResult {
    Allowed,
    Denied(String),
    RequiresApproval(String),
}

impl BridgePermissions {
    /// Check if a file path is allowed for reading
    pub fn can_read_file(&self, path: &Path) -> PermissionResult {
        // Deny list always takes priority
        if Self::matches_any(path, &self.file_deny) {
            return PermissionResult::Denied(format!(
                "Path '{}' matches a deny rule",
                path.display()
            ));
        }

        // Check if allowed
        if !self.file_read.is_empty() && !Self::matches_any(path, &self.file_read) {
            return PermissionResult::Denied(format!(
                "Path '{}' is not in the allowed read list",
                path.display()
            ));
        }

        if self.auto_approve.contains(&"file_read".to_string()) {
            PermissionResult::Allowed
        } else if self.require_approval.contains(&"file_read".to_string()) {
            PermissionResult::RequiresApproval("file_read requires approval".into())
        } else {
            PermissionResult::Allowed
        }
    }

    /// Check if a file path is allowed for writing
    pub fn can_write_file(&self, path: &Path) -> PermissionResult {
        if Self::matches_any(path, &self.file_deny) {
            return PermissionResult::Denied(format!(
                "Path '{}' matches a deny rule",
                path.display()
            ));
        }

        if !self.file_write.is_empty() && !Self::matches_any(path, &self.file_write) {
            return PermissionResult::Denied(format!(
                "Path '{}' is not in the allowed write list",
                path.display()
            ));
        }

        if self.auto_approve.contains(&"file_write".to_string()) {
            PermissionResult::Allowed
        } else if self.require_approval.contains(&"file_write".to_string()) {
            PermissionResult::RequiresApproval("file_write requires approval".into())
        } else {
            PermissionResult::Allowed
        }
    }

    /// Check if a command is allowed to execute
    pub fn can_exec(&self, command: &str) -> PermissionResult {
        // Check deny list first
        for deny in &self.exec_deny {
            if command.starts_with(deny) || command.contains(deny) {
                return PermissionResult::Denied(format!(
                    "Command '{}' matches deny rule '{}'",
                    command, deny
                ));
            }
        }

        // Check allow list — extract the base command (first word)
        let base_cmd = command.split_whitespace().next().unwrap_or("");
        // Also check just the binary name without path
        let binary_name = Path::new(base_cmd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(base_cmd);

        if !self.exec_allow.is_empty()
            && !self.exec_allow.iter().any(|a| a == base_cmd || a == binary_name)
        {
            return PermissionResult::Denied(format!(
                "Command '{}' is not in the allowed exec list",
                base_cmd
            ));
        }

        if self.auto_approve.contains(&"exec".to_string()) {
            PermissionResult::Allowed
        } else if self.require_approval.contains(&"exec".to_string()) {
            PermissionResult::RequiresApproval("exec requires approval".into())
        } else {
            PermissionResult::Allowed
        }
    }

    /// Expand ~ to home directory in pattern
    fn expand_pattern(pattern: &str) -> String {
        if pattern.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                return format!("{}{}", home.display(), &pattern[1..]);
            }
        }
        pattern.to_string()
    }

    /// Match a path against glob patterns
    ///
    /// Supports basic glob matching:
    /// - `*` matches any single path component (no `/`)
    /// - `**` matches zero or more path components
    /// - exact prefix matching for non-glob patterns
    fn matches_any(path: &Path, patterns: &[String]) -> bool {
        // Normalize to forward slashes for cross-platform matching
        let path_str = path.to_string_lossy().replace('\\', "/");
        for pattern in patterns {
            let expanded = Self::expand_pattern(pattern).replace('\\', "/");
            if Self::glob_match(&path_str, &expanded) {
                return true;
            }
        }
        false
    }

    /// Simple glob matching supporting `*` and `**`
    fn glob_match(path: &str, pattern: &str) -> bool {
        // Handle ** — matches any number of path segments
        if let Some(prefix) = pattern.strip_suffix("/**") {
            return path.starts_with(prefix);
        }

        // Handle trailing * (single segment match)
        if pattern.ends_with("/*") && !pattern.ends_with("/**") {
            let prefix = &pattern[..pattern.len() - 2];
            if let Some(rest) = path.strip_prefix(prefix) {
                // Must be a single segment (one `/` then no more `/`)
                return rest.starts_with('/')
                    && rest[1..].find('/').is_none();
            }
            return false;
        }

        // Handle patterns like ~/.env* — prefix match with trailing wildcard
        if let Some(prefix) = pattern.strip_suffix('*') {
            return path.starts_with(prefix);
        }

        // Exact match
        path == pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    

    #[test]
    fn test_deny_takes_priority() {
        let perms = BridgePermissions::default();
        let home = dirs::home_dir().unwrap();
        let ssh_path = home.join(".ssh/id_rsa");
        assert!(matches!(
            perms.can_read_file(&ssh_path),
            PermissionResult::Denied(_)
        ));
    }

    #[test]
    fn test_read_allowed_path() {
        let perms = BridgePermissions::default();
        let home = dirs::home_dir().unwrap();
        let doc_path = home.join("Documents/test.txt");
        assert_eq!(perms.can_read_file(&doc_path), PermissionResult::Allowed);
    }

    #[test]
    fn test_exec_denied() {
        let perms = BridgePermissions::default();
        assert!(matches!(
            perms.can_exec("sudo rm -rf /"),
            PermissionResult::Denied(_)
        ));
    }

    #[test]
    fn test_exec_allowed() {
        let perms = BridgePermissions::default();
        let result = perms.can_exec("git status");
        // git is in allow list, but exec requires approval by default
        assert!(matches!(result, PermissionResult::RequiresApproval(_)));
    }

    #[test]
    fn test_glob_match() {
        assert!(BridgePermissions::glob_match("/home/user/Documents/a/b", "/home/user/Documents/**"));
        assert!(BridgePermissions::glob_match("/home/user/.env.local", "/home/user/.env*"));
        assert!(!BridgePermissions::glob_match("/home/user/other", "/home/user/Documents/**"));
    }
}
