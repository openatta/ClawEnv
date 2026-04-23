//! Integration tests for `LocalProcessRunner`, exercised against fake claw
//! fixture scripts in `core/tests/fixtures/fake-claws/`.
//!
//! These tests do NOT require any VM, any real claw CLI, or any network —
//! they validate the runner's core mechanics (timeout / cancel / streaming /
//! JSON parsing / stdin / stream separation) using shell scripts that
//! simulate claw behaviors.
//!
//! Unix-only; Windows CI is expected to cover the unit tests only. The
//! scripts rely on POSIX sh.

#![cfg(unix)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use clawenv_core::claw_ops::{
    CancellationToken, CommandError, CommandRunner, CommandSpec, ExecEvent, OutputFormat,
};
use clawenv_core::claw_ops::runners::LocalProcessRunner;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/fake-claws");
    p.push(name);
    assert!(p.exists(), "fixture missing: {}", p.display());
    p
}

fn spec_for(name: &str) -> CommandSpec {
    CommandSpec::new(fixture(name).to_string_lossy().to_string(), Vec::<String>::new())
}

async fn drain(mut rx: mpsc::Receiver<ExecEvent>) -> Vec<ExecEvent> {
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev);
    }
    out
}

// ——— exec() path ———

#[tokio::test]
async fn exec_captures_stdout() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("echo-json-lines.sh");
    let result = runner.exec(spec, CancellationToken::new()).await.unwrap();
    assert!(result.success());
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains(r#""step":"pull""#));
    assert!(result.stdout.contains(r#""step":"done""#));
    assert!(result.stderr.is_empty());
    assert!(!result.was_cancelled);
    assert!(!result.was_timed_out);
}

#[tokio::test]
async fn exec_separates_stdout_and_stderr() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("mixed-stdout-stderr.sh");
    let result = runner.exec(spec, CancellationToken::new()).await.unwrap();
    assert!(result.stdout.contains("out-line-1"));
    assert!(result.stdout.contains("out-line-2"));
    assert!(!result.stdout.contains("err-line"));
    assert!(result.stderr.contains("err-line-1"));
    assert!(result.stderr.contains("err-line-2"));
    assert!(!result.stderr.contains("out-line"));
}

#[tokio::test]
async fn exec_surfaces_nonzero_exit() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("exit-nonzero.sh");
    // Non-zero exit is a valid ExecResult, not an Err.
    let result = runner.exec(spec, CancellationToken::new()).await.unwrap();
    assert_eq!(result.exit_code, 42);
    assert!(result.stderr.contains("something went wrong"));
    assert!(!result.success());
}

#[tokio::test]
async fn exec_parses_jsonfinal() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("echo-json-final.sh").with_output_format(OutputFormat::JsonFinal);
    let result = runner.exec(spec, CancellationToken::new()).await.unwrap();
    let v = result.structured.expect("structured output should be parsed");
    assert_eq!(v["status"], "success");
    assert_eq!(v["from_version"], "2026.4.1");
    assert_eq!(v["to_version"], "2026.4.5");
}

#[tokio::test]
async fn exec_times_out_and_kills_process() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("sleep-forever.sh").with_timeout(Duration::from_millis(200));
    let started = Instant::now();
    let result = runner.exec(spec, CancellationToken::new()).await.unwrap();
    assert!(result.was_timed_out, "expected timeout flag");
    assert_eq!(result.exit_code, -1);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(5), "timeout should kill promptly, elapsed={elapsed:?}");
}

#[tokio::test]
async fn exec_cancel_token_stops_process() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("sleep-forever.sh");  // no timeout — rely purely on cancel
    let cancel = CancellationToken::new();
    let cancel2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel2.cancel();
    });
    let started = Instant::now();
    let result = runner.exec(spec, cancel).await.unwrap();
    assert!(result.was_cancelled);
    assert_eq!(result.exit_code, -1);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(5), "cancel should kill promptly, elapsed={elapsed:?}");
}

#[tokio::test]
async fn exec_stdin_is_forwarded() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("stdin-echo.sh").with_stdin("hello from test\n");
    let result = runner.exec(spec, CancellationToken::new()).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("got: hello from test"), "stdout was: {:?}", result.stdout);
}

#[tokio::test]
async fn exec_spawn_failed_for_missing_binary() {
    let runner = LocalProcessRunner::new();
    let spec = CommandSpec::new("/nonexistent/claw-binary-xyz", ["update"]);
    let err = runner.exec(spec, CancellationToken::new()).await.unwrap_err();
    match err {
        CommandError::SpawnFailed { binary, .. } => {
            assert_eq!(binary, "/nonexistent/claw-binary-xyz");
        }
        other => panic!("expected SpawnFailed, got {other:?}"),
    }
}

// ——— exec_streaming() path ———

