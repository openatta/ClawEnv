//! Uniform progress channel. UI / CLI / log backends are all interchangeable consumers.

use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct ProgressEvent {
    /// 0..=100 when known.
    pub percent: Option<u8>,
    /// High-level stage label (e.g. "download", "extract", "verify").
    pub stage: String,
    /// Human-readable status line.
    pub message: String,
}

/// Handle for sending progress. Cloneable so multiple producers can publish.
#[derive(Clone)]
pub struct ProgressSink {
    tx: Option<mpsc::Sender<ProgressEvent>>,
}

impl ProgressSink {
    pub fn new(tx: mpsc::Sender<ProgressEvent>) -> Self {
        Self { tx: Some(tx) }
    }

    /// A no-op sink for callers that don't care about progress.
    pub fn noop() -> Self {
        Self { tx: None }
    }

    pub fn is_noop(&self) -> bool {
        self.tx.is_none()
    }

    /// Send an event. Silently drops if sink is noop or receiver was dropped.
    pub async fn send(&self, ev: ProgressEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(ev).await;
        }
    }

    pub async fn info(&self, stage: impl Into<String>, message: impl Into<String>) {
        self.send(ProgressEvent {
            percent: None,
            stage: stage.into(),
            message: message.into(),
        }).await;
    }

    pub async fn at(&self, percent: u8, stage: impl Into<String>, message: impl Into<String>) {
        self.send(ProgressEvent {
            percent: Some(percent),
            stage: stage.into(),
            message: message.into(),
        }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_is_safe() {
        let sink = ProgressSink::noop();
        assert!(sink.is_noop());
    }

    #[tokio::test]
    async fn sink_forwards_to_receiver() {
        let (tx, mut rx) = mpsc::channel(4);
        let sink = ProgressSink::new(tx);
        sink.at(50, "download", "halfway").await;
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.percent, Some(50));
        assert_eq!(ev.stage, "download");
        assert_eq!(ev.message, "halfway");
    }

    #[tokio::test]
    async fn noop_sink_drops_silently() {
        let sink = ProgressSink::noop();
        sink.info("x", "y").await;  // must not panic
    }
}
