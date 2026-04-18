//! ManagedShell — unified abstraction for executing commands with ClawEnv's
//! own Node.js, Git, and instance binaries in PATH.
//!
//! All native-mode command execution goes through this module, ensuring:
//! - ClawEnv's node/npm/git are always found before system ones
//! - PATH construction is consistent across all call sites
//! - Detached process spawning works correctly on all platforms

use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Unified shell for native mode — manages PATH and process spawning.
pub struct ManagedShell {
    /// ClawEnv home directory (~/.clawenv)
    clawenv_dir: PathBuf,
    /// Native instance install directory (~/.clawenv/native)
    install_dir: PathBuf,
}

impl Default for ManagedShell {
    fn default() -> Self {
        Self::new()
    }
}

impl ManagedShell {
    /// Create a ManagedShell for the native instance.
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let clawenv_dir = home.join(".clawenv");
        let install_dir = clawenv_dir.join("native");
        Self { clawenv_dir, install_dir }
    }

    /// ClawEnv node directory
    pub fn node_dir(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        { self.clawenv_dir.join("node") }
        #[cfg(not(target_os = "windows"))]
        { self.clawenv_dir.join("node").join("bin") }
    }

    /// ClawEnv git directory
    pub fn git_dir(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        { self.clawenv_dir.join("git").join("cmd") }
        #[cfg(not(target_os = "windows"))]
        { self.clawenv_dir.join("git").join("bin") }
    }

    /// Instance binary directory (npm --prefix puts bins here)
    pub fn inst_bin_dir(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        { self.install_dir.clone() }
        #[cfg(not(target_os = "windows"))]
        { self.install_dir.join("bin") }
    }

    /// Build full PATH string: ClawEnv dirs first, then system PATH.
    pub fn path(&self) -> String {
        let node = self.node_dir();
        let git = self.git_dir();
        let inst = self.inst_bin_dir();
        let sys = std::env::var("PATH").unwrap_or_default();

        #[cfg(target_os = "windows")]
        { format!("{};{};{};{}", node.display(), git.display(), inst.display(), sys) }
        #[cfg(not(target_os = "windows"))]
        { format!("{}:{}:{}:{}", node.display(), git.display(), inst.display(), sys) }
    }

    /// Node.exe / node binary path
    pub fn node_bin(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        { self.clawenv_dir.join("node").join("node.exe") }
        #[cfg(not(target_os = "windows"))]
        { self.clawenv_dir.join("node").join("bin").join("node") }
    }

    /// Find the openclaw JS entry point for direct node execution.
    /// Avoids .cmd/.ps1 wrappers entirely.
    pub fn find_claw_entry(&self, cli_binary: &str) -> Option<PathBuf> {
        // npm --prefix installs to {prefix}/node_modules/{pkg}/
        // The entry point is typically {pkg}.mjs or bin/{pkg}.js
        let pkg_dir = self.install_dir.join("node_modules").join(cli_binary);
        // Try common entry points
        for entry in [
            format!("{cli_binary}.mjs"),
            format!("bin/{cli_binary}.js"),
            format!("dist/{cli_binary}.mjs"),
            "dist/index.mjs".to_string(),
        ] {
            let p = pkg_dir.join(&entry);
            if p.exists() { return Some(p); }
        }
        // Fallback: check package.json "bin" field
        let pkg_json = pkg_dir.join("package.json");
        if let Ok(content) = std::fs::read_to_string(&pkg_json) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(bin) = v.get("bin") {
                    // "bin": "dist/index.mjs" or "bin": { "openclaw": "dist/index.mjs" }
                    let entry = if bin.is_string() {
                        bin.as_str().map(String::from)
                    } else if let Some(obj) = bin.as_object() {
                        obj.get(cli_binary).and_then(|v| v.as_str().map(String::from))
                    } else { None };
                    if let Some(e) = entry {
                        let p = pkg_dir.join(&e);
                        if p.exists() { return Some(p); }
                    }
                }
            }
        }
        None
    }

    /// Create a shell command with ClawEnv PATH injected.
    /// Windows: PowerShell with -ExecutionPolicy Bypass
    /// Unix: sh -c with export PATH=...
    pub fn cmd(&self, command: &str) -> Command {
        let path = self.path();
        #[cfg(target_os = "windows")]
        {
            let mut c = super::process::silent_cmd("powershell");
            let full = format!("$env:PATH = '{}'; {}", path.replace('\'', "''"), command);
            c.args(["-Command", &full]);
            c
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut c = Command::new("sh");
            c.args(["-c", &format!("export PATH='{}'; {}", path, command)]);
            c
        }
    }

    /// Spawn a truly detached background process.
    /// Windows: write a one-shot `.bat` wrapper (path env + node invocation +
    ///          log redirection), then `powershell Start-Process` the .bat
    ///          with `-WindowStyle Hidden`. Using a file instead of inline
    ///          ArgumentList sidesteps PowerShell's positional-parameter
    ///          parsing which chokes on `--flag=value` args inside the
    ///          argument list. The .bat explicitly redirects stdin from NUL
    ///          and stdout/stderr to the log, so the node child gets a clean
    ///          stdio setup even when the outer cmd window is hidden.
    /// Unix: nohup via sh.
    pub async fn spawn_detached(
        &self,
        cli_binary: &str,
        args: &[&str],
        log_path: &Path,
    ) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let bat_line = if let Some(entry) = self.find_claw_entry(cli_binary) {
                let node = self.node_bin();
                format!(
                    "\"{node}\" --disable-warning=ExperimentalWarning \"{entry}\" {args} < NUL > \"{log}\" 2>&1\r\n",
                    node = node.display(),
                    entry = entry.display(),
                    args = args.join(" "),
                    log = log_path.display()
                )
            } else {
                format!(
                    "{cli} {args} < NUL > \"{log}\" 2>&1\r\n",
                    cli = cli_binary,
                    args = args.join(" "),
                    log = log_path.display()
                )
            };

            // Temp .bat next to the install dir. Overwritten each spawn so the
            // file reflects the current invocation (useful for debugging with
            // `type spawn.bat`). Keeping it local-to-install avoids %TEMP%
            // cleanup oddities on multi-user machines.
            let bat_path = self.install_dir.join(".clawenv-spawn.bat");
            let bat_body = format!(
                "@echo off\r\nSET PATH={path};%PATH%\r\n{line}",
                path = self.path(),
                line = bat_line,
            );
            std::fs::write(&bat_path, bat_body)?;

            // PowerShell Start-Process — the bat handles all redirection, so
            // no -RedirectStandardOutput/Error is passed (that flag is
            // separately buggy for long-running hidden children on Windows).
            // Gateway readiness takes ~20s to reach the "listening" state on
            // Windows ARM64; the outer start_instance health-check must give
            // it enough time.
            let bat_ps = bat_path.to_string_lossy().replace('\'', "''");
            let ps_cmd = format!(
                "Start-Process -WindowStyle Hidden -FilePath '{bat_ps}' | Out-Null"
            );

            let status = super::process::silent_cmd("powershell")
                .args(["-NoProfile", "-Command", &ps_cmd])
                .current_dir(&self.install_dir)
                .status()
                .await?;

            if !status.success() {
                anyhow::bail!(
                    "Failed to spawn {cli_binary} via PowerShell Start-Process. \
                     See gateway log at {}",
                    log_path.display()
                );
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let full_cmd = format!("{} {}", cli_binary, args.join(" "));
            let path = self.path();
            let log = log_path.to_string_lossy();
            let shell_cmd = format!("export PATH='{}'; nohup {} > '{}' 2>&1 &", path, full_cmd, log);
            tokio::process::Command::new("sh")
                .args(["-c", &shell_cmd])
                .current_dir(&self.install_dir)
                .status()
                .await?;
        }
        Ok(())
    }
}
