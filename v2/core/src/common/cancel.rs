//! `CancellationToken` — lightweight cancel signal, no tokio-util dep.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancellationToken {
    pub fn new() -> Self { Self::default() }

    pub fn cancel(&self) {
        let prev = self.inner.cancelled.swap(true, Ordering::SeqCst);
        if !prev {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() { return; }
        let notified = self.inner.notify.notified();
        if self.is_cancelled() { return; }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn new_token_is_not_cancelled() {
        let tok = CancellationToken::new();
        assert!(!tok.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent_and_shared() {
        let a = CancellationToken::new();
        let b = a.clone();
        a.cancel();
        a.cancel();
        assert!(a.is_cancelled());
        assert!(b.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_resolves_immediately_if_pre_cancelled() {
        let tok = CancellationToken::new();
        tok.cancel();
        tokio::time::timeout(Duration::from_millis(100), tok.cancelled())
            .await.expect("should resolve");
    }

    #[tokio::test]
    async fn cancelled_wakes_on_cancel() {
        let tok = CancellationToken::new();
        let tok2 = tok.clone();
        let handle = tokio::spawn(async move { tok2.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        tok.cancel();
        tokio::time::timeout(Duration::from_millis(200), handle)
            .await.expect("wake").expect("task");
    }
}
