//! End-to-end through the real router: authentication, routing and the Telegram commands.
//! GitHub event wording is covered in `github_events.rs`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Harness, fixture, message, sign};
use devy::domain::Destination;
use tower::ServiceExt;

// ---------------------------------------------------------------- github

// ---------------------------------------------------------------- github

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

    // Valid JSON in a shape octocrab does not know is GitHub's change, not a bad delivery.
    assert_eq!(
        h.github("push", "{\"unexpected\":true}").await,
        StatusCode::ACCEPTED
    );
    assert!(h.sent().is_empty());

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
