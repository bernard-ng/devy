//! Shared test harness: the real router with only Telegram's delivery faked.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use devy::app::App;
use devy::config::Secret;
use devy::domain::Notification;
use devy::domain::message::TextBudget;
use devy::domain::ports::{Notifier, NotifyError};
use devy::http::router;
use devy::sources::Sources;
use devy::sources::github::GithubSource;
use devy::sources::telegram::TelegramSource;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tower::ServiceExt;

pub const GITHUB_SECRET: &str = "gh-secret";
pub const TELEGRAM_SECRET: &str = "tg-secret";

#[derive(Default)]
pub struct Recorder(pub Mutex<Vec<Notification>>);

#[async_trait]
impl Notifier for Recorder {
    async fn notify(&self, notification: Notification) -> Result<(), NotifyError> {
        self.0.lock().unwrap().push(notification);
        Ok(())
    }
}

pub struct Harness {
    pub app: Arc<App>,
    pub router: Router,
    pub recorder: Arc<Recorder>,
}

impl Harness {
    pub fn new() -> Self {
        let recorder = Arc::new(Recorder::default());
        let sources = Sources::new()
            .with(GithubSource::new(
                Secret::new(GITHUB_SECRET),
                TextBudget {
                    excerpt_chars: 1500,
                    max_text_chars: 4096,
                },
            ))
            .with(TelegramSource::new(Secret::new(TELEGRAM_SECRET), "DdevyBot"));
        let app = Arc::new(App::new(sources, recorder.clone()));
        Self {
            router: router(app.clone(), 25 * 1024 * 1024),
            app,
            recorder,
        }
    }

    pub async fn post(&self, uri: &str, headers: &[(&str, &str)], body: &str) -> StatusCode {
        let mut request = Request::post(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        self.app.drain().await;
        response.status()
    }

    pub async fn github(&self, event: &str, body: &str) -> StatusCode {
        let signature = sign(body);
        self.post(
            "/webhook/github",
            &[
                ("x-github-event", event),
                ("x-hub-signature-256", &signature),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await
    }

    pub async fn telegram(&self, body: serde_json::Value) -> StatusCode {
        self.post(
            "/webhook/telegram",
            &[
                ("x-telegram-bot-api-secret-token", TELEGRAM_SECRET),
                ("content-type", "application/json"),
            ],
            &body.to_string(),
        )
        .await
    }

    pub fn sent(&self) -> Vec<Notification> {
        self.recorder.0.lock().unwrap().clone()
    }

    pub fn only(&self) -> Notification {
        let mut sent = self.sent();
        assert_eq!(sent.len(), 1, "expected exactly one notification, got {sent:?}");
        sent.remove(0)
    }
}

pub fn sign(body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(GITHUB_SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/github/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

pub fn message(text: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut message = serde_json::json!({
        "message_id": 77,
        "date": 1_700_000_000,
        "chat": { "id": -100123, "type": "supergroup", "title": "Devy", "is_forum": true },
        "from": { "id": 1, "is_bot": false, "first_name": "Ada" },
        "text": text,
    });
    message
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::json!({ "update_id": 1, "message": message })
}
