//! Which GitHub events and actions are announced, and how loudly. The map of what is kept and
//! what is dropped (and why) lives here; the wording is in `messages`.
//!
//! Dropped on purpose: `create`/`delete` (a `push` already says it), `watch` (duplicates `star`),
//! `check_run`/`check_suite`/`status` (`workflow_run` covers CI), and the noisy actions
//! (synchronize, labeled, edited, ...).

use octocrab::models::webhook_events::payload::*;
use octocrab::models::webhook_events::{WebhookEvent, WebhookEventPayload};
use serde_json::Value;

use super::Ctx;
use super::json::{at, text};
use super::messages::{
    self as m, IssueEvent, PullRequestEvent, ReleaseEvent, RepositoryEvent, ReviewEvent, RunOutcome,
};
use crate::domain::Topic;
use crate::domain::message::Card;
use octocrab::models::pulls::ReviewState;

/// Turns an event into a card, or `None` when it is not worth a message.
pub(super) fn announce(ctx: &Ctx, event: &WebhookEvent) -> Option<Card> {
    let card = match &event.specific {
        WebhookEventPayload::Ping(p) => Some(m::ping(ctx, p.zen.as_deref())),
        WebhookEventPayload::Push(p) => push(ctx, p),
        WebhookEventPayload::PullRequest(p) => pull_request(ctx, p),
        WebhookEventPayload::PullRequestReview(p) => review(ctx, p),
        WebhookEventPayload::Issues(p) => issues(ctx, p),
        WebhookEventPayload::IssueComment(p) => matches!(p.action, IssueCommentWebhookEventAction::Created)
            .then(|| m::comment(ctx, &p.issue, &p.comment)),
        WebhookEventPayload::CommitComment(p) => Some(m::commit_comment(ctx, &p.comment)),
        WebhookEventPayload::Release(p) => release(ctx, p),
        WebhookEventPayload::Discussion(p) => match p.action {
            DiscussionWebhookEventAction::Created => Some(m::discussion(ctx, &p.discussion, false)),
            DiscussionWebhookEventAction::Answered => Some(m::discussion(ctx, &p.discussion, true)),
            _ => None,
        },
        WebhookEventPayload::DiscussionComment(p) => {
            matches!(p.action, DiscussionCommentWebhookEventAction::Created)
                .then(|| m::discussion_comment(ctx, &p.discussion, &p.comment))
        }
        WebhookEventPayload::Milestone(p) => matches!(p.action, MilestoneWebhookEventAction::Closed)
            .then(|| m::milestone_closed(ctx, &p.milestone)),

        WebhookEventPayload::WorkflowRun(p) => workflow_run(ctx, p),
        WebhookEventPayload::WorkflowJob(p) => matches!(p.action, WorkflowJobWebhookEventAction::Waiting)
            .then(|| m::workflow_waiting(ctx, &p.workflow_job)),
        WebhookEventPayload::DeploymentStatus(p) => deployment_status(ctx, p),
        WebhookEventPayload::DeploymentProtectionRule(p) => Some(m::deployment_approval(
            ctx,
            p.environment.as_deref(),
            p.event.as_deref(),
        )),
        WebhookEventPayload::PageBuild(p) => {
            (text(&p.build, "status") == Some("errored")).then(|| m::page_build_failed(ctx, &p.build))
        }
        WebhookEventPayload::Package(p) => matches!(p.action, PackageWebhookEventAction::Published)
            .then(|| m::package_published(ctx, &p.package)),

        WebhookEventPayload::SecretScanningAlert(p) => {
            let (state, loud) = match p.action {
                SecretScanningAlertWebhookEventAction::Created => ("detected", true),
                SecretScanningAlertWebhookEventAction::Reopened => ("reopened", true),
                SecretScanningAlertWebhookEventAction::Resolved => ("resolved", false),
                SecretScanningAlertWebhookEventAction::Revoked => ("revoked", false),
                _ => return None,
            };
            Some(m::secret_alert(ctx, state, loud, &p.alert))
        }
        WebhookEventPayload::DependabotAlert(p) => {
            let (state, loud) = match p.action {
                DependabotAlertWebhookEventAction::Created => ("opened", true),
                DependabotAlertWebhookEventAction::Reopened
                | DependabotAlertWebhookEventAction::AutoReopened => ("reopened", true),
                DependabotAlertWebhookEventAction::Reintroduced => ("reintroduced", true),
                DependabotAlertWebhookEventAction::Fixed => ("fixed", false),
                DependabotAlertWebhookEventAction::Dismissed
                | DependabotAlertWebhookEventAction::AutoDismissed => ("dismissed", false),
                _ => return None,
            };
            Some(m::dependabot_alert(ctx, state, loud, &p.alert))
        }
        WebhookEventPayload::CodeScanningAlert(p) => {
            let (state, loud) = match p.action {
                CodeScanningAlertWebhookEventAction::Created => ("opened", true),
                CodeScanningAlertWebhookEventAction::Reopened
                | CodeScanningAlertWebhookEventAction::ReopenedByUser => ("reopened", true),
                CodeScanningAlertWebhookEventAction::Fixed => ("fixed", false),
                CodeScanningAlertWebhookEventAction::ClosedByUser => ("dismissed", false),
                _ => return None,
            };
            Some(m::code_scanning_alert(ctx, state, loud, &p.alert, &p.r#ref))
        }
        WebhookEventPayload::SecurityAdvisory(p) => {
            matches!(p.action, SecurityAdvisoryWebhookEventAction::Published)
                .then(|| m::security_advisory(ctx, &p.security_advisory))
        }
        WebhookEventPayload::RepositoryAdvisory(p) => {
            let headline = match p.action {
                RepositoryAdvisoryWebhookEventAction::Reported => "Security vulnerability reported",
                RepositoryAdvisoryWebhookEventAction::Published => "Repository advisory published",
                _ => return None,
            };
            Some(m::repository_advisory(ctx, headline, &p.repository_advisory))
        }
        WebhookEventPayload::BranchProtectionRule(p) => {
            let headline = match p.action {
                BranchProtectionRuleWebhookEventAction::Created => "Branch protection added",
                BranchProtectionRuleWebhookEventAction::Edited => "Branch protection changed",
                BranchProtectionRuleWebhookEventAction::Deleted => "Branch protection removed",
                _ => return None,
            };
            Some(m::audit(ctx, headline, &format!("rule for {}", p.rule.name)))
        }
        WebhookEventPayload::DeployKey(p) => {
            let headline = match p.action {
                DeployKeyWebhookEventAction::Created => "Deploy key added",
                DeployKeyWebhookEventAction::Deleted => "Deploy key removed",
                _ => return None,
            };
            Some(m::audit(
                ctx,
                headline,
                text(&p.key, "title").unwrap_or("untitled key"),
            ))
        }
        WebhookEventPayload::Meta(p) => Some(m::hook_deleted(ctx, p.hook_id.0)),
        WebhookEventPayload::PersonalAccessTokenRequest(p) => {
            matches!(p.action, PersonalAccessTokenRequestWebhookEventAction::Created)
                .then(|| m::token_request(ctx, &p.personal_access_token_request))
        }

        WebhookEventPayload::Repository(p) => {
            let event = match p.action {
                RepositoryWebhookEventAction::Created => RepositoryEvent::Created,
                RepositoryWebhookEventAction::Archived => RepositoryEvent::Archived,
                RepositoryWebhookEventAction::Unarchived => RepositoryEvent::Unarchived,
                RepositoryWebhookEventAction::Renamed => RepositoryEvent::Renamed {
                    from: p
                        .changes
                        .as_ref()
                        .and_then(|c| c.repository.as_ref())
                        .and_then(|r| r.name.as_ref())
                        .map(|n| n.from.clone()),
                },
                RepositoryWebhookEventAction::Deleted => RepositoryEvent::Deleted,
                RepositoryWebhookEventAction::Privatized => RepositoryEvent::Privatized,
                RepositoryWebhookEventAction::Publicized => RepositoryEvent::Publicized,
                RepositoryWebhookEventAction::Transferred => RepositoryEvent::Transferred,
                _ => return None,
            };
            Some(m::repository(ctx, event))
        }
        WebhookEventPayload::Member(p) => {
            let headline = match p.action {
                MemberWebhookEventAction::Added => "Collaborator added",
                MemberWebhookEventAction::Removed => "Collaborator removed",
                _ => return None,
            };
            Some(m::member(
                ctx,
                headline,
                text(&p.member, "login").unwrap_or("someone"),
            ))
        }
        WebhookEventPayload::Membership(p) => {
            let headline = match p.action {
                MembershipWebhookEventAction::Added => "Team member added",
                MembershipWebhookEventAction::Removed => "Team member removed",
                _ => return None,
            };
            Some(m::team_member(ctx, headline, &p.member, &p.team))
        }
        WebhookEventPayload::Organization(p) => organization(ctx, p),
        WebhookEventPayload::Team(p) => {
            let headline = match p.action {
                TeamWebhookEventAction::Created => "Team created",
                TeamWebhookEventAction::Deleted => "Team deleted",
                _ => return None,
            };
            Some(m::audit(
                ctx,
                headline,
                text(&p.team, "name").unwrap_or("unnamed team"),
            ))
        }

        WebhookEventPayload::Star(p) => {
            matches!(p.action, StarWebhookEventAction::Created).then(|| m::star(ctx))
        }
        WebhookEventPayload::Fork(p) => Some(m::fork(ctx, &p.forkee)),
        WebhookEventPayload::Sponsorship(p) => {
            let headline = match p.action {
                SponsorshipWebhookEventAction::Created => "New sponsor",
                SponsorshipWebhookEventAction::Cancelled => "Sponsorship cancelled",
                SponsorshipWebhookEventAction::TierChanged => "Sponsorship tier changed",
                _ => return None,
            };
            Some(m::sponsorship(ctx, headline, &p.sponsorship))
        }

        _ => None,
    }?;

    // Bots are chatty; they stay quiet unless the message is addressed to a person.
    Some(if ctx.is_bot && card.topic() != Topic::Notifications {
        card.quiet()
    } else {
        card
    })
}

fn push(ctx: &Ctx, p: &PushWebhookEventPayload) -> Option<Card> {
    let pusher = p.pusher.user.name.as_str();

    if let Some(tag) = p.r#ref.strip_prefix("refs/tags/") {
        return Some(if p.deleted {
            m::ref_deleted(ctx, pusher, "Tag", tag)
        } else {
            m::tag_created(ctx, pusher, tag, p.head_commit.as_ref())
        });
    }

    let branch = p.r#ref.strip_prefix("refs/heads/")?;
    if p.deleted {
        return Some(m::ref_deleted(ctx, pusher, "Branch", branch));
    }
    if p.created {
        return Some(m::branch_created(ctx, pusher, branch, p.commits.len()));
    }
    if p.commits.is_empty() && !p.forced {
        return None;
    }
    let default_branch = ctx.default_branch.as_deref() == Some(branch);
    Some(m::commits_pushed(
        ctx,
        pusher,
        branch,
        &p.commits,
        p.forced,
        default_branch,
        p.compare.as_str(),
    ))
}

fn pull_request(ctx: &Ctx, p: &PullRequestWebhookEventPayload) -> Option<Card> {
    let event = match p.action {
        PullRequestWebhookEventAction::Opened => PullRequestEvent::Opened,
        PullRequestWebhookEventAction::Reopened => PullRequestEvent::Reopened,
        PullRequestWebhookEventAction::ReadyForReview => PullRequestEvent::ReadyForReview,
        PullRequestWebhookEventAction::Closed if p.pull_request.merged_at.is_some() => {
            PullRequestEvent::Merged
        }
        PullRequestWebhookEventAction::Closed => PullRequestEvent::Closed,
        PullRequestWebhookEventAction::ConvertedToDraft => PullRequestEvent::ConvertedToDraft,
        PullRequestWebhookEventAction::AutoMergeEnabled => PullRequestEvent::AutoMerge { enabled: true },
        PullRequestWebhookEventAction::AutoMergeDisabled => PullRequestEvent::AutoMerge { enabled: false },
        PullRequestWebhookEventAction::ReviewRequested => PullRequestEvent::ReviewRequested {
            from: p
                .requested_reviewer
                .as_ref()
                .map(|u| u.login.clone())
                .or_else(|| p.requested_team.as_ref().map(|t| format!("team {}", t.name)))?,
        },
        PullRequestWebhookEventAction::Assigned => PullRequestEvent::Assigned {
            to: p.assignee.as_ref()?.login.clone(),
        },
        _ => return None,
    };
    Some(m::pull_request(ctx, event, &p.pull_request))
}

fn review(ctx: &Ctx, p: &PullRequestReviewWebhookEventPayload) -> Option<Card> {
    let event = match p.action {
        PullRequestReviewWebhookEventAction::Dismissed => ReviewEvent::Dismissed,
        PullRequestReviewWebhookEventAction::Submitted => match p.review.state? {
            ReviewState::Approved => ReviewEvent::Approved,
            ReviewState::ChangesRequested => ReviewEvent::ChangesRequested,
            // A bare "comment" review only wraps line comments, which we do not announce.
            ReviewState::Commented if p.review.body.as_deref().is_some_and(|b| !b.trim().is_empty()) => {
                ReviewEvent::Commented
            }
            _ => return None,
        },
        _ => return None,
    };
    Some(m::review(ctx, event, &p.review, &p.pull_request))
}

fn issues(ctx: &Ctx, p: &IssuesWebhookEventPayload) -> Option<Card> {
    let event = match p.action {
        IssuesWebhookEventAction::Opened => IssueEvent::Opened,
        IssuesWebhookEventAction::Reopened => IssueEvent::Reopened,
        IssuesWebhookEventAction::Closed => IssueEvent::Closed,
        IssuesWebhookEventAction::Transferred => IssueEvent::Transferred,
        IssuesWebhookEventAction::Assigned => IssueEvent::Assigned {
            to: p.assignee.as_ref()?.login.clone(),
        },
        IssuesWebhookEventAction::Deleted => IssueEvent::Deleted,
        IssuesWebhookEventAction::Locked => IssueEvent::Locked,
        IssuesWebhookEventAction::Pinned => IssueEvent::Pinned,
        _ => return None,
    };
    Some(m::issue(ctx, event, &p.issue))
}

/// GitHub fires several actions for one release (created, published, released); only the
/// publication is announced, and a pre-release is announced once, by its own action.
fn release(ctx: &Ctx, p: &ReleaseWebhookEventPayload) -> Option<Card> {
    let event = match p.action {
        ReleaseWebhookEventAction::Published if super::json::flag(&p.release, "prerelease") => return None,
        ReleaseWebhookEventAction::Published => ReleaseEvent::Published,
        ReleaseWebhookEventAction::Prereleased => ReleaseEvent::Prereleased,
        ReleaseWebhookEventAction::Deleted => ReleaseEvent::Deleted,
        ReleaseWebhookEventAction::Unpublished => ReleaseEvent::Unpublished,
        _ => return None,
    };
    Some(m::release(ctx, event, &p.release))
}

/// Something that looks like `v1.2.3` or `2024.05`, which is how releases are tagged.
fn looks_like_a_release(reference: &str) -> bool {
    let version = reference.strip_prefix('v').unwrap_or(reference);
    version.starts_with(|c: char| c.is_ascii_digit())
}

fn workflow_run(ctx: &Ctx, p: &WorkflowRunWebhookEventPayload) -> Option<Card> {
    if !matches!(p.action, WorkflowRunWebhookEventAction::Completed) {
        return None;
    }
    let run = &p.workflow_run;
    let branch = text(run, "head_branch").unwrap_or_default();
    let on_default_branch = ctx.default_branch.as_deref() == Some(branch);
    let is_release = looks_like_a_release(branch);
    let has_pull_request = at(run, "pull_requests")
        .and_then(Value::as_array)
        .is_some_and(|prs| !prs.is_empty());

    let outcome = match text(run, "conclusion")? {
        // Failures on throwaway branches are the author's business, not the team's.
        "failure" if on_default_branch || is_release || has_pull_request => RunOutcome::Failed,
        "timed_out" if on_default_branch || is_release || has_pull_request => RunOutcome::TimedOut,
        "success" if on_default_branch || is_release => RunOutcome::Succeeded,
        "action_required" => RunOutcome::NeedsApproval,
        _ => return None,
    };
    Some(m::workflow_run(ctx, outcome, run))
}

fn deployment_status(ctx: &Ctx, p: &DeploymentStatusWebhookEventPayload) -> Option<Card> {
    let failed = match text(&p.deployment_status, "state")? {
        "success" => false,
        "failure" | "error" => true,
        _ => return None,
    };
    Some(m::deployment(ctx, &p.deployment, &p.deployment_status, failed))
}

fn organization(ctx: &Ctx, p: &OrganizationWebhookEventPayload) -> Option<Card> {
    let who = || {
        p.membership
            .as_ref()
            .and_then(|m| text(m, "user.login"))
            .unwrap_or("someone")
    };
    Some(match p.action {
        OrganizationWebhookEventAction::MemberAdded => m::audit(ctx, "Organization member added", who()),
        OrganizationWebhookEventAction::MemberInvited => m::audit(ctx, "Organization member invited", who()),
        OrganizationWebhookEventAction::MemberRemoved => m::audit(ctx, "Organization member removed", who()),
        OrganizationWebhookEventAction::Deleted => ctx.card(Topic::Notifications, "Organization deleted"),
        OrganizationWebhookEventAction::Renamed => ctx.card(Topic::Notifications, "Organization renamed"),
        _ => return None,
    })
}
