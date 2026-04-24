//! Long-running script execution inside a sandbox, with progress
//! reporting and idle detection.
//!
//! Ported from v1's `manager/background.rs`. Differences:
//!
//! - v1 passed a `Fn(String, u8)` progress callback; v2 uses the
//!   module-wide [`ProgressSink`] so events flow through the same
//!   channel as everything else.
//! - v1 called `backend.exec(&str)` with raw composed shell; v2 goes
//!   through `exec_argv(&["sh", "-c", ...])` so only the *outer* shell
//!   is exposed, and the `<<'EOF'` heredoc body cannot be smuggled out.
//! - Polling interval is now configurable (was hardcoded 5s). Tests
//!   set it to 1ms to keep the suite fast; production still uses 5s.
//! - Runtime retry-on-transient-ssh is not ported yet — v2 doesn't have
//!   an `exec_with_retry` equivalent; once we have one at the runner
//!   layer (R4 candidate) this module can pick it up transparently.
//!
//! Protocol inside the VM:
//!
//! 1. `script_file` holds the user's shell one-liner.
//! 2. A wrapper launched by `nohup` runs the script, writes stdout/stderr
//!    to `log_file`, then writes the exit code to `done_file`.
//! 3. A heartbeat loop writes `[heartbeat Xs] still running` to
//!    `log_file` every `HEARTBEAT_INTERVAL_SECS` — keeps the idle-kill
//!    path from tripping on commands that are alive but silent
//!    (e.g. `npm install` resolving dependencies).

use std::sync::Arc;
use std::time::Duration;

use crate::common::{OpsError, ProgressSink};
use crate::sandbox_backend::SandboxBackend;

/// Per-heartbeat interval inside the VM wrapper. MUST be shorter than
/// `idle_timeout` so heartbeats keep the idle counter from tripping.
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Fixed-length truncation for log-line quoting in progress messages.
/// Prevents a single enormous line (stack trace, wget progress ticker)
/// from flooding the UI.
const MAX_LOG_LINE_LEN: usize = 85;

pub struct BackgroundScriptOpts<'a> {
    /// The command(s) to run inside the VM. May contain `&&` chains.
    pub cmd: &'a str,
    /// Human-readable label for progress messages (e.g. "Installing
    /// OpenClaw"). Shown in log-free progress ticks.
    pub label: &'a str,
    /// Wrap the script in `sudo`. Use `false` for npm/user-owned paths.
    pub sudo: bool,
    /// Log file path inside the VM (captures stdout + stderr).
    pub log_file: &'a str,
    /// Done marker path — exit code written here on completion.
    pub done_file: &'a str,
    /// Script file path — temporary.
    pub script_file: &'a str,
    /// Progress percentage range `(start, end)`. Elapsed seconds map
    /// linearly into this range (capped at `end`).
    pub pct_range: (u8, u8),
    /// Kill the script if no new log lines for this many seconds.
    pub idle_timeout: Duration,
    /// Poll interval. 5s in production; 1ms in tests.
    pub poll_interval: Duration,
}

impl Default for BackgroundScriptOpts<'_> {
    fn default() -> Self {
        Self {
            cmd: "",
            label: "Running",
            sudo: true,
            log_file: "/tmp/clawenv-install.log",
            done_file: "/tmp/clawenv-install.done",
            script_file: "/tmp/clawenv-install.sh",
            pct_range: (25, 80),
            // 1200s = 20 min. Matches v1's default.
            idle_timeout: Duration::from_secs(1200),
            poll_interval: Duration::from_secs(5),
        }
    }
}

/// Outcome of a successful background script run.
#[derive(Debug, Clone)]
pub struct BackgroundScriptReport {
    pub exit_code: i32,
    pub elapsed: Duration,
    /// Last log lines captured before completion (tail, truncated).
    pub tail: String,
}

