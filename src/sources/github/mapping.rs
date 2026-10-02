//! Anti-corruption layer between octocrab's wire models and our [`Activity`].
//!
//! Actions the bot doesn't announce (labels, synchronize, ...) map to `None`.

use octocrab::models::webhook_events::payload::{
    IssueCommentWebhookEventAction as CommentWire, IssuesWebhookEventAction as IssueWire,
    PullRequestReviewWebhookEventAction as ReviewWire, PullRequestWebhookEventAction as PullRequestWire,
    ReleaseWebhookEventAction as ReleaseWire, RepositoryWebhookEventAction as RepositoryWire,
    StarWebhookEventAction as StarWire,
};
use octocrab::models::webhook_events::{WebhookEvent, WebhookEventPayload};

use crate::domain::github::*;

#[derive(Debug, thiserror::Error)]
#[error("payload is missing `{0}`")]
pub struct Missing(&'static str);

pub fn to_activity(event: WebhookEvent) -> Result<Option<Activity>, Missing> {
    let actor = event.sender.as_ref().map(|s| s.login.clone());
    let repository = event.repository.as_ref();
    let repo_name = || repository.map(|r| r.name.clone()).ok_or(Missing("repository"));
    let repo_url = || {
        repository
            .and_then(|r| r.html_url.as_ref())
            .map(|u| u.to_string())
            .ok_or(Missing("repository.html_url"))
    };
    let actor = || actor.clone().ok_or(Missing("sender"));

    let activity = match event.specific {
        WebhookEventPayload::Ping(p) => Activity::Ping { zen: p.zen },

        WebhookEventPayload::Push(p) => Activity::Push {
            pusher: p.pusher.user.name.clone(),
            repository: repo_name()?,
            target: GitRef::parse(&p.r#ref),
            change: if p.deleted {
                PushChange::Deleted
            } else {
                let head = p.head_commit.as_ref();
                PushChange::Pushed {
                    head_message: head.map(|c| c.message.clone()).unwrap_or_default(),
                    url: head.map(|c| c.url.to_string()),
                }
            },
        },

        WebhookEventPayload::Fork(p) => Activity::Fork {
            forker: p
                .forkee
                .owner
                .as_ref()
                .map(|o| o.login.clone())
                .ok_or(Missing("forkee.owner"))?,
            repository: repo_name()?,
            organization: event.organization.as_ref().map(|o| o.login.clone()),
        },

        WebhookEventPayload::PullRequest(p) => {
            let action = match p.action {
                PullRequestWire::Opened => PullRequestAction::Opened,
                PullRequestWire::Closed => PullRequestAction::Closed {
                    merged: p.pull_request.merged_at.is_some(),
                },
                PullRequestWire::Reopened => PullRequestAction::Reopened,
                PullRequestWire::Edited => PullRequestAction::Edited,
                PullRequestWire::ReadyForReview => PullRequestAction::ReadyForReview,
                PullRequestWire::ConvertedToDraft => PullRequestAction::ConvertedToDraft,
                PullRequestWire::Assigned => PullRequestAction::Assigned(
                    p.assignee
                        .as_ref()
                        .map(|a| a.login.clone())
                        .ok_or(Missing("assignee"))?,
                ),
                PullRequestWire::Unassigned => PullRequestAction::Unassigned(
                    p.assignee
                        .as_ref()
                        .map(|a| a.login.clone())
                        .ok_or(Missing("assignee"))?,
                ),
                PullRequestWire::ReviewRequested => PullRequestAction::ReviewRequested {
                    reviewer: p
                        .requested_reviewer
                        .as_ref()
                        .map(|a| a.login.clone())
                        .ok_or(Missing("requested_reviewer"))?,
                },
                PullRequestWire::Milestoned => PullRequestAction::Milestoned(
                    p.milestone
                        .as_ref()
                        .map(|m| m.title.clone())
                        .ok_or(Missing("milestone"))?,
                ),
                _ => return Ok(None),
            };
            Activity::PullRequest {
                actor: actor()?,
                action,
                url: p
                    .pull_request
                    .html_url
                    .as_ref()
                    .map(|u| u.to_string())
                    .ok_or(Missing("pull_request.html_url"))?,
            }
        }

        WebhookEventPayload::PullRequestReview(p) => Activity::Review {
            actor: actor()?,
            repository: repo_name()?,
            action: match p.action {
                ReviewWire::Submitted => ReviewAction::Submitted,
                ReviewWire::Edited => ReviewAction::Edited,
                ReviewWire::Dismissed => ReviewAction::Dismissed,
                _ => return Ok(None),
            },
            body: p.review.body.clone(),
            url: p.review.html_url.to_string(),
        },

        WebhookEventPayload::Issues(p) => Activity::Issue {
            actor: actor()?,
            action: match p.action {
                IssueWire::Opened => IssueAction::Opened,
                IssueWire::Closed => IssueAction::Closed,
                IssueWire::Reopened => IssueAction::Reopened,
                IssueWire::Edited => IssueAction::Edited,
                IssueWire::Locked => IssueAction::Locked,
                IssueWire::Unlocked => IssueAction::Unlocked,
                IssueWire::Deleted => IssueAction::Deleted,
                IssueWire::Transferred => IssueAction::Transferred,
                _ => return Ok(None),
            },
            url: p.issue.html_url.to_string(),
        },

        WebhookEventPayload::IssueComment(p) => Activity::IssueComment {
            actor: actor()?,
            action: match p.action {
                CommentWire::Created => CommentAction::Created,
                CommentWire::Edited => CommentAction::Edited,
                _ => return Ok(None),
            },
            body: p.comment.body.clone().unwrap_or_default(),
            url: p.comment.html_url.to_string(),
        },

        WebhookEventPayload::Star(p) => Activity::Star {
            actor: actor()?,
            repository: repo_name()?,
            stars: repository.and_then(|r| r.stargazers_count).unwrap_or_default(),
            starred: match p.action {
                StarWire::Created => true,
                StarWire::Deleted => false,
                _ => return Ok(None),
            },
        },

        WebhookEventPayload::Repository(p) => Activity::Repository {
            actor: actor()?,
            repository: repo_name()?,
            action: match p.action {
                RepositoryWire::Archived => RepositoryAction::Archived,
                RepositoryWire::Created => RepositoryAction::Created,
                RepositoryWire::Deleted => RepositoryAction::Deleted,
                RepositoryWire::Publicized => RepositoryAction::Publicized,
                RepositoryWire::Privatized => RepositoryAction::Privatized,
                RepositoryWire::Renamed => RepositoryAction::Renamed,
                RepositoryWire::Transferred => RepositoryAction::Transferred,
                RepositoryWire::Unarchived => RepositoryAction::Unarchived,
                _ => return Ok(None),
            },
            url: repo_url()?,
        },

        WebhookEventPayload::Release(p) => {
            let text = |field: &'static str| {
                p.release
                    .get(field)
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .ok_or(Missing(field))
            };
            Activity::Release {
                actor: actor()?,
                repository: repo_name()?,
                tag: text("tag_name")?,
                action: match p.action {
                    ReleaseWire::Published => ReleaseAction::Published,
                    ReleaseWire::Created => ReleaseAction::Created,
                    ReleaseWire::Deleted => ReleaseAction::Deleted,
                    ReleaseWire::Edited => ReleaseAction::Edited,
                    ReleaseWire::Released => ReleaseAction::Released,
                    ReleaseWire::Prereleased => ReleaseAction::Prereleased,
                    ReleaseWire::Unpublished => ReleaseAction::Unpublished,
                    _ => return Ok(None),
                },
                url: text("html_url")?,
            }
        }

        _ => return Ok(None),
    };

    Ok(Some(activity))
}
