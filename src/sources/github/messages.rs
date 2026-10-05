//! All the wording of GitHub notifications. Each function turns one kind of event into a
//! [`Card`]; whether an event is announced at all is decided in `events`.
//!
//! Conventions: the scope line names the repository, the headline says what happened, the
//! subject is the thing it happened to (linked), and the details say who and where. Anything
//! needing a human goes to `notifications`, machine output to `logs`, community life to `general`.

use std::fmt::Debug;

use octocrab::models::Repository;
use octocrab::models::code_scannings::CodeScanningAlert;
use octocrab::models::commits::Comment as CommitComment;
use octocrab::models::issues::{Comment, Issue, IssueStateReason};
use octocrab::models::pulls::{PullRequest, Review};
use octocrab::models::webhook_events::payload::PushWebhookEventCommit;
use serde_json::Value;

use super::Ctx;
use super::json::{number, text};
use crate::domain::Topic;
use crate::domain::message::Card;

/// Commits listed in a push before "and N more".
const LISTED_COMMITS: usize = 3;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn first_line(message: &str) -> &str {
    message.lines().next().unwrap_or_default()
}

fn lower<T: Debug>(value: &T) -> String {
    format!("{value:?}").to_lowercase().replace('_', " ")
}

fn numbered(number: u64, title: &str) -> String {
    format!("#{number} {title}")
}

// ------------------------------------------------------------------ pushes

pub fn commits_pushed(
    ctx: &Ctx,
    pusher: &str,
    branch: &str,
    commits: &[PushWebhookEventCommit],
    forced: bool,
    default_branch: bool,
    compare_url: &str,
) -> Card {
    let headline = match (forced, commits.len()) {
        (true, 0) => format!("Force-pushed to {branch}"),
        (true, n) => format!("Force-pushed {} to {branch}", plural(n, "commit", "commits")),
        (false, n) => format!("{} pushed to {branch}", plural(n, "commit", "commits")),
    };
    let mut card = ctx.card(Topic::Github, headline).detail(format!("by {pusher}"));
    for commit in commits.iter().take(LISTED_COMMITS) {
        card = card.detail(format!(
            "{} {}",
            short_sha(&commit.id),
            first_line(&commit.message)
        ));
    }
    if commits.len() > LISTED_COMMITS {
        card = card.detail(format!("and {} more", commits.len() - LISTED_COMMITS));
    }
    let card = card.action("Compare changes", compare_url);
    if default_branch { card } else { card.quiet() }
}

pub fn branch_created(ctx: &Ctx, pusher: &str, branch: &str, commits: usize) -> Card {
    let mut card = ctx
        .card(Topic::Github, format!("Branch created: {branch}"))
        .detail(format!("by {pusher}"))
        .quiet();
    if commits > 0 {
        card = card.detail(plural(commits, "commit", "commits"));
    }
    if let Some(url) = &ctx.repo_url {
        card = card.action("View branch", format!("{url}/tree/{branch}"));
    }
    card
}

pub fn tag_created(ctx: &Ctx, pusher: &str, tag: &str, head: Option<&PushWebhookEventCommit>) -> Card {
    let mut card = ctx
        .card(Topic::Github, format!("Tag created: {tag}"))
        .detail(format!("by {pusher}"))
        .quiet();
    if let Some(head) = head {
        card = card
            .detail(first_line(&head.message))
            .action("View commit", head.url.as_str());
    }
    card
}

pub fn ref_deleted(ctx: &Ctx, pusher: &str, kind: &str, name: &str) -> Card {
    ctx.card(Topic::Github, format!("{kind} deleted: {name}"))
        .detail(format!("by {pusher}"))
        .quiet()
}

// ----------------------------------------------------------- pull requests

pub enum PullRequestEvent {
    Opened,
    Reopened,
    ReadyForReview,
    Merged,
    Closed,
    ConvertedToDraft,
    AutoMerge { enabled: bool },
    ReviewRequested { from: String },
    Assigned { to: String },
}

