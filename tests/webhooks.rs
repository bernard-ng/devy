//! End-to-end through the real router: authentication, payload typing, mapping, rendering and
//! routing. Only Telegram's delivery is faked.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use devy::app::App;
use devy::config::Secret;
use devy::domain::ports::{Notifier, NotifyError};
use devy::domain::{Destination, Notification, Topic};
use devy::http::router;
use devy::sources::Sources;
use devy::sources::github::GithubSource;
use devy::sources::telegram::TelegramSource;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tower::ServiceExt;

const GITHUB_SECRET: &str = "gh-secret";
const TELEGRAM_SECRET: &str = "tg-secret";

#[derive(Default)]
struct Recorder(Mutex<Vec<Notification>>);

#[async_trait]
impl Notifier for Recorder {
    async fn notify(&self, notification: Notification) -> Result<(), NotifyError> {
        self.0.lock().unwrap().push(notification);
        Ok(())
    }
}

struct Harness {
    app: Arc<App>,
    router: Router,
    recorder: Arc<Recorder>,
}

impl Harness {
    fn new() -> Self {
        let recorder = Arc::new(Recorder::default());
        let sources = Sources::new()
            .with(GithubSource::new(Secret::new(GITHUB_SECRET), 1500))
            .with(TelegramSource::new(Secret::new(TELEGRAM_SECRET), "DdevyBot"));
        let app = Arc::new(App::new(sources, recorder.clone()));
        Self {
            router: router(app.clone(), 25 * 1024 * 1024),
            app,
            recorder,
        }
    }

