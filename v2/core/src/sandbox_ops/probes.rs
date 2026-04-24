//! Shared doctor probes. Each probe returns `Option<DoctorIssue>` (or a
//! `Vec<..>` when it can surface multiple rows) so the caller just
//! `.extend()`s results into the final issue list.
//!
//! Backend-scoped probes (DNS, disk) use `SandboxBackend::exec_argv` and
//! therefore work against any backend that implements the trait — no
//! copy-paste across Lima/WSL/Podman.
//!
//! Host-scoped probes (port-conflict) go through `LocalProcessRunner` and
//! query the host OS directly.

use std::sync::Arc;
use std::time::Duration;

use crate::common::{CancellationToken, CommandRunner, CommandSpec};
use crate::runners::LocalProcessRunner;
use crate::sandbox_backend::SandboxBackend;

use super::types::{DoctorIssue, Severity};

/// Run `nslookup github.com` inside the sandbox. If the command fails or
/// the output signals no-resolution, emit a `dns-broken` issue. Returns
/// `None` when resolution succeeds AND when we cannot even contact the
/// sandbox (caller already has a `vm-not-running` issue for that).
pub(crate) async fn probe_dns(backend: &Arc<dyn SandboxBackend>) -> Option<DoctorIssue> {
    let out = backend.exec_argv(&["nslookup", "github.com"]).await.ok()?;
    // "can't resolve" / "NXDOMAIN" / "server can't find" — any one of
    // these is a clear fail signal. A successful resolve prints an
    // "Address:" line.
    let lower = out.to_lowercase();
    let fail = lower.contains("can't resolve")
        || lower.contains("nxdomain")
        || lower.contains("server can't find")
        || (!lower.contains("address:") && !lower.contains("addresses:"));
    if !fail {
        return None;
    }
    Some(DoctorIssue {
        id: "dns-broken".into(),
        severity: Severity::Error,
        message: "DNS resolution for github.com failed inside the sandbox".into(),
        repair_hint: Some(
            "Check /etc/resolv.conf inside the VM or verify proxy settings".into(),
        ),
        auto_repairable: false,
    })
}

const DISK_LOW_THRESHOLD_MB: u64 = 500;

/// Run `df -Pm /` in the sandbox and parse the Available-MB column. If
/// less than 500 MB free on /, emit `disk-low`.
pub(crate) async fn probe_disk(backend: &Arc<dyn SandboxBackend>) -> Option<DoctorIssue> {
    // -P forces POSIX output (stable columns); -m forces MB units. Alpine's
    // busybox df supports both.
    let out = backend.exec_argv(&["df", "-Pm", "/"]).await.ok()?;
    let avail_mb = parse_df_available_mb(&out)?;
    if avail_mb >= DISK_LOW_THRESHOLD_MB {
        return None;
    }
    Some(DoctorIssue {
        id: "disk-low".into(),
        severity: Severity::Warning,
        message: format!(
            "Sandbox root filesystem has only {avail_mb} MB free (< {DISK_LOW_THRESHOLD_MB})"
        ),
        repair_hint: Some(
            "Free up space inside the VM, or resize its disk (backend-specific)".into(),
        ),
        auto_repairable: false,
    })
}

/// Parse busybox / POSIX `df -Pm` output. Second line is the root row;
/// Available is column 4. Returns None if we can't parse (caller treats
/// that as "no signal" rather than an issue).
fn parse_df_available_mb(out: &str) -> Option<u64> {
    let mut lines = out.lines();
    // Header line
    let _ = lines.next()?;
    for line in lines {
        // Columns: Filesystem  1M-blocks  Used  Available  Capacity  Mounted-on
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 { continue; }
        // Target "/": mount point is last column.
        if *cols.last().unwrap() != "/" { continue; }
        if let Ok(avail) = cols[3].parse::<u64>() {
            return Some(avail);
        }
    }
    None
}

/// Check host-side whether the given TCP listen ports are already bound
/// by some other process. Returns one DoctorIssue per conflicting port.
///
/// Unix path uses `lsof -iTCP:<port> -sTCP:LISTEN -Pn`. Windows uses
/// `netstat -ano -p TCP | findstr :<port>`.
pub(crate) async fn probe_port_conflicts(ports: &[u16]) -> Vec<DoctorIssue> {
    let runner = LocalProcessRunner::new();
    let mut issues = Vec::new();
    for &port in ports {
        if let Some(desc) = check_one_port(&runner, port).await {
            issues.push(DoctorIssue {
                id: "port-conflict".into(),
                severity: Severity::Error,
                message: format!("Host port {port} already in use: {desc}"),
                repair_hint: Some(format!(
                    "Either stop the other listener, or change the sandbox port rule for {port}"
                )),
                auto_repairable: false,
            });
        }
    }
    issues
}

