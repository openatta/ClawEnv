//! Integration tests for LocalProcessRunner against fake claw fixtures.

#![cfg(unix)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clawops_core::runners::LocalProcessRunner;
use clawops_core::{
    CancellationToken, CommandError, CommandRunner, CommandSpec, ExecEvent, OutputFormat,
};
use tokio::sync::mpsc;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/fake-claws");
    p.push(name);
    assert!(p.exists(), "missing fixture: {}", p.display());
    p
}

fn spec(name: &str) -> CommandSpec {
    CommandSpec::new(fixture(name).to_string_lossy().to_string(), Vec::<String>::new())
}

async fn drain(mut rx: mpsc::Receiver<ExecEvent>) -> Vec<ExecEvent> {
    let mut out = Vec::new();
    while let Some(e) = rx.recv().await { out.push(e); }
    out
}

#[tokio::test]
async fn exec_captures_stdout() {
    let runner = LocalProcessRunner::new();
    let r = runner.exec(spec("echo-json-lines.sh"), CancellationToken::new()).await.unwrap();
    assert!(r.success());
    assert!(r.stdout.contains(r#""step":"pull""#));
}

#[tokio::test]
async fn exec_separates_stdout_stderr() {
    let runner = LocalProcessRunner::new();
    let r = runner.exec(spec("mixed-stdout-stderr.sh"), CancellationToken::new()).await.unwrap();
    assert!(r.stdout.contains("out-line-1"));
    assert!(r.stderr.contains("err-line-1"));
    assert!(!r.stdout.contains("err-"));
    assert!(!r.stderr.contains("out-"));
}

#[tokio::test]
async fn exec_surfaces_nonzero_exit() {
    let runner = LocalProcessRunner::new();
    let r = runner.exec(spec("exit-nonzero.sh"), CancellationToken::new()).await.unwrap();
    assert_eq!(r.exit_code, 42);
    assert!(!r.success());
}

#[tokio::test]
async fn exec_parses_jsonfinal() {
    let runner = LocalProcessRunner::new();
    let s = spec("echo-json-final.sh").with_output_format(OutputFormat::JsonFinal);
    let r = runner.exec(s, CancellationToken::new()).await.unwrap();
    let v = r.structured.unwrap();
    assert_eq!(v["status"], "success");
}

#[tokio::test]
async fn exec_times_out() {
    let runner = LocalProcessRunner::new();
    let s = spec("sleep-forever.sh").with_timeout(Duration::from_millis(200));
    let r = runner.exec(s, CancellationToken::new()).await.unwrap();
    assert!(r.was_timed_out);
}

#[tokio::test]
async fn exec_cancel_kills_process() {
    let runner = LocalProcessRunner::new();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        c2.cancel();
    });
    let started = Instant::now();
    let r = runner.exec(spec("sleep-forever.sh"), cancel).await.unwrap();
    assert!(r.was_cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn exec_stdin_forwarded() {
    let runner = LocalProcessRunner::new();
    let s = spec("stdin-echo.sh").with_stdin("hello\n");
    let r = runner.exec(s, CancellationToken::new()).await.unwrap();
    assert!(r.stdout.contains("got: hello"));
}

#[tokio::test]
async fn exec_spawn_missing_binary() {
    let runner = LocalProcessRunner::new();
    let s = CommandSpec::new("/nonexistent/xyz-clawops", ["x"]);
    match runner.exec(s, CancellationToken::new()).await {
        Err(CommandError::SpawnFailed { .. }) => {}
        other => panic!("expected SpawnFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn streaming_in_order_with_completed_last() {
    let runner = LocalProcessRunner::new();
    let rx = runner.exec_streaming(spec("echo-json-lines.sh"), CancellationToken::new());
    let events = drain(rx).await;
    assert!(matches!(events.last().unwrap(), ExecEvent::Completed { exit_code: Some(0) }));
}

#[tokio::test]
async fn streaming_emits_structured_progress_for_jsonlines() {
    let runner = LocalProcessRunner::new();
    let s = spec("echo-json-lines.sh").with_output_format(OutputFormat::JsonLines);
    let rx = runner.exec_streaming(s, CancellationToken::new());
    let events = drain(rx).await;
    let structured: Vec<_> = events.iter().filter_map(|e| match e {
        ExecEvent::StructuredProgress(v) => Some(v), _ => None,
    }).collect();
    assert_eq!(structured.len(), 3);
}

#[tokio::test]
async fn streaming_events_gap_proves_streaming() {
    let runner = LocalProcessRunner::new();
    let s = spec("slow-then-line.sh").with_output_format(OutputFormat::JsonLines);
    let mut rx = runner.exec_streaming(s, CancellationToken::new());

    let mut first_at: Option<Instant> = None;
    let mut second_at: Option<Instant> = None;
    while let Some(ev) = rx.recv().await {
        if matches!(ev, ExecEvent::StructuredProgress(_)) {
            if first_at.is_none() { first_at = Some(Instant::now()); }
            else if second_at.is_none() { second_at = Some(Instant::now()); }
        }
    }
    let gap = second_at.unwrap().duration_since(first_at.unwrap());
    assert!(gap >= Duration::from_millis(80), "gap={gap:?}");
}

#[tokio::test]
async fn streaming_cancel_emits_none_exit() {
    let runner = LocalProcessRunner::new();
    let cancel = CancellationToken::new();
    let rx = runner.exec_streaming(spec("sleep-forever.sh"), cancel.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    let events = drain(rx).await;
    assert!(matches!(events.last().unwrap(), ExecEvent::Completed { exit_code: None }));
}

#[tokio::test]
async fn streaming_timeout_emits_none_exit() {
    let runner = LocalProcessRunner::new();
    let s = spec("sleep-forever.sh").with_timeout(Duration::from_millis(200));
    let rx = runner.exec_streaming(s, CancellationToken::new());
    let events = drain(rx).await;
    assert!(matches!(events.last().unwrap(), ExecEvent::Completed { exit_code: None }));
}

#[tokio::test]
async fn streaming_spawn_failure_emits_sentinel() {
    let runner = LocalProcessRunner::new();
    let s = CommandSpec::new("/nonexistent/zzz-clawops", Vec::<String>::new());
    let rx = runner.exec_streaming(s, CancellationToken::new());
    let events = drain(rx).await;
    assert!(events.iter().any(|e|
        matches!(e, ExecEvent::Stderr(l) if l.contains("spawn failed"))
    ));
    assert!(matches!(events.last().unwrap(), ExecEvent::Completed { exit_code: None }));
}
