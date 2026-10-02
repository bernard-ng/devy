//! GitHub webhooks: verified with the HMAC signature, typed with octocrab's models, then mapped
//! into the domain's [`Activity`](crate::domain::github::Activity).

mod mapping;

use async_trait::async_trait;
use octocrab::models::webhook_events::WebhookEvent;

use super::{SourceError, WebhookRequest, WebhookSource, auth};
use crate::config::Secret;
use crate::domain::Notification;

/// Events the bot has an opinion about. Anything else is acknowledged without being parsed, as
/// octocrab's payload models are strict and we'd rather not fail on events we ignore anyway.
const HANDLED_EVENTS: &[&str] = &[
    "ping",
    "push",
    "fork",
    "pull_request",
    "pull_request_review",
    "issues",
    "issue_comment",
    "star",
    "repository",
    "release",
];

pub struct GithubSource {
    secret: Secret,
}

impl GithubSource {
    pub fn new(secret: Secret) -> Self {
        Self { secret }
    }
}

#[async_trait]
impl WebhookSource for GithubSource {
    fn name(&self) -> &'static str {
        "github"
    }

    async fn receive(&self, request: &WebhookRequest) -> Result<Vec<Notification>, SourceError> {
        auth::verify_github_signature(&self.secret, request.header("x-hub-signature-256"), &request.body)?;

        let name = request
            .header("x-github-event")
            .ok_or_else(|| SourceError::Malformed("missing X-GitHub-Event header".into()))?;

        if !HANDLED_EVENTS.contains(&name) {
            tracing::debug!(event = name, "ignoring github event");
            return Ok(vec![]);
        }

        let event = WebhookEvent::try_from_header_and_body(name, &request.body)
            .map_err(|e| SourceError::Malformed(format!("github `{name}` payload: {e}")))?;

        let activity = mapping::to_activity(event).map_err(|e| SourceError::Malformed(e.to_string()))?;
        Ok(activity.map(|a| a.into_notification()).into_iter().collect())
    }
}