#[tokio::test]
async fn streaming_emits_stdout_lines_in_order() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("echo-json-lines.sh");
    let rx = runner.exec_streaming(spec, CancellationToken::new());
    let events = drain(rx).await;

    let stdout_lines: Vec<&str> = events.iter().filter_map(|e| match e {
        ExecEvent::Stdout(l) => Some(l.as_str()),
        _ => None,
    }).collect();
    assert_eq!(stdout_lines.len(), 3, "expected 3 stdout lines, got: {events:?}");
    assert!(stdout_lines[0].contains(r#""step":"pull""#));
    assert!(stdout_lines[2].contains(r#""step":"done""#));

    // Completed must be the last event
    let last = events.last().unwrap();
    assert!(matches!(last, ExecEvent::Completed { exit_code: Some(0) }), "last was {last:?}");
}

#[tokio::test]
async fn streaming_jsonlines_emits_structured_progress() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("echo-json-lines.sh").with_output_format(OutputFormat::JsonLines);
    let rx = runner.exec_streaming(spec, CancellationToken::new());
    let events = drain(rx).await;

    let structured: Vec<&serde_json::Value> = events.iter().filter_map(|e| match e {
        ExecEvent::StructuredProgress(v) => Some(v),
        _ => None,
    }).collect();
    assert_eq!(structured.len(), 3);
    assert_eq!(structured[0]["step"], "pull");
    assert_eq!(structured[0]["progress"], 10);
    assert_eq!(structured[2]["step"], "done");
    assert_eq!(structured[2]["progress"], 100);
}

#[tokio::test]
async fn streaming_separates_stdout_stderr() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("mixed-stdout-stderr.sh");
    let rx = runner.exec_streaming(spec, CancellationToken::new());
    let events = drain(rx).await;

    let stdout: Vec<&str> = events.iter().filter_map(|e| match e {
        ExecEvent::Stdout(l) => Some(l.as_str()), _ => None,
    }).collect();
    let stderr: Vec<&str> = events.iter().filter_map(|e| match e {
        ExecEvent::Stderr(l) => Some(l.as_str()), _ => None,
    }).collect();
    assert_eq!(stdout, vec!["out-line-1", "out-line-2"]);
    assert_eq!(stderr, vec!["err-line-1", "err-line-2"]);
}

#[tokio::test]
async fn streaming_events_arrive_before_exit() {
    // Verify the streaming path is *actually* streaming, not batched at end.
    //
    // Absolute arrival time is unreliable under heavy parallel-test scheduling
    // (cold spawn of `sh` can take several seconds on macOS when dozens of
    // tests fork at once). The robust signal is the *gap* between two events:
    // if batched, both arrive within milliseconds of each other at process
    // exit; if streaming, the fixture's 0.2s sleep between echoes produces a
    // clear temporal gap.
    let runner = LocalProcessRunner::new();
    let spec = spec_for("slow-then-line.sh").with_output_format(OutputFormat::JsonLines);
    let mut rx = runner.exec_streaming(spec, CancellationToken::new());

    let mut first_step: Option<String> = None;
    let mut second_step: Option<String> = None;
    let mut first_at: Option<Instant> = None;
    let mut second_at: Option<Instant> = None;

    while let Some(ev) = rx.recv().await {
        if let ExecEvent::StructuredProgress(v) = &ev {
            let step = v["step"].as_str().unwrap_or("").to_string();
            if first_step.is_none() {
                first_step = Some(step);
                first_at = Some(Instant::now());
            } else if second_step.is_none() {
                second_step = Some(step);
                second_at = Some(Instant::now());
            }
        }
    }

    assert_eq!(first_step.as_deref(), Some("first"), "first event's step field");
    assert_eq!(second_step.as_deref(), Some("second"), "second event's step field");
    let gap = second_at.unwrap().duration_since(first_at.unwrap());
    assert!(
        gap >= Duration::from_millis(80),
        "expected ~200ms gap (proving streaming, not batching); observed gap={gap:?}"
    );
}

#[tokio::test]
async fn streaming_cancel_emits_completed_with_none() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("sleep-forever.sh");
    let cancel = CancellationToken::new();
    let rx = runner.exec_streaming(spec, cancel.clone());
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();

    let events = drain(rx).await;
    let last = events.last().expect("should have at least Completed event");
    match last {
        ExecEvent::Completed { exit_code: None } => {}
        other => panic!("expected Completed {{ exit_code: None }}, got {other:?}"),
    }
    // Also should have a stderr sentinel
    assert!(
        events.iter().any(|e| matches!(e, ExecEvent::Stderr(l) if l.contains("cancelled"))),
        "expected cancel sentinel in stderr events: {events:?}"
    );
}

#[tokio::test]
async fn streaming_timeout_emits_completed_with_none() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("sleep-forever.sh").with_timeout(Duration::from_millis(200));
    let rx = runner.exec_streaming(spec, CancellationToken::new());

    let events = drain(rx).await;
    let last = events.last().expect("should have Completed");
    assert!(matches!(last, ExecEvent::Completed { exit_code: None }));
    assert!(
        events.iter().any(|e| matches!(e, ExecEvent::Stderr(l) if l.contains("timed out"))),
        "expected timeout sentinel"
    );
}

#[tokio::test]
async fn streaming_spawn_failure_emits_stderr_and_completed() {
    let runner = LocalProcessRunner::new();
    let spec = CommandSpec::new("/nonexistent/claw-binary-xyz", Vec::<String>::new());
    let rx = runner.exec_streaming(spec, CancellationToken::new());
    let events = drain(rx).await;
    assert!(
        events.iter().any(|e| matches!(e, ExecEvent::Stderr(l) if l.contains("spawn failed"))),
        "expected spawn-failed sentinel: {events:?}"
    );
    assert!(matches!(
        events.last().unwrap(),
        ExecEvent::Completed { exit_code: None }
    ));
}

#[tokio::test]
async fn streaming_nonzero_exit_still_completes_with_code() {
    let runner = LocalProcessRunner::new();
    let spec = spec_for("exit-nonzero.sh");
    let rx = runner.exec_streaming(spec, CancellationToken::new());
    let events = drain(rx).await;
    match events.last().unwrap() {
        ExecEvent::Completed { exit_code: Some(42) } => {}
        other => panic!("expected Completed {{ exit_code: Some(42) }}, got {other:?}"),
    }
}
