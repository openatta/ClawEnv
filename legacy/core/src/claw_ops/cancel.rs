//! `CancellationToken` — 轻量取消信号，不引入 tokio-util 依赖。
//!
//! 线程安全、可 Clone（内部 Arc），可在 runner 和 UI 按钮之间共享。

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
    pub fn new() -> Self {
        Self::default()
    }

    /// 触发取消。幂等：多次调用无副作用。
    pub fn cancel(&self) {
        let prev = self.inner.cancelled.swap(true, Ordering::SeqCst);
        if !prev {
            // 首次取消时唤醒所有等待者。
            self.inner.notify.notify_waiters();
        }
    }

    /// 是否已取消。
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// 等待取消信号。如果已经取消，立即返回。
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        // 先订阅 Notify 再次检查，避免 cancel() 发生在 is_cancelled() 与
        // notified() 之间的窗口里丢失唤醒。
        let notified = self.inner.notify.notified();
        if self.is_cancelled() {
            return;
        }
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
    fn cancel_sets_flag() {
        let tok = CancellationToken::new();
        tok.cancel();
        assert!(tok.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let tok = CancellationToken::new();
        tok.cancel();
        tok.cancel();
        assert!(tok.is_cancelled());
    }

    #[test]
    fn clones_share_state() {
        let a = CancellationToken::new();
        let b = a.clone();
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_returns_immediately_if_already_cancelled() {
        let tok = CancellationToken::new();
        tok.cancel();
        // 应立即 resolve，不阻塞
        tokio::time::timeout(Duration::from_millis(100), tok.cancelled())
            .await
            .expect("cancelled() should resolve immediately when pre-cancelled");
    }

    #[tokio::test]
    async fn cancelled_wakes_up_on_cancel() {
        let tok = CancellationToken::new();
        let tok2 = tok.clone();
        let handle = tokio::spawn(async move { tok2.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        tok.cancel();
        tokio::time::timeout(Duration::from_millis(200), handle)
            .await
            .expect("waiter should wake within 200ms")
            .expect("task should not panic");
    }
}
