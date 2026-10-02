//! What happens on GitHub, as the bot understands it.
//!
//! [`Activity`] is our own model, decoupled from octocrab's wire types: the source maps payloads
//! into it (dropping what we don't care about), and rendering it into a [`Notification`] is the
//! only place that knows the bot's wording.

use crate::domain::notification::excerpt;
use crate::domain::{Notification, Topic};

/// Longest user-written body quoted in a message; leaves room for the surrounding text and link.
const BODY_EXCERPT_CHARS: usize = 1500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity {
    Ping {
        zen: Option<String>,
    },
    Push {
        pusher: String,
        repository: String,
        target: GitRef,
        change: PushChange,
    },
    Fork {
        forker: String,
        repository: String,
        organization: Option<String>,
    },
    PullRequest {
        actor: String,
        action: PullRequestAction,
        url: String,
    },
    Review {
        actor: String,
        repository: String,
        action: ReviewAction,
        body: Option<String>,
        url: String,
    },
    Issue {
        actor: String,
        action: IssueAction,
        url: String,
    },
    IssueComment {
        actor: String,
        action: CommentAction,
        body: String,
        url: String,
    },
    Star {
        actor: String,
        repository: String,
        stars: u32,
        starred: bool,
    },
    Repository {
        actor: String,
        repository: String,
        action: RepositoryAction,
        url: String,
    },
    Release {
        actor: String,
        repository: String,
        tag: String,
        action: ReleaseAction,
        url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRef {
    Branch(String),
    Tag(String),
    Other(String),
}

impl GitRef {
    pub fn parse(full: &str) -> Self {
        if let Some(branch) = full.strip_prefix("refs/heads/") {
            Self::Branch(branch.to_owned())
        } else if let Some(tag) = full.strip_prefix("refs/tags/") {
            Self::Tag(tag.to_owned())
        } else {
            Self::Other(full.to_owned())
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Branch(name) | Self::Other(name) => name.clone(),
            Self::Tag(name) => format!("tag {name}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushChange {
    Deleted,
    Pushed {
        head_message: String,
        url: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestAction {
    Opened,
    Closed { merged: bool },
    Reopened,
    Edited,
    ReadyForReview,
    ConvertedToDraft,
    Assigned(String),
    Unassigned(String),
    ReviewRequested { reviewer: String },
    Milestoned(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewAction {
    Submitted,
    Edited,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAction {
    Opened,
    Closed,
    Reopened,
    Edited,
    Locked,
    Unlocked,
    Deleted,
    Transferred,
}

impl IssueAction {
    fn verb(self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::Closed => "closed",
            Self::Reopened => "reopened",
            Self::Edited => "edited",
            Self::Locked => "locked",
            Self::Unlocked => "unlocked",
            Self::Deleted => "deleted",
            Self::Transferred => "transferred",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentAction {
    Created,
    Edited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryAction {
    Archived,
    Created,
    Deleted,
    Publicized,
    Privatized,
    Renamed,
    Transferred,
    Unarchived,
}

impl RepositoryAction {
    fn verb(self) -> &'static str {
        match self {
            Self::Archived => "archived",
            Self::Created => "created",
            Self::Deleted => "deleted",
            Self::Publicized => "publicized",
            Self::Privatized => "privatized",
            Self::Renamed => "renamed",
            Self::Transferred => "transferred",
            Self::Unarchived => "unarchived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseAction {
    Published,
    Created,
    Deleted,
    Edited,
    Released,
    Prereleased,
    Unpublished,
}

impl ReleaseAction {
    fn verb(self) -> &'static str {
        match self {
            Self::Published => "published",
            Self::Created => "created",
            Self::Deleted => "deleted",
            Self::Edited => "edited",
            Self::Released => "released",
            Self::Prereleased => "prereleased",
            Self::Unpublished => "unpublished",
        }
    }

    fn emoji(self) -> &'static str {
        match self {
            Self::Released | Self::Prereleased => "🥳🥳🥳",
            _ => "🔖",
        }
    }
}

impl Activity {
    /// Renders the activity for the GitHub topic. Noisy, low-signal activity is delivered silently.
    pub fn into_notification(self) -> Notification {
        let silent = matches!(self, Self::Fork { .. } | Self::Star { .. });
        let notification = Notification::to_topic(Topic::Github, self.render());
        if silent {
            notification.silent()
        } else {
            notification
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Ping { zen } => format!("👉 Github ping {}", zen.as_deref().unwrap_or_default())
                .trim_end()
                .to_owned(),

            Self::Push {
                pusher,
                repository,
                target,
                change,
            } => match change {
                PushChange::Deleted => {
                    format!("🔥 {pusher} deleted {} on {repository}", target.label())
                }
                PushChange::Pushed { head_message, url } => {
                    let mut text = format!(
                        "🔥 {pusher} pushed {} on {repository}\n\n{}",
                        target.label(),
                        excerpt(head_message, BODY_EXCERPT_CHARS)
                    );
                    if let Some(url) = url {
                        text.push_str(&format!("\n\n{url}"));
                    }
                    text
                }
            },

            Self::Fork {
                forker,
                repository,
                organization,
            } => match organization {
                Some(org) => format!("🍴 {forker} forked {repository} from {org}"),
                None => format!("🍴 {forker} forked {repository}"),
            },

            Self::PullRequest { actor, action, url } => {
                let headline = match action {
                    PullRequestAction::Opened => format!("{actor} opened a pull request"),
                    PullRequestAction::Closed { merged: true } => format!("{actor} merged a pull request"),
                    PullRequestAction::Closed { merged: false } => format!("{actor} closed a pull request"),
                    PullRequestAction::Reopened => format!("{actor} reopened a pull request"),
                    PullRequestAction::Edited => format!("{actor} edited a pull request"),
                    PullRequestAction::ReadyForReview => "a pull request is ready for review".to_owned(),
                    PullRequestAction::ConvertedToDraft => "a pull request was converted to draft".to_owned(),
                    PullRequestAction::Assigned(who) => format!("a pull request was assigned to {who}"),
                    PullRequestAction::Unassigned(who) => format!("a pull request was unassigned from {who}"),
                    PullRequestAction::ReviewRequested { reviewer } => {
                        format!("{reviewer}, {actor} requested you to review a pull request")
                    }
                    PullRequestAction::Milestoned(title) => {
                        format!("a pull request was added to milestone {title}")
                    }
                };
                format!("🚀 {headline}\n\n{url}")
            }

            Self::Review {
                actor,
                repository,
                action,
                body,
                url,
            } => match action {
                ReviewAction::Submitted | ReviewAction::Edited => {
                    let verb = if *action == ReviewAction::Submitted {
                        "submitted"
                    } else {
                        "edited"
                    };
                    let mut text = format!("👀 {actor} {verb} a pull request review");
                    if let Some(body) = body.as_deref().filter(|b| !b.trim().is_empty()) {
                        text.push_str(&format!("\n\n{}", excerpt(body, BODY_EXCERPT_CHARS)));
                    }
                    text.push_str(&format!("\n\n{url}"));
                    text
                }
                ReviewAction::Dismissed => {
                    format!("👀 {actor} dismissed a pull request review\n\n{repository}")
                }
            },

            Self::Issue { actor, action, url } => {
                format!("🔧 {actor} {} an issue\n\n{url}", action.verb())
            }

            Self::IssueComment {
                actor,
                action,
                body,
                url,
            } => {
                let verb = match action {
                    CommentAction::Created => "commented on",
                    CommentAction::Edited => "edited a comment on",
                };
                format!(
                    "💬 {actor} {verb} an issue\n\n{}\n\n{url}",
                    excerpt(body, BODY_EXCERPT_CHARS)
                )
            }

            Self::Star {
                actor,
                repository,
                stars,
                starred,
            } => {
                if *starred {
                    format!("👍 {actor} starred {repository} ({stars} stars)")
                } else {
                    format!("👎 {actor} unstarred {repository} ({stars} stars)")
                }
            }

            Self::Repository {
                actor,
                repository,
                action,
                url,
            } => {
                format!("📚 {actor} {} {repository}\n\n{url}", action.verb())
            }

            Self::Release {
                actor,
                repository,
                tag,
                action,
                url,
            } => {
                format!(
                    "{} {actor} {} {tag} on {repository}\n\n{url}",
                    action.emoji(),
                    action.verb()
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(activity: Activity) -> String {
        activity.into_notification().text
    }

    #[test]
    fn push_to_a_branch() {
        let t = text(Activity::Push {
            pusher: "bernard-ng".into(),
            repository: "devy".into(),
            target: GitRef::parse("refs/heads/main"),
            change: PushChange::Pushed {
                head_message: "fix: things".into(),
                url: Some("https://x/c".into()),
            },
        });
        assert_eq!(
            t,
            "🔥 bernard-ng pushed main on devy\n\nfix: things\n\nhttps://x/c"
        );
    }

    #[test]
    fn deleted_tag_is_labelled_as_a_tag() {
        let t = text(Activity::Push {
            pusher: "bernard-ng".into(),
            repository: "devy".into(),
            target: GitRef::parse("refs/tags/v1"),
            change: PushChange::Deleted,
        });
        assert_eq!(t, "🔥 bernard-ng deleted tag v1 on devy");
    }

    #[test]
    fn merged_pull_request_is_not_reported_as_closed() {
        let t = text(Activity::PullRequest {
            actor: "a".into(),
            action: PullRequestAction::Closed { merged: true },
            url: "https://x/pr/1".into(),
        });
        assert_eq!(t, "🚀 a merged a pull request\n\nhttps://x/pr/1");
    }

    #[test]
    fn review_request_names_both_people() {
        let t = text(Activity::PullRequest {
            actor: "alice".into(),
            action: PullRequestAction::ReviewRequested {
                reviewer: "bob".into(),
            },
            url: "u".into(),
        });
        assert_eq!(t, "🚀 bob, alice requested you to review a pull request\n\nu");
    }

    #[test]
    fn empty_review_body_is_skipped() {
        let t = text(Activity::Review {
            actor: "a".into(),
            repository: "r".into(),
            action: ReviewAction::Submitted,
            body: Some("  ".into()),
            url: "u".into(),
        });
        assert_eq!(t, "👀 a submitted a pull request review\n\nu");
    }

    #[test]
    fn stars_and_forks_are_silent_everything_else_is_not() {
        let star = Activity::Star {
            actor: "a".into(),
            repository: "r".into(),
            stars: 3,
            starred: true,
        };
        let fork = Activity::Fork {
            forker: "a".into(),
            repository: "r".into(),
            organization: None,
        };
        let issue = Activity::Issue {
            actor: "a".into(),
            action: IssueAction::Opened,
            url: "u".into(),
        };
        assert!(star.into_notification().silent);
        assert!(fork.into_notification().silent);
        assert!(!issue.into_notification().silent);
    }

    #[test]
    fn release_celebrates() {
        let t = text(Activity::Release {
            actor: "a".into(),
            repository: "r".into(),
            tag: "v1.0.0".into(),
            action: ReleaseAction::Released,
            url: "u".into(),
        });
        assert_eq!(t, "🥳🥳🥳 a released v1.0.0 on r\n\nu");
    }

    #[test]
    fn long_comments_are_cut_but_keep_their_link() {
        let t = text(Activity::IssueComment {
            actor: "a".into(),
            action: CommentAction::Created,
            body: "x".repeat(10_000),
            url: "https://x/c".into(),
        });
        assert!(t.chars().count() < 1700);
        assert!(t.ends_with("\n\nhttps://x/c"));
    }

    #[test]
    fn fork_without_organization() {
        let t = text(Activity::Fork {
            forker: "a".into(),
            repository: "r".into(),
            organization: None,
        });
        assert_eq!(t, "🍴 a forked r");
    }
}
