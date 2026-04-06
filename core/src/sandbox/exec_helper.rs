//! Unified exec helper for all sandbox backends.
//!
//! Solves the pipe-inheritance problem: VM manager processes (Lima hostagent,
//! WSL2 init, Podman conmon) can inherit stdout/stderr FDs from their parent
//! (limactl/wsl/podman), keeping pipes open indefinitely and causing
//! `.output().await` to hang forever.
//!
//! Solution: redirect command output to temp files inside the sandbox,
//! then read them back. No pipes between host and sandbox.

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::mpsc;

/// Execute a command via a shell wrapper, redirecting output to temp files.
/// `shell_cmd` is the host command (e.g., "limactl shell vm -- sh -c").
/// `shell_args` are the args before the actual command.
/// Returns (stdout, stderr, exit_code).
pub async fn exec_via_tempfile(
    program: &str,
    base_args: &[&str],
    cmd: &str,
) -> Result<(String, String, i32)> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let out_file = format!("/tmp/.clawenv_exec_{stamp}");

    // Wrapper: run command, capture output + exit code to temp files
    let wrapper = format!(
        "({cmd}) > {out_file}.out 2> {out_file}.err; echo $? > {out_file}.rc",
    );

    // Execute with no pipes (prevents hang)
    let mut args: Vec<&str> = base_args.to_vec();
    args.push(&wrapper);

    let status = Command::new(program)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await?;

    // Read results back via separate commands
    let read = |suffix: &str| {
        let file = format!("{out_file}.{suffix}");
        let prog = program.to_string();
        let ba: Vec<String> = base_args.iter().map(|s| s.to_string()).collect();
        async move {
            let mut args: Vec<String> = ba;
            args.push(format!("cat {file}"));
            let out = Command::new(&prog)
                .args(args.iter().map(|s| s.as_str()))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                .await?;
            Ok::<String, anyhow::Error>(String::from_utf8_lossy(&out.stdout).to_string())
        }
    };

    let stdout = read("out").await.unwrap_or_default();
    let stderr = read("err").await.unwrap_or_default();
    let rc_str = read("rc").await.unwrap_or_default();
    let rc: i32 = rc_str.trim().parse().unwrap_or(-1);

    // Cleanup temp files
    let cleanup_cmd = format!("rm -f {out_file}.out {out_file}.err {out_file}.rc");
    let mut cleanup_args: Vec<&str> = base_args.to_vec();
    cleanup_args.push(&cleanup_cmd);
    let _ = Command::new(program)
        .args(&cleanup_args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    Ok((stdout, stderr, rc))
}

/// Stream command output by periodically tailing the temp file.
pub async fn exec_with_progress_via_tempfile(
    program: &str,
    base_args: &[&str],
    cmd: &str,
    tx: &mpsc::Sender<String>,
) -> Result<(String, i32)> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let out_file = format!("/tmp/.clawenv_exec_{stamp}");

    let wrapper = format!(
        "({cmd}) > {out_file}.out 2>&1; echo $? > {out_file}.rc",
    );

    let mut args: Vec<&str> = base_args.to_vec();
    args.push(&wrapper);

    // Start command (no pipes)
    let mut child = Command::new(program)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    // Tail output file periodically
    let prog2 = program.to_string();
    let ba2: Vec<String> = base_args.iter().map(|s| s.to_string()).collect();
    let out_file2 = out_file.clone();
    let tx2 = tx.clone();
    let tail_task = tokio::spawn(async move {
        let mut last_size = 0usize;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let mut tail_args: Vec<String> = ba2.clone();
            tail_args.push(format!("tail -c +{} {}.out 2>/dev/null", last_size + 1, out_file2));
            let result = Command::new(&prog2)
                .args(tail_args.iter().map(|s| s.as_str()))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                .await;
            if let Ok(out) = result {
                let new_content = String::from_utf8_lossy(&out.stdout);
                if !new_content.is_empty() {
                    last_size += new_content.len();
                    for line in new_content.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            let _ = tx2.send(trimmed.to_string()).await;
                        }
                    }
                }
            }
        }
    });

    let status = child.wait().await?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    tail_task.abort();

    // Read final output
    let read_final = |suffix: &str| {
        let file = format!("{out_file}.{suffix}");
        let prog = program.to_string();
        let ba: Vec<String> = base_args.iter().map(|s| s.to_string()).collect();
        async move {
            let mut args: Vec<String> = ba;
            args.push(format!("cat {file}"));
            let out = Command::new(&prog)
                .args(args.iter().map(|s| s.as_str()))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                .await?;
            Ok::<String, anyhow::Error>(String::from_utf8_lossy(&out.stdout).to_string())
        }
    };

    let stdout = read_final("out").await.unwrap_or_default();
    let rc_str = read_final("rc").await.unwrap_or_default();
    let rc: i32 = rc_str.trim().parse().unwrap_or(-1);

    // Send remaining output
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let _ = tx.send(trimmed.to_string()).await;
        }
    }

    // Cleanup
    let cleanup_cmd = format!("rm -f {out_file}.out {out_file}.rc");
    let mut cleanup_args: Vec<&str> = base_args.to_vec();
    cleanup_args.push(&cleanup_cmd);
    let _ = Command::new(program)
        .args(&cleanup_args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    Ok((stdout, rc))
}
