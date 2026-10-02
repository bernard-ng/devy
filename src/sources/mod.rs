//! Inbound adapters. A [`WebhookSource`] authenticates one kind of webhook delivery and turns it
//! into zero or more [`Notification`]s. To add an integration: implement the trait, register it in
//! [`Sources`], and it is served at `POST /webhook/{name}`.

pub mod auth;
pub mod github;
pub mod telegram;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::HeaderMap;

use crate::domain::Notification;

/// A raw webhook delivery, before any interpretation.
#[derive(Debug, Clone)]
pub struct WebhookRequest {
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl WebhookRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SourceError {
    #[error("invalid authentication")]
    Unauthorized,
    #[error("malformed webhook: {0}")]
    Malformed(String),
}

#[async_trait]
pub trait WebhookSource: Send + Sync {
    /// URL segment: the source is served at `/webhook/{name}`.
    fn name(&self) -> &'static str;

    /// Authenticates and interprets a delivery. Deliveries that are valid but uninteresting
    /// yield an empty list.
    async fn receive(&self, request: &WebhookRequest) -> Result<Vec<Notification>, SourceError>;
}

#[derive(Default)]
pub struct Sources {
    by_name: HashMap<&'static str, Arc<dyn WebhookSource>>,
}

impl Sources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, source: impl WebhookSource + 'static) -> Self {
        let previous = self.by_name.insert(source.name(), Arc::new(source));
        assert!(previous.is_none(), "two webhook sources share a name");
        self
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn WebhookSource>> {
        self.by_name.get(name)
    }
}
