//! Integration test: NativeOps install_component end-to-end.
//!
//! Spins up a local axum server serving a dynamically-built tar.gz, then
//! uses DefaultNativeOps::install_component to download + extract it into
//! a temp dir, verifying the extracted layout.

#![cfg(unix)]

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderValue, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use clawops_core::download_ops::{CatalogBackedDownloadOps, DownloadCatalog, PlatformKey};
use clawops_core::native_ops::{DefaultNativeOps, VersionSpec};
use clawops_core::{CancellationToken, ProgressSink};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

fn make_demo_tar_gz() -> Vec<u8> {
    use std::io::Cursor;
    let buf: Vec<u8> = Vec::new();
    let gz = flate2::write::GzEncoder::new(buf, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);

    let mut header = tar::Header::new_gnu();
    let script = b"#!/bin/sh\necho hello from demo\n";
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "demo-1.0.0/bin/hello", Cursor::new(script.to_vec())).unwrap();

    let mut header = tar::Header::new_gnu();
    let readme = b"demo readme";
    header.set_size(readme.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "demo-1.0.0/README", Cursor::new(readme.to_vec())).unwrap();

    let gz = tar.into_inner().unwrap();
    gz.finish().unwrap()
}

async fn serve(bytes: Vec<u8>) -> (SocketAddr, oneshot::Sender<()>) {
    let shared = std::sync::Arc::new(bytes);
    let app = Router::new().route("/demo.tar.gz", get(move || {
        let shared = shared.clone();
        async move {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, HeaderValue::from_static("application/gzip"))
                .body(Body::from(shared.as_slice().to_vec()))
                .unwrap()
        }
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let s = axum::serve(listener, app)
            .with_graceful_shutdown(async move { let _ = shutdown_rx.await; });
        let _ = s.await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, shutdown_tx)
}

fn catalog_for(url: &str) -> DownloadCatalog {
    let toml = format!(r#"
        [[artifact]]
        name = "demo"
        version = "1.0.0"
        os = "macos"
        arch = "arm64"
        url = "{url}"
        kind = "tarball"

        [[artifact]]
        name = "demo"
        version = "1.0.0"
        os = "linux"
        arch = "x86_64"
        url = "{url}"
        kind = "tarball"

        [[artifact]]
        name = "demo"
        version = "1.0.0"
        os = "linux"
        arch = "arm64"
        url = "{url}"
        kind = "tarball"
    "#);
    DownloadCatalog::from_toml_str(&toml).unwrap()
}

fn platform() -> PlatformKey {
    PlatformKey {
        os: if cfg!(target_os = "macos") { "macos".into() } else { "linux".into() },
        arch: if cfg!(target_arch = "aarch64") { "arm64".into() } else { "x86_64".into() },
    }
}

#[tokio::test]
async fn install_component_downloads_and_extracts_to_target() {
    let tmp = TempDir::new().unwrap();
    let (addr, _shutdown) = serve(make_demo_tar_gz()).await;
    let url = format!("http://{}/demo.tar.gz", addr);

    let downloader = CatalogBackedDownloadOps::new(
        catalog_for(&url),
        tmp.path().join("cache"),
        platform(),
    );
    let ops = DefaultNativeOps::with_downloader(downloader);

    let target = tmp.path().join("install");
    let cancel = CancellationToken::new();
    ops.install_component("demo", VersionSpec::Latest, &target, &ProgressSink::noop(), &cancel)
        .await
        .expect("install_component should succeed");

    // strip_components=1 ⇒ demo-1.0.0/bin/hello becomes target/bin/hello
    let bin = target.join("bin/hello");
    assert!(bin.exists(), "expected extracted binary at {}", bin.display());
    let readme = target.join("README");
    assert!(readme.exists());

    // content sanity
    let content = tokio::fs::read_to_string(&readme).await.unwrap();
    assert_eq!(content, "demo readme");
}

#[tokio::test]
async fn install_component_overwrites_existing() {
    let tmp = TempDir::new().unwrap();
    let (addr, _shutdown) = serve(make_demo_tar_gz()).await;
    let url = format!("http://{}/demo.tar.gz", addr);

    let downloader = CatalogBackedDownloadOps::new(
        catalog_for(&url),
        tmp.path().join("cache"),
        platform(),
    );
    let ops = DefaultNativeOps::with_downloader(downloader);

    let target = tmp.path().join("install");

    // Seed target with an existing (stale) file that should be gone after upgrade
    tokio::fs::create_dir_all(&target).await.unwrap();
    let stale = target.join("STALE");
    tokio::fs::write(&stale, b"stale content").await.unwrap();
    assert!(stale.exists());

    ops.install_component("demo", VersionSpec::Latest, &target, &ProgressSink::noop(),
        &CancellationToken::new()).await.unwrap();

    assert!(!stale.exists(), "stale file should be wiped by upgrade");
    assert!(target.join("README").exists());
    assert!(target.join("bin/hello").exists());
}

#[tokio::test]
async fn install_component_rejects_unknown_artifact() {
    let tmp = TempDir::new().unwrap();
    let (addr, _shutdown) = serve(make_demo_tar_gz()).await;
    let url = format!("http://{}/demo.tar.gz", addr);

    let downloader = CatalogBackedDownloadOps::new(
        catalog_for(&url),
        tmp.path().join("cache"),
        platform(),
    );
    let ops = DefaultNativeOps::with_downloader(downloader);

    let target = tmp.path().join("install");
    let err = ops.install_component("nonexistent", VersionSpec::Latest, &target,
        &ProgressSink::noop(), &CancellationToken::new()).await.unwrap_err();
    match err {
        clawops_core::OpsError::Download(_) => {}
        other => panic!("expected Download err, got {other:?}"),
    }
    assert!(!target.exists(), "should not have created target on lookup failure");
}

// Silence unused import warning when this path helper is not used in asserts.
#[allow(dead_code)]
fn _path_helper(p: &Path) -> &Path { p }
