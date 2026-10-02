use async_trait::async_trait;

use crate::domain::Notification;

/// Outbound port: something able to deliver a [`Notification`] (Telegram today, anything tomorrow).
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, notification: Notification) -> Result<(), NotifyError>;
}

#[derive(Debug, thiserror::Error)]
#[error("failed to deliver notification: {0}")]
pub struct NotifyError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

impl NotifyError {
    pub fn new(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(source))
    }
}
