//! Background script execution and polling for sandbox VMs.
//!
//! Provides a unified way to run long-running commands inside a sandbox
//! (e.g., npm install, pip install, apk add) as background scripts with
//! progress polling, idle detection, and timeout handling.
//!
//! Used by: install.rs, upgrade.rs

use anyhow::Result;

use crate::sandbox::SandboxBackend;

/// Options for running a background script in a sandbox VM.
pub struct BackgroundScriptOpts<'a> {
    /// The command(s) to run. May contain `&&` chains.
    pub cmd: &'a str,
    /// Human-readable label for progress messages (e.g., "Installing OpenClaw").
    pub label: &'a str,
    /// Whether to wrap the command in `sudo`.
    pub sudo: bool,
    /// Log file path inside the VM.
    pub log_file: &'a str,
    /// Done marker file path inside the VM.
    pub done_file: &'a str,
    /// Script file path inside the VM.
    pub script_file: &'a str,
    /// Progress percentage range: (start, end).
    pub pct_range: (u8, u8),
    /// Maximum idle time (seconds) before considering the script stalled.
    pub max_idle_secs: u64,
}

impl<'a> Default for BackgroundScriptOpts<'a> {
    fn default() -> Self {
        Self {
            cmd: "",
            label: "Running",
            sudo: true,
            log_file: "/tmp/clawenv-bg.log",
            done_file: "/tmp/clawenv-bg.done",
            script_file: "/tmp/clawenv-bg.sh",
            pct_range: (25, 80),
            max_idle_secs: 1200,
        }
    }
}

/// Interval (seconds) between heartbeat lines written to the log from the VM
/// wrapper. Any value shorter than `max_idle_secs` prevents the idle kill path
/// from tripping while the real command is still alive but silent.
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Progress callback: (message, percent).
pub type ProgressFn = Box<dyn Fn(String, u8) + Send>;

/// Run a command as a background script in the VM, polling for progress.
///
/// 1. Writes `opts.cmd` to a script file in the VM.
/// 2. Launches it in the background with nohup.
/// 3. Polls the log file every 5 seconds for new output.
/// 4. Returns Ok(()) when the done marker appears with exit code 0.
/// 5. Returns Err if exit code != 0 or idle timeout is reached.
pub async fn run_background_script(
    backend: &dyn SandboxBackend,
    opts: &BackgroundScriptOpts<'_>,
    on_progress: impl Fn(String, u8) + Send,
) -> Result<()> {
    let log = opts.log_file;
    let done = opts.done_file;
    let script = opts.script_file;
    let (pct_start, pct_end) = opts.pct_range;

    // Merge the three pre-run setup commands (cleanup + write script +
    // launch) into a SINGLE exec. Previously 3 separate SSH round-trips
    // right after VM boot — Lima's ControlMaster warmup window sometimes
    // killed the 2nd or 3rd with `Connection reset by peer`. Also wrap
    // in retry-with-backoff for the same transient-ssh class.
    let sudo_prefix = if opts.sudo { "sudo " } else { "" };
    let hb = HEARTBEAT_INTERVAL_SECS;
    let setup_and_launch = format!(
        r#"set -e
rm -f {log} {done} {script}
cat > {script} << 'SCRIPTEOF'
#!/bin/sh
set -e
{cmd}
SCRIPTEOF
chmod +x {script}
nohup sh -c '({sudo_prefix}sh {script} > {log} 2>&1; echo $? > {done}) & \
    CMD_PID=$!; \
    hb_elapsed=0; \
    while kill -0 $CMD_PID 2>/dev/null; do \
      sleep {hb}; \
      hb_elapsed=$((hb_elapsed + {hb})); \
      echo "[heartbeat ${{hb_elapsed}}s] still running" >> {log}; \
    done' > /dev/null 2>&1 &
"#,
        cmd = opts.cmd,
    );
    crate::config::proxy_resolver::exec_with_retry(backend, &setup_and_launch, "background_setup").await?;

    // Poll for completion
    let mut last_lines = 0usize;
    let mut elapsed = 0u64;
    let mut idle = 0u64;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        elapsed += 5;

        // Check done marker
        let done_content = backend.exec(&format!("cat {done} 2>/dev/null || echo ''")).await.unwrap_or_default();
        let done_val = done_content.trim();

        // Read new log lines
        let new_output = backend.exec(&format!(
            "tail -n +{} {log} 2>/dev/null | head -50 || echo ''",
            last_lines + 1
        )).await.unwrap_or_default();

        let new_lines: Vec<&str> = new_output.lines()
            .filter(|l| !l.trim().is_empty())
            .collect();

        if !new_lines.is_empty() {
            idle = 0;
            last_lines += new_lines.len();
            let last = new_lines.last().unwrap_or(&"");
            let short = if last.len() > 85 { &last[..85] } else { last };
            let pct = std::cmp::min(pct_start + (elapsed / 10) as u8, pct_end);
            on_progress(format!("[{elapsed}s] {short}"), pct);
        } else {
            idle += 5;
            let pct = std::cmp::min(pct_start + (elapsed / 10) as u8, pct_end);
            on_progress(format!("{}... ({elapsed}s)", opts.label), pct);
        }

        // Check completion
        if !done_val.is_empty() {
            let exit_code: i32 = done_val.parse().unwrap_or(-1);
            // Cleanup
            backend.exec(&format!("rm -f {script} {log} {done}")).await.ok();
            if exit_code != 0 {
                let tail = backend.exec(&format!("tail -10 {log} 2>/dev/null || echo 'no log'")).await.unwrap_or_default();
                anyhow::bail!("{} failed (exit {exit_code}):\n{tail}", opts.label);
            }
            return Ok(());
        }

        // Idle timeout
        if idle >= opts.max_idle_secs {
            let tail = backend.exec(&format!("tail -10 {log} 2>/dev/null || echo 'no log'")).await.unwrap_or_default();
            anyhow::bail!("{} stalled — no output for {} min:\n{tail}", opts.label, opts.max_idle_secs / 60);
        }
    }
}