    async fn post(&self, uri: &str, headers: &[(&str, &str)], body: &str) -> StatusCode {
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

    async fn github(&self, event: &str, body: &str) -> StatusCode {
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

    async fn telegram(&self, body: serde_json::Value) -> StatusCode {
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

    fn sent(&self) -> Vec<Notification> {
        self.recorder.0.lock().unwrap().clone()
    }

    fn only(&self) -> Notification {
        let mut sent = self.sent();
        assert_eq!(sent.len(), 1, "expected exactly one notification, got {sent:?}");
        sent.remove(0)
    }
}

fn sign(body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(GITHUB_SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/github/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn message(text: &str, extra: serde_json::Value) -> serde_json::Value {
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

// ---------------------------------------------------------------- github

#[tokio::test]
async fn github_ping() {
    let h = Harness::new();
    assert_eq!(h.github("ping", &fixture("ping")).await, StatusCode::ACCEPTED);

    let n = h.only();
    assert_eq!(n.destination, Destination::Topic(Topic::Github));
    assert_eq!(n.text, "👉 Github ping Design for failure.");
    assert!(!n.silent);
}

#[tokio::test]
async fn github_push_of_a_tag() {
    let h = Harness::new();
    assert_eq!(h.github("push", &fixture("push")).await, StatusCode::ACCEPTED);

    let n = h.only();
    assert!(
        n.text
            .starts_with("🔥 gagbo pushed tag v0.2.1 on app-test-repo\n\nMerge pull request #2"),
        "{}",
        n.text
    );
    assert!(
        n.text.ends_with(
            "https://github.com/gagbo/app-test-repo/commit/bd7a63727468ca899e6a43e40fe9d76b3501f3f4"
        )
    );
}

#[tokio::test]
async fn github_issue_opened() {
    let h = Harness::new();
    assert_eq!(
        h.github("issues", &fixture("issues_opened")).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        h.only().text,
        "🔧 gagbo opened an issue\n\nhttps://github.com/gagbo/circadian.nvim/issues/1"
    );
}

#[tokio::test]
async fn github_pull_request_opened_and_closed() {
    let h = Harness::new();
    assert_eq!(
        h.github("pull_request", &fixture("pull_request_opened")).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        h.github("pull_request", &fixture("pull_request_closed")).await,
        StatusCode::ACCEPTED
    );

    let sent = h.sent();
    assert_eq!(sent.len(), 2);
    assert!(
        sent[0]
            .text
            .starts_with("🚀 gagbo opened a pull request\n\nhttps://github.com/")
    );
    // the fixture was closed without merging
    assert!(sent[1].text.starts_with("🚀 gagbo closed a pull request"));
}

#[tokio::test]
async fn github_issue_comment() {
    let h = Harness::new();
    assert_eq!(
        h.github("issue_comment", &fixture("issue_comment_created")).await,
        StatusCode::ACCEPTED
    );

    let text = h.only().text;
    assert!(
        text.starts_with("💬 gagbo-test-app[bot] commented on an issue\n\nJust received an event"),
        "{text}"
    );
    assert!(text.ends_with("#issuecomment-1633968123"));
}

#[tokio::test]
async fn github_repository_deleted() {
    let h = Harness::new();
    assert_eq!(
        h.github("repository", &fixture("repository_deleted")).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        h.only().text,
        "📚 gagbo deleted otp\n\nhttps://github.com/gagbo/otp"
    );
}

#[tokio::test]
async fn github_events_we_do_not_handle_are_acknowledged_untouched() {
    let h = Harness::new();
    assert_eq!(
        h.github("workflow_run", "{\"not\":\"parsed\"}").await,
        StatusCode::ACCEPTED
    );
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn github_actions_we_do_not_announce_are_dropped() {
    let h = Harness::new();
    let mut payload: serde_json::Value = serde_json::from_str(&fixture("pull_request_opened")).unwrap();
    payload["action"] = "synchronize".into();
    assert_eq!(
        h.github("pull_request", &payload.to_string()).await,
        StatusCode::ACCEPTED
    );
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn github_rejects_missing_or_wrong_signatures() {
    let h = Harness::new();
    let body = fixture("ping");
    assert_eq!(
        h.post("/webhook/github", &[("x-github-event", "ping")], &body)
            .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post(
            "/webhook/github",
            &[("x-github-event", "ping"), ("x-hub-signature-256", "sha256=00")],
            &body
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn github_rejects_malformed_deliveries() {
    let h = Harness::new();
    assert_eq!(h.github("push", "not json").await, StatusCode::BAD_REQUEST);

    let body = fixture("ping");
    let headers = [("x-hub-signature-256", sign(&body))];
    assert_eq!(
        h.post(
            "/webhook/github",
            &[("x-hub-signature-256", &headers[0].1)],
            &body
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}

// -------------------------------------------------------------- telegram

#[tokio::test]
async fn telegram_start_replies_in_the_same_topic() {
    let h = Harness::new();
    let update = message(
        "/start",
        serde_json::json!({ "message_thread_id": 4905, "is_topic_message": true }),
    );
    assert_eq!(h.telegram(update).await, StatusCode::ACCEPTED);

    let n = h.only();
    assert_eq!(n.text, "Hello! I'm Devy !");
    match n.destination {
        Destination::Reply(to) => {
            assert_eq!(
                (to.chat_id, to.thread_id, to.message_id),
                (-100123, Some(4905), 77)
            );
        }
        other => panic!("expected a reply, got {other:?}"),
    }
}

#[tokio::test]
async fn telegram_ignores_thread_ids_outside_forum_topics() {
    let h = Harness::new();
    // In plain groups Telegram sets message_thread_id on replies, but it is not a topic.
    let update = message("/start", serde_json::json!({ "message_thread_id": 5 }));
    assert_eq!(h.telegram(update).await, StatusCode::ACCEPTED);

    match h.only().destination {
        Destination::Reply(to) => assert_eq!(to.thread_id, None),
        other => panic!("expected a reply, got {other:?}"),
    }
}

#[tokio::test]
async fn telegram_answers_commands_addressed_to_the_bot() {
    let h = Harness::new();
    assert_eq!(
        h.telegram(message("/start@DdevyBot", serde_json::json!({})))
            .await,
        StatusCode::ACCEPTED
    );
    assert_eq!(h.only().text, "Hello! I'm Devy !");
}

#[tokio::test]
async fn telegram_unknown_commands_get_a_skull() {
    let h = Harness::new();
    assert_eq!(
        h.telegram(message("/nope", serde_json::json!({}))).await,
        StatusCode::ACCEPTED
    );

    let n = h.only();
    assert_eq!(n.text, "💀");
    assert!(n.silent);
}

#[tokio::test]
async fn telegram_help_lists_commands() {
    let h = Harness::new();
    h.telegram(message("/help", serde_json::json!({}))).await;
    let text = h.only().text;
    assert!(text.contains("/start") && text.contains("/help"), "{text}");
}

#[tokio::test]
async fn telegram_stays_quiet_for_other_bots_and_normal_chatter() {
    let h = Harness::new();
    assert_eq!(
        h.telegram(message("/start@SomeoneElseBot", serde_json::json!({})))
            .await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        h.telegram(message("good morning", serde_json::json!({}))).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        h.telegram(serde_json::json!({ "update_id": 2, "poll": null }))
            .await,
        StatusCode::ACCEPTED
    );
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn telegram_rejects_a_wrong_secret() {
    let h = Harness::new();
    let body = message("/start", serde_json::json!({})).to_string();
    assert_eq!(
        h.post(
            "/webhook/telegram",
            &[("x-telegram-bot-api-secret-token", "nope")],
            &body
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post("/webhook/telegram", &[], &body).await,
        StatusCode::UNAUTHORIZED
    );
    assert!(h.sent().is_empty());
}

// ---------------------------------------------------------------- router

#[tokio::test]
async fn unknown_sources_are_404_and_health_is_ok() {
    let h = Harness::new();
    assert_eq!(h.post("/webhook/unknown", &[], "{}").await, StatusCode::NOT_FOUND);

    let response = h
        .router
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