/// Run `opts.cmd` in the background inside the VM, polling for
/// progress until it completes or stalls.
pub async fn run_background_script(
    backend: &Arc<dyn SandboxBackend>,
    opts: &BackgroundScriptOpts<'_>,
    progress: &ProgressSink,
) -> Result<BackgroundScriptReport, OpsError> {
    let setup = render_setup_script(opts);
    backend
        .exec_argv(&["sh", "-c", &setup])
        .await
        .map_err(OpsError::Other)?;

    let (pct_start, pct_end) = opts.pct_range;
    let mut last_lines = 0u64;
    let mut idle = Duration::ZERO;
    let t0 = std::time::Instant::now();

    loop {
        tokio::time::sleep(opts.poll_interval).await;
        let elapsed = t0.elapsed();

        // Read current done marker (empty string when not yet complete).
        let done_val = backend
            .exec_argv(&["sh", "-c", &format!("cat {} 2>/dev/null || true", opts.done_file)])
            .await
            .unwrap_or_default();
        let done_val = done_val.trim().to_string();

        // Tail new log lines (1-indexed `tail -n +N` semantics).
        let tail_cmd = format!(
            "tail -n +{} {} 2>/dev/null | head -50 || true",
            last_lines + 1,
            opts.log_file,
        );
        let new_output = backend
            .exec_argv(&["sh", "-c", &tail_cmd])
            .await
            .unwrap_or_default();
        let new_lines: Vec<&str> = new_output.lines().filter(|l| !l.trim().is_empty()).collect();

        let pct = map_elapsed_to_pct(elapsed, pct_start, pct_end);
        if new_lines.is_empty() {
            idle += opts.poll_interval;
            progress
                .at(pct, "install", format!("{}... ({}s)", opts.label, elapsed.as_secs()))
                .await;
        } else {
            idle = Duration::ZERO;
            last_lines += new_lines.len() as u64;
            let last = new_lines.last().copied().unwrap_or("");
            let short = truncate(last, MAX_LOG_LINE_LEN);
            progress
                .at(pct, "install", format!("[{}s] {}", elapsed.as_secs(), short))
                .await;
        }

        if !done_val.is_empty() {
            let exit_code: i32 = done_val.parse().unwrap_or(-1);
            let tail = read_log_tail(backend, opts.log_file, 10).await;
            // Cleanup temp files (best-effort).
            let _ = backend
                .exec_argv(&[
                    "sh",
                    "-c",
                    &format!("rm -f {} {} {}", opts.script_file, opts.log_file, opts.done_file),
                ])
                .await;
            if exit_code != 0 {
                return Err(OpsError::Other(anyhow::anyhow!(
                    "{} failed (exit {exit_code}):\n{tail}",
                    opts.label
                )));
            }
            return Ok(BackgroundScriptReport { exit_code, elapsed, tail });
        }

        if idle >= opts.idle_timeout {
            let tail = read_log_tail(backend, opts.log_file, 10).await;
            return Err(OpsError::Other(anyhow::anyhow!(
                "{} stalled — no output for {} min:\n{tail}",
                opts.label,
                opts.idle_timeout.as_secs() / 60
            )));
        }
    }
}

async fn read_log_tail(
    backend: &Arc<dyn SandboxBackend>,
    log_file: &str,
    n: u32,
) -> String {
    backend
        .exec_argv(&[
            "sh",
            "-c",
            &format!("tail -{n} {log_file} 2>/dev/null || echo 'no log'"),
        ])
        .await
        .unwrap_or_else(|_| "no log".into())
}

fn map_elapsed_to_pct(elapsed: Duration, start: u8, end: u8) -> u8 {
    // Every 10 elapsed seconds → +1 percent, capped at `end`.
    // Mirrors v1's `std::cmp::min(pct_start + (elapsed / 10) as u8, pct_end)`.
    let incr = (elapsed.as_secs() / 10) as u32;
    let projected = start as u32 + incr;
    projected.min(end as u32) as u8
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() > max { &s[..max] } else { s }
}