async fn check_one_port(runner: &LocalProcessRunner, port: u16) -> Option<String> {
    let port_s = port.to_string();

    #[cfg(not(target_os = "windows"))]
    let spec = {
        // lsof wants the port glued to the -iTCP: flag, so we build a
        // single argv element rather than two.
        let iarg = format!("-iTCP:{port_s}");
        CommandSpec::new("lsof", [iarg.as_str(), "-sTCP:LISTEN", "-Pn"])
            .with_timeout(Duration::from_secs(3))
    };

    #[cfg(target_os = "windows")]
    let spec = {
        // netstat doesn't filter by port natively; we fetch all TCP rows
        // and grep in Rust.
        CommandSpec::new("netstat", ["-ano", "-p", "TCP"])
            .with_timeout(Duration::from_secs(3))
    };

    let res = runner.exec(spec, CancellationToken::new()).await.ok()?;
    if !res.success() { return None; }

    #[cfg(not(target_os = "windows"))]
    {
        // lsof output lines look like:
        //   COMMAND  PID  USER  FD TYPE DEVICE SIZE/OFF NODE NAME
        //   nginx    123  root  6u IPv4 0x...  0t0      TCP  *:3000 (LISTEN)
        // The first non-header match is enough to report.
        for line in res.stdout.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 2 {
                return Some(format!("{} (pid {})", fields[0], fields[1]));
            }
        }
        None
    }
    #[cfg(target_os = "windows")]
    {
        // netstat line we care about:
        //   TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       1234
        let needle = format!(":{port_s}");
        for line in res.stdout.lines() {
            if !line.contains("LISTENING") { continue; }
            if !line.contains(&needle) { continue; }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 5 {
                return Some(format!("pid {}", fields[4]));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_df_happy_path() {
        let out = "\
Filesystem   1M-blocks  Used  Available  Capacity  Mounted on
/dev/vda1         2000   800       1200       41%  /
tmpfs              100    10         90       11%  /tmp
";
        assert_eq!(parse_df_available_mb(out), Some(1200));
    }

    #[test]
    fn parse_df_ignores_non_root_rows() {
        let out = "\
Filesystem   1M-blocks  Used  Available  Capacity  Mounted on
tmpfs              100    10         90       11%  /tmp
/dev/vda1         2000   800        300       73%  /
";
        assert_eq!(parse_df_available_mb(out), Some(300));
    }

    #[test]
    fn parse_df_empty_returns_none() {
        assert_eq!(parse_df_available_mb(""), None);
        assert_eq!(parse_df_available_mb("only-one-line"), None);
    }

    #[tokio::test]
    async fn probe_dns_fires_on_failure_output() {
        use crate::sandbox_ops::testing::MockBackend;
        let mock: Arc<dyn SandboxBackend> = Arc::new(
            MockBackend::new("fake").with_stdout("** server can't find github.com: NXDOMAIN\n"),
        );
        let issue = probe_dns(&mock).await.expect("should fire");
        assert_eq!(issue.id, "dns-broken");
        assert_eq!(issue.severity, Severity::Error);
    }

    #[tokio::test]
    async fn probe_dns_silent_on_success() {
        use crate::sandbox_ops::testing::MockBackend;
        let mock: Arc<dyn SandboxBackend> = Arc::new(MockBackend::new("fake").with_stdout(
            "Server:\t1.1.1.1\nAddress:\t1.1.1.1#53\n\nNon-authoritative answer:\nName:\tgithub.com\nAddress: 140.82.121.4\n",
        ));
        assert!(probe_dns(&mock).await.is_none());
    }

    #[tokio::test]
    async fn probe_disk_fires_when_below_threshold() {
        use crate::sandbox_ops::testing::MockBackend;
        let low = "\
Filesystem   1M-blocks  Used  Available  Capacity  Mounted on
/dev/vda1         2000  1900        100       95%  /
";
        let mock: Arc<dyn SandboxBackend> =
            Arc::new(MockBackend::new("fake").with_stdout(low));
        let issue = probe_disk(&mock).await.unwrap();
        assert_eq!(issue.id, "disk-low");
        assert!(issue.message.contains("100 MB"));
    }

    #[tokio::test]
    async fn probe_disk_silent_when_plenty_free() {
        use crate::sandbox_ops::testing::MockBackend;
        let plenty = "\
Filesystem   1M-blocks  Used  Available  Capacity  Mounted on
/dev/vda1         5000  1000       4000       20%  /
";
        let mock: Arc<dyn SandboxBackend> =
            Arc::new(MockBackend::new("fake").with_stdout(plenty));
        assert!(probe_disk(&mock).await.is_none());
    }
}
