//! GitHub webhooks: verified with the HMAC signature, typed with octocrab's models, then turned
//! into cards.
//!
//! * [`events`] decides which events and actions are announced, where, and how loudly.
//! * [`messages`] holds all the wording.

mod events;
mod json;
mod messages;

use async_trait::async_trait;
use octocrab::models::webhook_events::WebhookEvent;

use super::{SourceError, WebhookRequest, WebhookSource, auth};
use crate::config::Secret;
use crate::domain::message::{Card, TextBudget};
use crate::domain::{Notification, Topic};

/// Events the bot has an opinion about. Anything else is acknowledged without being parsed, as
/// octocrab's payload models are strict and we'd rather not fail on events we ignore anyway.
/// Keep in sync with `events::announce`.
const HANDLED_EVENTS: &[&str] = &[
    "ping",
    "push",
    "pull_request",
    "pull_request_review",
    "issues",
    "issue_comment",
    "commit_comment",
    "release",
    "discussion",
    "discussion_comment",
    "milestone",
    "workflow_run",
    "workflow_job",
    "deployment_status",
    "deployment_protection_rule",
    "page_build",
    "package",
    "secret_scanning_alert",
    "dependabot_alert",
    "code_scanning_alert",
    "security_advisory",
    "repository_advisory",
    "branch_protection_rule",
    "deploy_key",
    "meta",
    "personal_access_token_request",
    "repository",
    "member",
    "membership",
    "organization",
    "team",
    "star",
    "fork",
    "sponsorship",
];

/// What every message needs to know about where an event happened and who caused it.
pub(crate) struct Ctx {
    /// `owner/repo`, or the organization (or "GitHub") for events that have no repository.
    pub scope: String,
    pub repo_url: Option<String>,
    pub default_branch: Option<String>,
    pub stars: Option<u32>,
    pub actor: String,
    pub is_bot: bool,
}

impl Ctx {
    fn of(event: &WebhookEvent) -> Self {
        let repository = event.repository.as_ref();
        let actor = event
            .sender
            .as_ref()
            .map_or_else(|| "someone".to_owned(), |s| s.login.clone());
        Self {
            scope: repository
                .map(|r| r.full_name.clone().unwrap_or_else(|| r.name.clone()))
                .or_else(|| event.organization.as_ref().map(|o| o.login.clone()))
                .unwrap_or_else(|| "GitHub".to_owned()),
            repo_url: repository
                .and_then(|r| r.html_url.as_ref())
                .map(|u| u.to_string()),
            default_branch: repository.and_then(|r| r.default_branch.clone()),
            stars: repository.and_then(|r| r.stargazers_count),
            is_bot: actor.ends_with("[bot]"),
            actor,
        }
    }

    pub fn card(&self, topic: Topic, headline: impl Into<String>) -> Card {
        Card::new(topic, self.scope.clone(), headline)
    }
}

pub struct GithubSource {
    secret: Secret,
    budget: TextBudget,
}

impl GithubSource {
    pub fn new(secret: Secret, budget: TextBudget) -> Self {
        Self { secret, budget }
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

        let event = match WebhookEvent::try_from_header_and_body(name, &request.body) {
            Ok(event) => event,
            // Well-formed JSON that octocrab's strict models do not accept means GitHub changed a
            // payload. Telling GitHub the delivery failed would not help, so log it loudly instead.
            Err(error) if error.is_data() => {
                tracing::warn!(event = name, %error, "github payload does not match the expected shape");
                return Ok(vec![]);
            }
            Err(error) => {
                return Err(SourceError::Malformed(format!(
                    "github `{name}` payload: {error}"
                )));
            }
        };

        let ctx = Ctx::of(&event);
        Ok(events::announce(&ctx, &event)
            .map(|card| card.build(self.budget))
            .into_iter()
            .collect())
    }
}