fn pr_subject(pr: &PullRequest) -> String {
    numbered(pr.number, pr.title.as_deref().unwrap_or("(untitled)"))
}

fn pr_url(pr: &PullRequest) -> String {
    pr.html_url.as_ref().map(|u| u.to_string()).unwrap_or_default()
}

fn pr_branches(pr: &PullRequest) -> String {
    format!("{} into {}", pr.head.ref_field, pr.base.ref_field)
}

fn pr_size(pr: &PullRequest) -> Option<String> {
    let (additions, deletions) = (pr.additions?, pr.deletions?);
    let files = pr
        .changed_files
        .map(|n| format!(" across {}", plural(n as usize, "file", "files")))
        .unwrap_or_default();
    Some(format!("+{additions} -{deletions}{files}"))
}

pub fn pull_request(ctx: &Ctx, event: PullRequestEvent, pr: &PullRequest) -> Card {
    let author = pr.user.as_ref().map_or("someone", |u| u.login.as_str());
    let topic = match event {
        PullRequestEvent::ReviewRequested { .. } | PullRequestEvent::Assigned { .. } => Topic::Notifications,
        _ => Topic::Github,
    };
    let headline = match &event {
        PullRequestEvent::Opened if pr.draft == Some(true) => "Draft pull request opened",
        PullRequestEvent::Opened => "Pull request opened",
        PullRequestEvent::Reopened => "Pull request reopened",
        PullRequestEvent::ReadyForReview => "Ready for review",
        PullRequestEvent::Merged => "Pull request merged",
        PullRequestEvent::Closed => "Pull request closed",
        PullRequestEvent::ConvertedToDraft => "Converted to draft",
        PullRequestEvent::AutoMerge { enabled: true } => "Auto-merge enabled",
        PullRequestEvent::AutoMerge { enabled: false } => "Auto-merge disabled",
        PullRequestEvent::ReviewRequested { .. } => "Review requested",
        PullRequestEvent::Assigned { .. } => "Pull request assigned",
    };

    let mut card = ctx.card(topic, headline).subject(pr_subject(pr), pr_url(pr));
    card = match &event {
        PullRequestEvent::Opened | PullRequestEvent::Reopened => {
            card.detail(format!("{author} wants to merge {}", pr_branches(pr)))
        }
        PullRequestEvent::ReadyForReview | PullRequestEvent::ConvertedToDraft => {
            card.detail(format!("{} on {}", ctx.actor, pr_branches(pr)))
        }
        PullRequestEvent::Merged => {
            let merger = pr
                .merged_by
                .as_ref()
                .map_or(ctx.actor.as_str(), |u| u.login.as_str());
            card.detail(format!("{merger} merged {}", pr_branches(pr)))
        }
        PullRequestEvent::Closed => card.detail(format!("{} closed it without merging", ctx.actor)),
        PullRequestEvent::AutoMerge { .. } => card.detail(format!("by {}", ctx.actor)),
        PullRequestEvent::ReviewRequested { from } => card.detail(format!(
            "{} asked {from} to review (opened by {author})",
            ctx.actor
        )),
        PullRequestEvent::Assigned { to } => card.detail(format!("{} assigned it to {to}", ctx.actor)),
    };
    let shows_size = matches!(
        event,
        PullRequestEvent::Opened | PullRequestEvent::Merged | PullRequestEvent::ReadyForReview
    );
    if let Some(size) = pr_size(pr).filter(|_| shows_size) {
        card = card.detail(size);
    }
    if matches!(event, PullRequestEvent::Opened) {
        card = card.quote(pr.body.as_deref().unwrap_or_default());
    }
    card
}

