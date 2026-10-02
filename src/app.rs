use std::sync::Arc;

use tokio_util::task::TaskTracker;

use crate::domain::Notification;
use crate::domain::ports::Notifier;
use crate::sources::{SourceError, Sources, WebhookRequest};

#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    #[error("no such webhook source")]
    UnknownSource,
    #[error(transparent)]
    Source(#[from] SourceError),
}

/// Wires sources to the notifier. Interpreting a webhook happens inline (so the sender learns
/// about auth/format problems); delivery happens in the background so slow Telegram calls never
/// make the sender time out and retry.
pub struct App {
    sources: Sources,
    notifier: Arc<dyn Notifier>,
    deliveries: TaskTracker,
}

impl App {
    pub fn new(sources: Sources, notifier: Arc<dyn Notifier>) -> Self {
        Self {
            sources,
            notifier,
            deliveries: TaskTracker::new(),
        }
    }

    /// Returns how many notifications were queued.
    pub async fn handle(&self, source: &str, request: WebhookRequest) -> Result<usize, HandleError> {
        let source = self.sources.get(source).ok_or(HandleError::UnknownSource)?;
        let notifications = source.receive(&request).await?;
        let queued = notifications.len();
        for notification in notifications {
            self.deliver(notification);
        }
        Ok(queued)
    }

    fn deliver(&self, notification: Notification) {
        let notifier = Arc::clone(&self.notifier);
        self.deliveries.spawn(async move {
            if let Err(error) = notifier.notify(notification).await {
                tracing::error!(%error, "notification delivery failed");
            }
        });
    }

    /// Waits for in-flight deliveries; call once no more webhooks are accepted.
    pub async fn drain(&self) {
        self.deliveries.close();
        self.deliveries.wait().await;
        self.deliveries.reopen();
    }
}
