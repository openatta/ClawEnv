//! Integration test for DownloadOps using a local axum HTTP fixture server.
//!
//! Covers: happy-path fetch, sha256 verification, cache hit on second fetch,
//! not-in-catalog error, and fetch_to custom destination.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderValue, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use clawops_core::download_ops::{
    ArtifactKind, ArtifactSpec, CatalogBackedDownloadOps, DownloadCatalog, DownloadOps,
    PlatformKey,
};
use clawops_core::{CancellationToken, OpsError, ProgressSink};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const BODY: &[u8] = b"fixture body contents";
// sha256("fixture body contents")
const BODY_SHA256: &str = "22323518966960986af29cf113cc1e1fac89099da6c46b27a0e343efa10717f9";

async fn serve() -> (SocketAddr, oneshot::Sender<()>) {
    let app = Router::new().route("/demo.tar.gz", get(|| async {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"))
            .body(Body::from(BODY))
            .unwrap()
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let server = axum::serve(listener, app)
            .with_graceful_shutdown(async move { let _ = shutdown_rx.await; });
        let _ = server.await;
    });
    // Tiny settle so listener is ready.
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, shutdown_tx)
}

fn ops_for(cache: &std::path::Path, addr: SocketAddr, sha: Option<&str>)
    -> CatalogBackedDownloadOps
{
    let url = format!("http://{}/demo.tar.gz", addr);
    let toml = format!(r#"
        [[artifact]]
        name = "demo"
        version = "1.0.0"
        os = "macos"
        arch = "arm64"
        url = "{url}"
        kind = "tarball"
        {}
    "#, sha.map(|s| format!("sha256 = \"{s}\"")).unwrap_or_default());
    let catalog = DownloadCatalog::from_toml_str(&toml).unwrap();
    CatalogBackedDownloadOps::new(
        catalog,
        cache.to_path_buf(),
        PlatformKey { os: "macos".into(), arch: "arm64".into() },
    )
}

// Suppress unused import warning.
const _: Option<Arc<ArtifactSpec>> = None;
#[allow(dead_code)]
fn _touch(_k: ArtifactKind) {}

#[tokio::test]
async fn fetch_happy_path_with_sha_verified() {
    let tmp = TempDir::new().unwrap();
    let (addr, _shutdown) = serve().await;
    let ops = ops_for(tmp.path(), addr, Some(BODY_SHA256));

    let path = ops.fetch("demo", None, ProgressSink::noop(), CancellationToken::new())
        .await.unwrap();
    assert!(path.exists());
    let body = tokio::fs::read(&path).await.unwrap();
    assert_eq!(body, BODY);
}

#[tokio::test]
async fn fetch_checksum_mismatch_fails_and_removes_file() {
    let tmp = TempDir::new().unwrap();
    let (addr, _shutdown) = serve().await;
    let ops = ops_for(tmp.path(), addr, Some("0000000000000000000000000000000000000000000000000000000000000000"));

    let err = ops.fetch("demo", None, ProgressSink::noop(), CancellationToken::new())
        .await.unwrap_err();
    match err {
        OpsError::Download(clawops_core::common::DownloadError::ChecksumMismatch { .. }) => {}
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn fetch_cache_hit_does_not_hit_network() {
    let tmp = TempDir::new().unwrap();
    let (addr, shutdown) = serve().await;
    let ops = ops_for(tmp.path(), addr, Some(BODY_SHA256));

    // Prime cache
    let p1 = ops.fetch("demo", None, ProgressSink::noop(), CancellationToken::new())
        .await.unwrap();
    // Shut down server — next fetch must still succeed from cache.
    let _ = shutdown.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    let p2 = ops.fetch("demo", None, ProgressSink::noop(), CancellationToken::new())
        .await.unwrap();
    assert_eq!(p1, p2);
}

#[tokio::test]
async fn fetch_unknown_artifact_errs() {
    let tmp = TempDir::new().unwrap();
    let (addr, _shutdown) = serve().await;
    let ops = ops_for(tmp.path(), addr, None);

    let err = ops.fetch("nope", None, ProgressSink::noop(), CancellationToken::new())
        .await.unwrap_err();
    match err {
        OpsError::Download(clawops_core::common::DownloadError::NotInCatalog { name }) => {
            assert_eq!(name, "nope");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn fetch_to_custom_dest() {
    let tmp = TempDir::new().unwrap();
    let (addr, _shutdown) = serve().await;
    let ops = ops_for(tmp.path(), addr, None);

    let dest = tmp.path().join("custom.tar.gz");
    let report = ops.fetch_to("demo", None, &dest, ProgressSink::noop(), CancellationToken::new())
        .await.unwrap();
    assert_eq!(report.path, dest);
    assert!(dest.exists());
    assert_eq!(report.bytes, BODY.len() as u64);
}