pub enum ReviewEvent {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

pub fn review(ctx: &Ctx, event: ReviewEvent, review: &Review, pr: &PullRequest) -> Card {
    let reviewer = review
        .user
        .as_ref()
        .map_or(ctx.actor.as_str(), |u| u.login.as_str());
    let (headline, verb) = match event {
        ReviewEvent::Approved => ("Pull request approved", "approved"),
        ReviewEvent::ChangesRequested => ("Changes requested", "requested changes"),
        ReviewEvent::Commented => ("Pull request reviewed", "left a review"),
        ReviewEvent::Dismissed => ("Review dismissed", "had a review dismissed"),
    };
    let card = ctx
        .card(Topic::Github, headline)
        .subject(pr_subject(pr), review.html_url.as_str())
        .detail(format!("{reviewer} {verb}"))
        .quote(review.body.as_deref().unwrap_or_default());
    if matches!(event, ReviewEvent::Dismissed) {
        card.quiet()
    } else {
        card
    }
}

// ------------------------------------------------------------------ issues

pub enum IssueEvent {
    Opened,
    Reopened,
    Closed,
    Transferred,
    Assigned { to: String },
    Deleted,
    Locked,
    Pinned,
}

pub fn issue(ctx: &Ctx, event: IssueEvent, issue: &Issue) -> Card {
    let topic = match event {
        IssueEvent::Assigned { .. } => Topic::Notifications,
        _ => Topic::Github,
    };
    let headline = match event {
        IssueEvent::Opened => "Issue opened",
        IssueEvent::Reopened => "Issue reopened",
        IssueEvent::Closed => "Issue closed",
        IssueEvent::Transferred => "Issue transferred",
        IssueEvent::Assigned { .. } => "Issue assigned",
        IssueEvent::Deleted => "Issue deleted",
        IssueEvent::Locked => "Issue locked",
        IssueEvent::Pinned => "Issue pinned",
    };
    let mut card = ctx
        .card(topic, headline)
        .subject(numbered(issue.number, &issue.title), issue.html_url.as_str());
    card = match &event {
        IssueEvent::Closed => {
            let how = match issue.state_reason {
                Some(IssueStateReason::NotPlanned) => " as not planned",
                Some(IssueStateReason::Duplicate) => " as a duplicate",
                Some(IssueStateReason::Completed) => " as completed",
                _ => "",
            };
            card.detail(format!("{} closed it{how}", ctx.actor))
        }
        IssueEvent::Assigned { to } => card.detail(format!("{} assigned it to {to}", ctx.actor)),
        IssueEvent::Opened => card.detail(format!("by {}", issue.user.login)),
        _ => card.detail(format!("by {}", ctx.actor)),
    };
    if matches!(event, IssueEvent::Opened) {
        if !issue.labels.is_empty() {
            let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
            card = card.detail(format!("Labels: {}", labels.join(", ")));
        }
        card = card.quote(issue.body.as_deref().unwrap_or_default());
    }
    if matches!(
        event,
        IssueEvent::Deleted | IssueEvent::Locked | IssueEvent::Pinned
    ) {
        card = card.quiet();
    }
    card
}

pub fn comment(ctx: &Ctx, issue: &Issue, comment: &Comment) -> Card {
    let kind = if issue.pull_request.is_some() {
        "pull request"
    } else {
        "issue"
    };
    ctx.card(Topic::Github, format!("Comment on {kind}"))
        .subject(numbered(issue.number, &issue.title), comment.html_url.as_str())
        .detail(format!("by {}", comment.user.login))
        .quote(comment.body.as_deref().unwrap_or_default())
}

pub fn commit_comment(ctx: &Ctx, comment: &CommitComment) -> Card {
    let author = comment.user.as_ref().map_or("someone", |u| u.login.as_str());
    let mut card = ctx
        .card(
            Topic::Github,
            format!("Comment on commit {}", short_sha(&comment.commit_id)),
        )
        .subject("View comment", comment.html_url.as_str())
        .detail(format!("by {author}"));
    if let Some(path) = &comment.path {
        let line = comment.line.map(|l| format!(":{l}")).unwrap_or_default();
        card = card.detail(format!("{path}{line}"));
    }
    card.quote(comment.body.as_deref().unwrap_or_default()).quiet()
}

// ---------------------------------------------------------------- releases

pub enum ReleaseEvent {
    Published,
    Prereleased,
    Deleted,
    Unpublished,
}

pub fn release(ctx: &Ctx, event: ReleaseEvent, release: &Value) -> Card {
    let tag = text(release, "tag_name").unwrap_or("(no tag)");
    let name = text(release, "name").filter(|n| *n != tag);
    let title = match name {
        Some(name) => format!("{name} ({tag})"),
        None => tag.to_owned(),
    };
    let url = text(release, "html_url").unwrap_or_default();
    let headline = match event {
        ReleaseEvent::Published => "Release published",
        ReleaseEvent::Prereleased => "Pre-release published",
        ReleaseEvent::Deleted => "Release deleted",
        ReleaseEvent::Unpublished => "Release unpublished",
    };
    let card = ctx
        .card(Topic::Github, headline)
        .subject(title, url)
        .detail(format!("by {}", ctx.actor));
    match event {
        ReleaseEvent::Published | ReleaseEvent::Prereleased => {
            card.quote(text(release, "body").unwrap_or_default())
        }
        _ => card.quiet(),
    }
}

// ------------------------------------------------------------- discussions

pub fn discussion(ctx: &Ctx, discussion: &Value, answered: bool) -> Card {
    let number = number(discussion, "number").unwrap_or_default();
    let category = text(discussion, "category.name");
    let headline = match (answered, category) {
        (true, _) => "Discussion answered".to_owned(),
        (false, Some(category)) => format!("Discussion opened in {category}"),
        (false, None) => "Discussion opened".to_owned(),
    };
    ctx.card(Topic::General, headline)
        .subject(
            numbered(number, text(discussion, "title").unwrap_or("(untitled)")),
            text(discussion, "html_url").unwrap_or_default(),
        )
        .detail(format!("by {}", ctx.actor))
        .quote(if answered {
            ""
        } else {
            text(discussion, "body").unwrap_or_default()
        })
        .quiet()
}

pub fn discussion_comment(ctx: &Ctx, discussion: &Value, comment: &Value) -> Card {
    ctx.card(Topic::General, "Comment on discussion")
        .subject(
            numbered(
                number(discussion, "number").unwrap_or_default(),
                text(discussion, "title").unwrap_or("(untitled)"),
            ),
            text(comment, "html_url").unwrap_or_default(),
        )
        .detail(format!("by {}", ctx.actor))
        .quote(text(comment, "body").unwrap_or_default())
        .quiet()
}

pub fn milestone_closed(ctx: &Ctx, milestone: &Value) -> Card {
    let closed = number(milestone, "closed_issues").unwrap_or_default();
    let open = number(milestone, "open_issues").unwrap_or_default();
    ctx.card(Topic::Github, "Milestone closed")
        .subject(
            text(milestone, "title").unwrap_or("(untitled)"),
            text(milestone, "html_url").unwrap_or_default(),
        )
        .detail(format!("{closed} closed, {open} still open"))
        .quiet()
}

// ---------------------------------------------------------------------- CI

pub enum RunOutcome {
    Failed,
    TimedOut,
    Succeeded,
    NeedsApproval,
}

pub fn workflow_run(ctx: &Ctx, outcome: RunOutcome, run: &Value) -> Card {
    let name = text(run, "name").unwrap_or("Workflow");
    let branch = text(run, "head_branch").unwrap_or("unknown branch");
    let (topic, headline) = match outcome {
        RunOutcome::Failed => (Topic::Notifications, format!("CI failed on {branch}")),
        RunOutcome::TimedOut => (Topic::Notifications, format!("CI timed out on {branch}")),
        RunOutcome::NeedsApproval => (Topic::Notifications, format!("CI needs approval on {branch}")),
        RunOutcome::Succeeded => (Topic::Logs, format!("CI passed on {branch}")),
    };
    let run_number = number(run, "run_number").unwrap_or_default();
    let attempt = number(run, "run_attempt").filter(|a| *a > 1);
    let mut subject = format!("{name}, run #{run_number}");
    if let Some(attempt) = attempt {
        subject.push_str(&format!(" (attempt {attempt})"));
    }
    let title = text(run, "display_title").unwrap_or_default();
    let who = text(run, "actor.login").unwrap_or(&ctx.actor);
    let mut card = ctx
        .card(topic, headline)
        .subject(subject, text(run, "html_url").unwrap_or_default())
        .detail(format!("{who}: {title}"));
    if matches!(outcome, RunOutcome::Succeeded) {
        card = card.quiet();
    }
    card
}

pub fn workflow_waiting(ctx: &Ctx, job: &Value) -> Card {
    let workflow = text(job, "workflow_name").unwrap_or("Workflow");
    ctx.card(Topic::Notifications, "Waiting for approval")
        .subject(
            format!("{workflow}, {}", text(job, "name").unwrap_or("job")),
            text(job, "html_url").unwrap_or_default(),
        )
        .detail(format!(
            "on {}",
            text(job, "head_branch").unwrap_or("unknown branch")
        ))
}

pub fn deployment(ctx: &Ctx, deployment: &Value, status: &Value, failed: bool) -> Card {
    let environment = text(status, "environment")
        .or_else(|| text(deployment, "environment"))
        .unwrap_or("unknown");
    let (topic, headline) = if failed {
        (
            Topic::Notifications,
            format!("Deployment to {environment} failed"),
        )
    } else {
        (Topic::Logs, format!("Deployed to {environment}"))
    };
    let mut card = ctx
        .card(topic, headline)
        .detail(format!(
            "{} at {}",
            text(deployment, "ref").unwrap_or("unknown ref"),
            short_sha(text(deployment, "sha").unwrap_or_default())
        ))
        .detail(format!(
            "by {}",
            text(status, "creator.login").unwrap_or(&ctx.actor)
        ));
    if let Some(url) = text(status, "log_url").or_else(|| text(status, "target_url")) {
        card = card.action(if failed { "View logs" } else { "View deployment" }, url);
    }
    if !failed {
        card = card.quiet();
    }
    card
}

pub fn deployment_approval(ctx: &Ctx, environment: Option<&str>, event: Option<&str>) -> Card {
    let mut card = ctx.card(
        Topic::Notifications,
        format!(
            "Deployment to {} needs approval",
            environment.unwrap_or("an environment")
        ),
    );
    if let Some(event) = event {
        card = card.detail(format!("triggered by {event}"));
    }
    card
}

pub fn page_build_failed(ctx: &Ctx, build: &Value) -> Card {
    ctx.card(Topic::Logs, "Pages build failed")
        .detail(format!(
            "by {}",
            text(build, "pusher.login").unwrap_or(&ctx.actor)
        ))
        .quote(text(build, "error.message").unwrap_or_default())
        .quiet()
}

pub fn package_published(ctx: &Ctx, package: &Value) -> Card {
    let name = text(package, "name").unwrap_or("package");
    let version = text(package, "package_version.version").unwrap_or_default();
    ctx.card(Topic::Logs, "Package published")
        .subject(
            format!("{name} {version}").trim(),
            text(package, "package_version.html_url")
                .or_else(|| text(package, "html_url"))
                .unwrap_or_default(),
        )
        .detail(format!("by {}", ctx.actor))
        .quiet()
}

// ---------------------------------------------------------------- security

/// `loud` alerts need attention now; the others (fixed, dismissed, resolved) are for the record.
fn alert(card: Card, loud: bool) -> Card {
    if loud { card } else { card.quiet() }
}

pub fn secret_alert(ctx: &Ctx, alert_state: &str, loud: bool, alert_json: &Value) -> Card {
    let kind = text(alert_json, "secret_type_display_name")
        .or_else(|| text(alert_json, "secret_type"))
        .unwrap_or("secret");
    let number = number(alert_json, "number").unwrap_or_default();
    alert(
        ctx.card(Topic::Notifications, format!("Secret alert {alert_state}"))
            .subject(
                format!("{kind}, alert #{number}"),
                text(alert_json, "html_url").unwrap_or_default(),
            )
            .detail(if loud {
                "Revoke the credential, then resolve the alert"
            } else {
                "No action needed"
            }),
        loud,
    )
}

pub fn dependabot_alert(ctx: &Ctx, alert_state: &str, loud: bool, alert_json: &Value) -> Card {
    let severity = text(alert_json, "security_advisory.severity").unwrap_or("unknown severity");
    let package = text(alert_json, "dependency.package.name").unwrap_or("a dependency");
    let ecosystem = text(alert_json, "dependency.package.ecosystem");
    let fix = match text(
        alert_json,
        "security_vulnerability.first_patched_version.identifier",
    ) {
        Some(version) => format!("Fixed in {version}"),
        None => "No patch available yet".to_owned(),
    };
    let package_line = match ecosystem {
        Some(ecosystem) => format!("{package} ({ecosystem})"),
        None => package.to_owned(),
    };
    alert(
        ctx.card(
            Topic::Notifications,
            format!("Dependabot alert {alert_state} ({severity})"),
        )
        .subject(
            text(alert_json, "security_advisory.summary").unwrap_or("Vulnerable dependency"),
            text(alert_json, "html_url").unwrap_or_default(),
        )
        .detail(package_line)
        .detail(fix),
        loud,
    )
}

pub fn code_scanning_alert(
    ctx: &Ctx,
    alert_state: &str,
    loud: bool,
    alert_json: &CodeScanningAlert,
    git_ref: &str,
) -> Card {
    let severity = alert_json
        .rule
        .security_severity_level
        .as_ref()
        .map(lower)
        .or_else(|| alert_json.rule.severity.as_ref().map(lower))
        .unwrap_or_else(|| "unknown severity".to_owned());
    alert(
        ctx.card(
            Topic::Notifications,
            format!("Code scanning alert {alert_state} ({severity})"),
        )
        .subject(&alert_json.rule.description, alert_json.html_url.as_str())
        .detail(format!(
            "{} on {}",
            alert_json.tool.name,
            git_ref.strip_prefix("refs/heads/").unwrap_or(git_ref)
        )),
        loud,
    )
}

pub fn security_advisory(ctx: &Ctx, advisory: &Value) -> Card {
    let ghsa = text(advisory, "ghsa_id").unwrap_or_default();
    ctx.card(
        Topic::Notifications,
        format!(
            "Security advisory published ({})",
            text(advisory, "severity").unwrap_or("unknown severity")
        ),
    )
    .subject(
        text(advisory, "summary").unwrap_or(ghsa),
        format!("https://github.com/advisories/{ghsa}"),
    )
}

pub fn repository_advisory(ctx: &Ctx, headline: &str, advisory: &Value) -> Card {
    ctx.card(
        Topic::Notifications,
        format!(
            "{headline} ({})",
            text(advisory, "severity").unwrap_or("unknown severity")
        ),
    )
    .subject(
        text(advisory, "summary").unwrap_or("Repository security advisory"),
        text(advisory, "html_url").unwrap_or_default(),
    )
    .detail(format!("by {}", ctx.actor))
}

pub fn audit(ctx: &Ctx, headline: impl Into<String>, subject: &str) -> Card {
    ctx.card(Topic::Logs, headline)
        .detail(subject)
        .detail(format!("by {}", ctx.actor))
        .quiet()
}

pub fn hook_deleted(ctx: &Ctx, hook_id: u64) -> Card {
    ctx.card(Topic::Notifications, "Webhook deleted")
        .detail(format!("Hook #{hook_id} was removed by {}", ctx.actor))
        .detail("GitHub no longer sends events through it")
}

pub fn token_request(ctx: &Ctx, request: &Value) -> Card {
    ctx.card(Topic::Notifications, "Token access requested")
        .detail(format!(
            "{} wants access to {}",
            text(request, "owner.login").unwrap_or(&ctx.actor),
            text(request, "repository_selection").unwrap_or("repositories")
        ))
        .quote(text(request, "reason").unwrap_or_default())
}

// -------------------------------------------------------------- repository

pub enum RepositoryEvent {
    Created,
    Archived,
    Unarchived,
    Renamed { from: Option<String> },
    Deleted,
    Privatized,
    Publicized,
    Transferred,
}

pub fn repository(ctx: &Ctx, event: RepositoryEvent) -> Card {
    let (topic, headline) = match event {
        RepositoryEvent::Created => (Topic::Github, "Repository created"),
        RepositoryEvent::Archived => (Topic::Github, "Repository archived"),
        RepositoryEvent::Unarchived => (Topic::Github, "Repository unarchived"),
        RepositoryEvent::Renamed { .. } => (Topic::Github, "Repository renamed"),
        RepositoryEvent::Deleted => (Topic::Notifications, "Repository deleted"),
        RepositoryEvent::Privatized => (Topic::Notifications, "Repository made private"),
        RepositoryEvent::Publicized => (Topic::Notifications, "Repository made public"),
        RepositoryEvent::Transferred => (Topic::Notifications, "Repository transferred"),
    };
    let mut card = ctx.card(topic, headline);
    if let Some(url) = &ctx.repo_url {
        card = card.subject(&ctx.scope, url);
    }
    card = card.detail(format!("by {}", ctx.actor));
    if let RepositoryEvent::Renamed { from: Some(from) } = event {
        card = card.detail(format!("previously {from}"));
    }
    card
}

pub fn ping(ctx: &Ctx, zen: Option<&str>) -> Card {
    let mut card = ctx.card(Topic::Logs, "Webhook connected").quiet();
    if let Some(zen) = zen {
        card = card.detail(zen);
    }
    card
}

// --------------------------------------------------------------- community

/// Stars are quiet noise, except at round numbers worth celebrating.
pub fn is_star_milestone(stars: u32) -> bool {
    matches!(stars, 10 | 25 | 50 | 100 | 250 | 500) || (stars >= 1000 && stars.is_multiple_of(1000))
}

pub fn star(ctx: &Ctx) -> Card {
    let stars = ctx.stars;
    let milestone = stars.is_some_and(is_star_milestone);
    let headline = match stars {
        Some(n) if milestone => format!("Reached {n} stars"),
        _ => "New star".to_owned(),
    };
    let mut card = ctx
        .card(Topic::General, headline)
        .detail(format!("starred by {}", ctx.actor));
    if let (Some(n), false) = (stars, milestone) {
        card = card.detail(plural(n as usize, "star", "stars"));
    }
    if milestone { card } else { card.quiet() }
}

pub fn fork(ctx: &Ctx, forkee: &Repository) -> Card {
    let name = forkee.full_name.clone().unwrap_or_else(|| forkee.name.clone());
    let card = ctx
        .card(Topic::General, "Repository forked")
        .detail(format!("by {}", ctx.actor))
        .quiet();
    match &forkee.html_url {
        Some(url) => card.subject(name, url.as_str()),
        None => card.detail(name),
    }
}

pub fn sponsorship(ctx: &Ctx, headline: &str, sponsorship: &Value) -> Card {
    let sponsor = text(sponsorship, "sponsor.login").unwrap_or(&ctx.actor);
    let mut card = ctx
        .card(Topic::General, headline)
        .detail(format!("from {sponsor}"));
    if let Some(tier) = text(sponsorship, "tier.name") {
        card = card.detail(format!("tier: {tier}"));
    }
    card
}

pub fn member(ctx: &Ctx, headline: &str, login: &str) -> Card {
    audit(ctx, headline, login)
}

pub fn team_member(ctx: &Ctx, headline: &str, member: &Value, team: &Value) -> Card {
    audit(
        ctx,
        headline,
        &format!(
            "{} in {}",
            text(member, "login").unwrap_or("someone"),
            text(team, "name").unwrap_or("a team")
        ),
    )
}
