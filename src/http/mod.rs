//! The HTTP layer: `POST /webhook/{source}` and a health check.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use crate::app::{App, HandleError};
use crate::sources::{SourceError, WebhookRequest};

/// GitHub caps webhook payloads at 25 MB.
const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/webhook/{source}", post(webhook))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(app)
}

async fn webhook(
    State(app): State<Arc<App>>,
    Path(source): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match app.handle(&source, WebhookRequest { headers, body }).await {
        Ok(_) => StatusCode::ACCEPTED.into_response(),
        Err(HandleError::UnknownSource) => StatusCode::NOT_FOUND.into_response(),
        Err(HandleError::Source(SourceError::Unauthorized)) => {
            tracing::warn!(source, "rejected webhook: invalid authentication");
            (StatusCode::UNAUTHORIZED, "invalid authentication").into_response()
        }
        Err(HandleError::Source(SourceError::Malformed(reason))) => {
            tracing::warn!(source, %reason, "rejected malformed webhook");
            (StatusCode::BAD_REQUEST, reason).into_response()
        }
    }
}
