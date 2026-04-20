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
        let clawenv_dir = crate::config::clawenv_root();
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
        let mut c = {
            let mut c = super::process::silent_cmd("powershell");
            let full = format!("$env:PATH = '{}'; {}", path.replace('\'', "''"), command);
            c.args(["-Command", &full]);
            c
        };
        #[cfg(not(target_os = "windows"))]
        let mut c = {
            let mut c = Command::new("sh");
            c.args(["-c", &format!("export PATH='{}'; {}", path, command)]);
            c
        };
        // Only inject the ssh→https rewrite when a proxy is in effect.
        // Strict semantic: proxy_on=false → do NOT modify git behaviour.
        // Rationale: without a proxy, rewriting ssh://git@github.com/ to
        // https://github.com/ forces HTTPS to github which is flaky in
        // GFW networks (TLS handshake hangs 30-60s per package, cascading
        // into multi-minute freezes on 748-package openclaw installs).
        // When there's no proxy, let the original ssh URL fail fast on
        // port 22 instead.
        if proxy_is_active() {
            apply_git_ssh_rewrite(&mut c);
        }
        self.apply_git_exec_path(&mut c);
        c
    }

    /// Dugite's `git` ships its helper commands (`git-remote-https` etc.)
    /// in `<clawenv>/git/libexec/git-core`, but the binary was built with
    /// a hardcoded RUNTIME_PREFIX that resolves to `//libexec/git-core`
    /// on Mac — so `git` can't find its own helpers and anything beyond
    /// a bare `git --version` fails with `remote-https is not a git command`.
    /// Setting `GIT_EXEC_PATH` overrides the broken auto-detection. Only
    /// set it when the dugite install actually exists, so a user's system
    /// git (if someone removed the bundled one) keeps its own resolution.
    fn apply_git_exec_path(&self, cmd: &mut Command) {
        #[cfg(not(target_os = "windows"))]
        {
            let exec_path = self.clawenv_dir.join("git").join("libexec").join("git-core");
            if exec_path.exists() {
                cmd.env("GIT_EXEC_PATH", exec_path);
            }
        }
        #[cfg(target_os = "windows")]
        {
            let exec_path = self.clawenv_dir.join("git").join("mingw64").join("libexec").join("git-core");
            if exec_path.exists() {
                cmd.env("GIT_EXEC_PATH", exec_path);
            }
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

            // IMPORTANT: we must NOT use `Start-Process -WindowStyle Hidden`
            // here. Over Windows OpenSSH (and likely any launcher running
            // inside a server-side job object), `Start-Process` leaves the
            // spawned child inside the caller's job object. When the outer
            // clawcli.exe returns and the SSH channel closes, Windows kills
            // the entire tree — including our "detached" gateway. E2E caught
            // this: install claimed gateway-up; a /health check issued from
            // a fresh SSH session moments later found port unbound and no
            // node.exe alive.
            //
            // `Invoke-CimMethod Win32_Process Create` creates the new
            // process through WMI. Unlike Start-Process, WMI process
            // creation does NOT inherit the caller's job object — the child
            // becomes a true top-level process and survives parent exit
            // (SSH disconnect, tray-app close, etc.). Same mechanism as the
            // deprecated `wmic process call create`, but via the modern CIM
            // cmdlet so it keeps working on Windows 11+ where wmic is being
            // phased out.
            //
            // The .bat handles redirection (stdin < NUL, stdout/stderr to
            // gateway.log) so the spawned cmd has no live console.
            //
            // We drop the PS script on disk (next to the .bat) and invoke it
            // via `powershell -File` rather than `-Command`. Much simpler
            // escaping story — the quoted path inside Invoke-CimMethod has
            // both `'` and `"` in a specific arrangement that's painful to
            // thread through `-Command` argv encoding correctly.
            let ps_path = self.install_dir.join(".clawenv-spawn.ps1");
            let ps_body = format!(
                "Invoke-CimMethod -ClassName Win32_Process -MethodName Create \
                 -Arguments @{{CommandLine='cmd /c \"{}\"'}} | Out-Null\r\n",
                bat_path.display()
            );
            std::fs::write(&ps_path, ps_body)?;

            let status = super::process::silent_cmd("powershell")
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy", "Bypass",
                    "-File", &ps_path.to_string_lossy(),
                ])
                .current_dir(&self.install_dir)
                .status()
                .await?;

            if !status.success() {
                anyhow::bail!(
                    "Failed to spawn {cli_binary} via Invoke-CimMethod. \
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

/// Inject an ephemeral git config that rewrites `ssh://git@github.com/` →
/// `https://github.com/` for every child process spawned through this
/// shell. Uses the `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_N` / `GIT_CONFIG_VALUE_N`
/// scheme (git 2.31+, 2021) so no user config file is touched.
///
/// Why: OpenClaw's npm dep graph includes a package (libsignal-node)
/// whose package.json lists a git dep via `ssh://git@github.com/…`.
/// Under an HTTP proxy (the Windows E2E proxy scenario), port 22 isn't
/// proxied, so `npm install` fails on that git clone. Rewriting the URL
/// at git level sends the clone over HTTPS which the proxy does handle.
/// Safe to apply unconditionally — we never want an ssh:// clone here.
pub(crate) fn apply_git_ssh_rewrite(cmd: &mut Command) {
    cmd.env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "url.https://github.com/.insteadOf")
        .env("GIT_CONFIG_VALUE_0", "ssh://git@github.com/");
}

/// Detect whether a proxy is active in the current process env. Reads
/// the canonical env vars (`apply_env` and `apply_child_cmd` populate
/// these) — doesn't re-enter the resolver because callers here are
/// already downstream of it. Empty = no proxy.
fn proxy_is_active() -> bool {
    for key in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy",
                "ALL_PROXY", "all_proxy"] {
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                return true;
            }
        }
    }
    false
}