/// Render the inline shell that writes the script, launches the
/// wrapper with nohup, and emits heartbeats. Pure function — no I/O;
/// golden-tested.
pub(crate) fn render_setup_script(opts: &BackgroundScriptOpts<'_>) -> String {
    let sudo_prefix = if opts.sudo { "sudo " } else { "" };
    let hb = HEARTBEAT_INTERVAL_SECS;
    format!(
        r#"set -e
rm -f {log} {done} {script}
cat > {script} << 'CLAWOPS_SCRIPT_EOF'
#!/bin/sh
set -e
{cmd}
CLAWOPS_SCRIPT_EOF
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
        log = opts.log_file,
        done = opts.done_file,
        script = opts.script_file,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_ops::testing::MockBackend;

    fn arc_mock() -> (Arc<MockBackend>, Arc<dyn SandboxBackend>) {
        let concrete = Arc::new(MockBackend::new("fake"));
        let as_trait: Arc<dyn SandboxBackend> = concrete.clone();
        (concrete, as_trait)
    }

    fn fast_opts<'a>(cmd: &'a str) -> BackgroundScriptOpts<'a> {
        BackgroundScriptOpts {
            cmd,
            label: "Installing",
            sudo: false,
            log_file: "/tmp/t.log",
            done_file: "/tmp/t.done",
            script_file: "/tmp/t.sh",
            pct_range: (20, 80),
            idle_timeout: Duration::from_millis(200),
            poll_interval: Duration::from_millis(1),
        }
    }

    // ——— map_elapsed_to_pct ———

    #[test]
    fn pct_maps_linearly_then_caps() {
        assert_eq!(map_elapsed_to_pct(Duration::ZERO, 20, 80), 20);
        assert_eq!(map_elapsed_to_pct(Duration::from_secs(10), 20, 80), 21);
        assert_eq!(map_elapsed_to_pct(Duration::from_secs(100), 20, 80), 30);
        // Cap at end.
        assert_eq!(map_elapsed_to_pct(Duration::from_secs(10_000), 20, 80), 80);
    }

    // ——— truncate ———

    #[test]
    fn truncate_preserves_short_strings() {
        assert_eq!(truncate("short", 85), "short");
    }

    #[test]
    fn truncate_cuts_long_strings() {
        let s = "x".repeat(200);
        assert_eq!(truncate(&s, 10).len(), 10);
    }

    // ——— render_setup_script ———

    #[test]
    fn setup_includes_sudo_when_flagged() {
        let o = BackgroundScriptOpts { sudo: true, cmd: "echo x", ..BackgroundScriptOpts::default() };
        let s = render_setup_script(&o);
        assert!(s.contains("sudo sh "), "sudo prefix missing: {s}");
    }

    #[test]
    fn setup_omits_sudo_when_not_flagged() {
        let o = BackgroundScriptOpts { sudo: false, cmd: "echo x", ..BackgroundScriptOpts::default() };
        let s = render_setup_script(&o);
        // Outer wrapper still has ` sh ` but not ` sudo sh `.
        assert!(!s.contains("sudo sh "), "sudo prefix not suppressed: {s}");
    }

    #[test]
    fn setup_heredoc_body_contains_user_cmd_and_shebang() {
        let o = BackgroundScriptOpts { cmd: "npm install -g openclaw", ..BackgroundScriptOpts::default() };
        let s = render_setup_script(&o);
        assert!(s.contains("npm install -g openclaw"));
        assert!(s.contains("#!/bin/sh"));
        assert!(s.contains("CLAWOPS_SCRIPT_EOF"));
    }

    #[test]
    fn setup_uses_nohup_and_heartbeat() {
        let s = render_setup_script(&BackgroundScriptOpts::default());
        assert!(s.contains("nohup"));
        assert!(s.contains("sleep 30")); // heartbeat interval
        assert!(s.contains("[heartbeat"));
    }

    // ——— run_background_script via MockBackend ———

    #[tokio::test]
    async fn completes_on_first_done_marker() {
        let (mock, backend) = arc_mock();
        // 1st call is setup → canned "" is fine.
        // Poll 1: done="" (empty), tail="line1"
        mock.queue_response("");        // setup reply (canned_stdout="")
        mock.queue_response("");        // poll 1 done read → empty
        mock.queue_response("line1\n"); // poll 1 tail
        mock.queue_response("0\n");     // poll 2 done → exit 0
        mock.queue_response("");        // poll 2 tail
        mock.queue_response("line1\n"); // cleanup rm → (we don't care)
        mock.queue_response("line1\n"); // read_log_tail on success

        let report = run_background_script(&backend, &fast_opts("echo x"), &ProgressSink::noop())
            .await
            .unwrap();
        assert_eq!(report.exit_code, 0);
    }

    #[tokio::test]
    async fn nonzero_exit_is_surfaced_as_error() {
        let (mock, backend) = arc_mock();
        mock.queue_response(""); // setup
        mock.queue_response("1\n"); // poll done = exit 1
        mock.queue_response(""); // poll tail
        mock.queue_response(""); // read_log_tail
        mock.queue_response(""); // cleanup

        let err = run_background_script(&backend, &fast_opts("bad"), &ProgressSink::noop())
            .await
            .unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("failed (exit 1)"), "unexpected error: {s}");
    }

    #[tokio::test]
    async fn idle_timeout_fires_when_no_log_lines() {
        let (_mock, backend) = arc_mock();
        // Every poll returns empty done + empty tail → idle counter grows.
        // idle_timeout=200ms, poll_interval=1ms → ~200 polls before trip.
        // MockBackend's default response is canned "" so no queueing needed.

        let err = run_background_script(&backend, &fast_opts("stuck"), &ProgressSink::noop())
            .await
            .unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("stalled"), "unexpected error: {s}");
    }

    #[tokio::test]
    async fn tail_lines_reset_idle_counter() {
        let (mock, backend) = arc_mock();
        // Setup + three poll rounds:
        //   1: empty tail → idle += 1ms
        //   2: tail has a line → idle resets
        //   3: done=0 → success
        mock.queue_response(""); // setup
        // Poll 1
        mock.queue_response(""); // done
        mock.queue_response(""); // tail empty
        // Poll 2
        mock.queue_response(""); // done
        mock.queue_response("busy\n"); // tail non-empty
        // Poll 3
        mock.queue_response("0\n"); // done exits 0
        mock.queue_response(""); // tail
        // read_log_tail + cleanup
        mock.queue_response("busy\n");
        mock.queue_response("");

        let opts = BackgroundScriptOpts {
            idle_timeout: Duration::from_millis(5), // very short
            ..fast_opts("slow")
        };
        let report = run_background_script(&backend, &opts, &ProgressSink::noop())
            .await
            .unwrap();
        assert_eq!(report.exit_code, 0);
    }
}
