//! What each GitHub event turns into: which are announced, where, how loudly, and what they say.
//! Payloads are built on top of real deliveries (the repository and sender come from a fixture),
//! so octocrab's strict models see realistic data.

mod common;

use axum::http::StatusCode;
use common::{Harness, fixture};
use devy::domain::{Destination, Notification, Topic};
use serde_json::{Value, json};

fn load(name: &str) -> Value {
    serde_json::from_str(&fixture(name)).unwrap()
}

/// The parts every repository event carries, taken from a real pull request delivery.
fn envelope() -> Value {
    let real = load("pull_request_opened");
    json!({ "repository": real["repository"], "sender": real["sender"] })
}

/// Recursively overlays `patch` on `base`.
fn merged(mut base: Value, patch: Value) -> Value {
    match (&mut base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                let current = base.remove(&key).unwrap_or(Value::Null);
                base.insert(key, merged(current, value));
            }
            base.clone().into()
        }
        (_, patch) => patch,
    }
}

async fn deliver(event: &str, body: Value) -> Vec<Notification> {
    let h = Harness::new();
    assert_eq!(h.github(event, &body.to_string()).await, StatusCode::ACCEPTED);
    h.sent()
}

async fn announced(event: &str, body: Value) -> Notification {
    let mut sent = deliver(event, body).await;
    assert_eq!(sent.len(), 1, "expected exactly one notification, got {sent:?}");
    sent.remove(0)
}

async fn dropped(event: &str, body: Value) {
    let sent = deliver(event, body).await;
    assert!(sent.is_empty(), "expected nothing, got {sent:?}");
}

fn topic(n: &Notification) -> Topic {
    match n.destination {
        Destination::Topic(topic) => topic,
        other => panic!("expected a topic, got {other:?}"),
    }
}

fn commit(sha: &str, message: &str) -> Value {
    let mut commit = load("push")["head_commit"].clone();
    commit["id"] = sha.into();
    commit["message"] = message.into();
    commit
}

/// A repository payload with the given overrides, e.g. for stars or the default branch.
fn repo(overrides: Value) -> Value {
    json!({ "repository": merged(load("pull_request_opened")["repository"].clone(), overrides) })
}

/// The organization object octocrab requires on org-level events.
fn organization() -> Value {
    let mut org = load("pull_request_opened")["repository"]["owner"].clone();
    for key in ["hooks_url", "issues_url", "members_url", "public_members_url"] {
        org[key] = format!("https://api.github.com/orgs/gagbo/{key}").into();
    }
    org
}

fn user(login: &str) -> Value {
    let mut user = load("pull_request_opened")["sender"].clone();
    user["login"] = login.into();
    user
}

// ------------------------------------------------------------------- basics

#[tokio::test]
async fn ping_confirms_the_webhook_quietly_in_logs() {
    let n = announced("ping", load("ping")).await;
    assert_eq!(topic(&n), Topic::Logs);
    assert!(n.silent);
    assert_eq!(n.text, "<b>GitHub</b> · Webhook connected\nDesign for failure.");
}

#[tokio::test]
async fn events_we_do_not_handle_are_acknowledged_untouched() {
    for event in [
        "check_run",
        "check_suite",
        "status",
        "watch",
        "create",
        "delete",
        "workflow_dispatch",
    ] {
        let h = Harness::new();
        assert_eq!(
            h.github(event, "{\"not\":\"parsed\"}").await,
            StatusCode::ACCEPTED
        );
        assert!(h.sent().is_empty(), "{event}");
    }
}

#[tokio::test]
async fn every_card_names_its_repository() {
    let n = announced("issues", load("issues_opened")).await;
    assert!(
        n.text.starts_with("<b>gagbo/circadian.nvim</b> · Issue opened\n"),
        "{}",
        n.text
    );
}

#[tokio::test]
async fn bots_are_quiet_unless_a_person_is_addressed() {
    let mut payload = load("issue_comment_created");
    payload["sender"]["login"] = "dependabot[bot]".into();
    let n = announced("issue_comment", payload).await;
    assert!(n.silent);

    let mut payload = load("pull_request_opened");
    payload["action"] = "review_requested".into();
    payload["sender"]["login"] = "dependabot[bot]".into();
    payload["requested_reviewer"] = user("bob");
    let n = announced("pull_request", payload).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(!n.silent);
}

// -------------------------------------------------------------------- pushes

#[tokio::test]
async fn push_to_the_default_branch_lists_commits_and_is_loud() {
    let mut p = load("push");
    p["ref"] = "refs/heads/main".into();
    p["created"] = false.into();
    p["commits"] = json!([
        commit("1111111aaaaaaa", "fix: one\n\nlong body"),
        commit("2222222bbbbbbb", "feat: two"),
        commit("3333333ccccccc", "chore: three"),
        commit("4444444ddddddd", "chore: four"),
    ]);
    let n = announced("push", p).await;
    assert_eq!(topic(&n), Topic::Github);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/app-test-repo</b> · 4 commits pushed to main\n\
         by gagbo\n\
         1111111 fix: one\n\
         2222222 feat: two\n\
         3333333 chore: three\n\
         and 1 more\n\
         <a href=\"https://github.com/gagbo/app-test-repo/compare/v0.2.1\">Compare changes</a>"
    );
}

#[tokio::test]
async fn push_to_another_branch_is_quiet_and_a_force_push_says_so() {
    let mut p = load("push");
    p["ref"] = "refs/heads/feature/x".into();
    p["created"] = false.into();
    p["forced"] = true.into();
    p["commits"] = json!([commit("1111111aaaaaaa", "wip")]);
    let n = announced("push", p).await;
    assert!(n.silent);
    assert!(
        n.text.contains("Force-pushed 1 commit to feature/x"),
        "{}",
        n.text
    );
}

#[tokio::test]
async fn pushed_tags_and_branch_changes_are_quiet() {
    let n = announced("push", load("push")).await;
    assert!(n.silent);
    assert!(n.text.contains("Tag created: v0.2.1"), "{}", n.text);
    assert!(n.text.contains("Merge pull request #2 from gagbo/add_slow_route"));
    assert!(!n.text.contains("Add slow route"), "only the subject line");

    let mut p = load("push");
    p["ref"] = "refs/heads/feature/x".into();
    p["created"] = true.into();
    let n = announced("push", p).await;
    assert!(n.silent);
    assert!(n.text.contains("Branch created: feature/x"), "{}", n.text);

    let mut p = load("push");
    p["ref"] = "refs/heads/feature/x".into();
    p["created"] = false.into();
    p["deleted"] = true.into();
    let n = announced("push", p).await;
    assert!(n.silent);
    assert!(n.text.contains("Branch deleted: feature/x"), "{}", n.text);
}

#[tokio::test]
async fn a_push_without_commits_is_not_news() {
    let mut p = load("push");
    p["ref"] = "refs/heads/main".into();
    p["created"] = false.into();
    p["forced"] = false.into();
    p["commits"] = json!([]);
    dropped("push", p).await;
}

// ------------------------------------------------------------- pull requests

#[tokio::test]
async fn pull_request_opened_shows_title_branches_size_and_description() {
    let mut p = load("pull_request_opened");
    p["pull_request"]["body"] = "Adds a retry.".into();
    let n = announced("pull_request", p).await;
    assert_eq!(topic(&n), Topic::Github);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/ouro-closures</b> · Pull request opened\n\
         <a href=\"https://github.com/gagbo/ouro-closures/pull/2\">#2 [do not merge] test commit</a>\n\
         gagbo wants to merge test_pr into trunk\n\
         +1 -0 across 1 file\n\
         <blockquote>Adds a retry.</blockquote>"
    );
}

#[tokio::test]
async fn merged_is_not_reported_as_closed() {
    let mut p = load("pull_request_closed");
    p["pull_request"]["merged_at"] = "2023-07-14T10:00:00Z".into();
    p["pull_request"]["merged_by"] = user("alice");
    let n = announced("pull_request", p).await;
    assert!(n.text.contains("Pull request merged"), "{}", n.text);
    assert!(n.text.contains("alice merged test_pr into trunk"), "{}", n.text);

    let n = announced("pull_request", load("pull_request_closed")).await;
    assert!(n.text.contains("Pull request closed"), "{}", n.text);
    assert!(n.text.contains("gagbo closed it without merging"));
}

#[tokio::test]
async fn review_requests_and_assignments_go_to_notifications() {
    let mut p = load("pull_request_opened");
    p["action"] = "review_requested".into();
    p["requested_reviewer"] = user("bob");
    let n = announced("pull_request", p).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(!n.silent);
    assert!(n.text.contains("Review requested"), "{}", n.text);
    assert!(n.text.contains("gagbo asked bob to review"));

    let mut p = load("pull_request_opened");
    p["action"] = "assigned".into();
    p["assignee"] = user("carol");
    let n = announced("pull_request", p).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(n.text.contains("gagbo assigned it to carol"), "{}", n.text);
}

#[tokio::test]
async fn pull_request_state_changes_that_are_background_noise_are_quiet() {
    for (action, headline) in [
        ("converted_to_draft", "Converted to draft"),
        ("auto_merge_enabled", "Auto-merge enabled"),
    ] {
        let mut p = load("pull_request_opened");
        p["action"] = action.into();
        let n = announced("pull_request", p).await;
        assert!(n.text.contains(headline), "{}", n.text);
    }
}

#[tokio::test]
async fn noisy_pull_request_actions_are_dropped() {
    for action in [
        "synchronize",
        "edited",
        "labeled",
        "milestoned",
        "locked",
        "review_request_removed",
    ] {
        let mut p = load("pull_request_opened");
        p["action"] = action.into();
        dropped("pull_request", p).await;
    }
}

fn review(action: &str, state: &str, body: Value) -> Value {
    let pr = load("pull_request_opened");
    json!({
        "action": action,
        "pull_request": pr["pull_request"],
        "repository": pr["repository"],
        "sender": pr["sender"],
        "review": {
            "id": 1,
            "node_id": "PRR_1",
            "html_url": "https://github.com/gagbo/ouro-closures/pull/2#pullrequestreview-1",
            "user": pr["sender"],
            "body": body,
            "state": state,
            "submitted_at": "2023-07-14T10:00:00Z",
        },
    })
}

#[tokio::test]
async fn reviews_say_what_the_reviewer_decided() {
    let n = announced(
        "pull_request_review",
        review("submitted", "approved", "Ship it".into()),
    )
    .await;
    assert_eq!(topic(&n), Topic::Github);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/ouro-closures</b> · Pull request approved\n\
         <a href=\"https://github.com/gagbo/ouro-closures/pull/2#pullrequestreview-1\">#2 [do not merge] test commit</a>\n\
         gagbo approved\n\
         <blockquote>Ship it</blockquote>"
    );

    let n = announced(
        "pull_request_review",
        review("submitted", "changes_requested", "Needs tests".into()),
    )
    .await;
    assert!(n.text.contains("Changes requested"), "{}", n.text);

    let n = announced(
        "pull_request_review",
        review("dismissed", "dismissed", Value::Null),
    )
    .await;
    assert!(n.silent);
    assert!(n.text.contains("Review dismissed"), "{}", n.text);
}

#[tokio::test]
async fn a_bare_review_comment_is_left_to_its_line_comments() {
    dropped(
        "pull_request_review",
        review("submitted", "commented", Value::Null),
    )
    .await;
    dropped(
        "pull_request_review",
        review("submitted", "commented", "  ".into()),
    )
    .await;
    let n = announced(
        "pull_request_review",
        review("submitted", "commented", "A thought".into()),
    )
    .await;
    assert!(n.text.contains("Pull request reviewed"), "{}", n.text);
}

// -------------------------------------------------------------------- issues

#[tokio::test]
async fn issue_opened_shows_the_title_and_description() {
    let n = announced("issues", load("issues_opened")).await;
    assert_eq!(topic(&n), Topic::Github);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/circadian.nvim</b> · Issue opened\n\
         <a href=\"https://github.com/gagbo/circadian.nvim/issues/1\">#1 Add option to remove the notification</a>\n\
         by gagbo\n\
         <blockquote>The notification can be annoying for non noice users, so it should be an option in setup to remove it.</blockquote>"
    );
}

#[tokio::test]
async fn closing_an_issue_says_why() {
    let mut p = load("issues_opened");
    p["action"] = "closed".into();
    p["issue"]["state_reason"] = "not_planned".into();
    let n = announced("issues", p).await;
    assert!(n.text.contains("Issue closed"), "{}", n.text);
    assert!(n.text.contains("gagbo closed it as not planned"), "{}", n.text);
}

#[tokio::test]
async fn issue_assignment_is_for_notifications_and_housekeeping_is_quiet() {
    let mut p = load("issues_opened");
    p["action"] = "assigned".into();
    p["assignee"] = user("carol");
    let n = announced("issues", p).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(n.text.contains("gagbo assigned it to carol"), "{}", n.text);

    for action in ["locked", "pinned", "deleted"] {
        let mut p = load("issues_opened");
        p["action"] = action.into();
        assert!(announced("issues", p).await.silent, "{action}");
    }
    for action in ["edited", "labeled", "milestoned", "demilestoned", "unassigned"] {
        let mut p = load("issues_opened");
        p["action"] = action.into();
        dropped("issues", p).await;
    }
}

#[tokio::test]
async fn comments_name_the_issue_and_quote_the_text() {
    let n = announced("issue_comment", load("issue_comment_created")).await;
    assert_eq!(topic(&n), Topic::Github);
    assert!(
        n.text.starts_with(
            "<b>gagbo/ouro-closures</b> · Comment on issue\n\
         <a href=\"https://github.com/gagbo/ouro-closures/issues/1#issuecomment-1633968123\">#1 Ping</a>\n\
         by gagbo-test-app[bot]\n"
        ),
        "{}",
        n.text
    );
    assert!(n.text.contains("<blockquote expandable>Just received an event"));
}

#[tokio::test]
async fn comments_on_pull_requests_are_not_called_issues() {
    let mut p = load("issue_comment_created");
    p["issue"]["pull_request"] = json!({
        "url": "https://api.github.com/repos/gagbo/ouro-closures/pulls/1",
        "html_url": "https://github.com/gagbo/ouro-closures/pull/1",
        "diff_url": "https://github.com/gagbo/ouro-closures/pull/1.diff",
        "patch_url": "https://github.com/gagbo/ouro-closures/pull/1.patch",
    });
    let n = announced("issue_comment", p).await;
    assert!(n.text.contains("Comment on pull request"), "{}", n.text);
}

#[tokio::test]
async fn edited_and_deleted_comments_are_dropped() {
    for action in ["edited", "deleted"] {
        let mut p = load("issue_comment_created");
        p["action"] = action.into();
        p["changes"] = json!({ "body": { "from": "before" } });
        dropped("issue_comment", p).await;
    }
}

#[tokio::test]
async fn comment_markup_is_escaped() {
    let mut p = load("issue_comment_created");
    p["comment"]["body"] = "use <b>vec![]</b> & `Option<T>`".into();
    let n = announced("issue_comment", p).await;
    assert!(
        n.text
            .contains("use &lt;b&gt;vec![]&lt;/b&gt; &amp; `Option&lt;T&gt;`"),
        "{}",
        n.text
    );
}

#[tokio::test]
async fn commit_comments_are_quiet() {
    let real = load("pull_request_opened");
    let n = announced(
        "commit_comment",
        merged(
            envelope(),
            json!({
                "action": "created",
                "comment": {
                    "html_url": "https://github.com/gagbo/ouro-closures/commit/abc#commitcomment-1",
                    "url": "https://api.github.com/repos/gagbo/ouro-closures/comments/1",
                    "id": 1,
                    "node_id": "CC_1",
                    "body": "Why this?",
                    "path": "src/lib.rs",
                    "line": 12,
                    "position": 3,
                    "commit_id": "abcdef1234567890",
                    "user": real["sender"],
                    "created_at": "2023-07-14T10:00:00Z",
                    "updated_at": "2023-07-14T10:00:00Z",
                    "author_association": "OWNER",
                },
            }),
        ),
    )
    .await;
    assert!(n.silent);
    assert!(n.text.contains("Comment on commit abcdef1"), "{}", n.text);
    assert!(n.text.contains("src/lib.rs:12"));
}

// ------------------------------------------------------------------ releases

fn release(action: &str, prerelease: bool) -> Value {
    merged(
        envelope(),
        json!({
            "action": action,
            "release": {
                "tag_name": "v1.2.0",
                "name": "Spring release",
                "html_url": "https://github.com/gagbo/ouro-closures/releases/tag/v1.2.0",
                "body": "- faster\n- smaller",
                "prerelease": prerelease,
                "draft": false,
            },
        }),
    )
}

#[tokio::test]
async fn a_published_release_quotes_its_notes() {
    let n = announced("release", release("published", false)).await;
    assert_eq!(topic(&n), Topic::Github);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/ouro-closures</b> · Release published\n\
         <a href=\"https://github.com/gagbo/ouro-closures/releases/tag/v1.2.0\">Spring release (v1.2.0)</a>\n\
         by gagbo\n\
         <blockquote>- faster\n- smaller</blockquote>"
    );
}

#[tokio::test]
async fn one_release_is_announced_once() {
    for action in ["created", "released", "edited"] {
        dropped("release", release(action, false)).await;
    }
    // GitHub publishes a pre-release with both `published` and `prereleased`.
    dropped("release", release("published", true)).await;
    let n = announced("release", release("prereleased", true)).await;
    assert!(n.text.contains("Pre-release published"), "{}", n.text);
}

// ------------------------------------------------------------------------ CI

fn run(conclusion: &str, branch: &str, pull_requests: Value) -> Value {
    merged(
        repo(json!({ "default_branch": "trunk" })),
        json!({
            "action": "completed",
            "sender": load("pull_request_opened")["sender"],
            "workflow_run": {
                "name": "ci",
                "display_title": "fix: retry",
                "html_url": "https://github.com/gagbo/ouro-closures/actions/runs/9",
                "run_number": 9,
                "run_attempt": 2,
                "head_branch": branch,
                "conclusion": conclusion,
                "actor": { "login": "alice" },
                "pull_requests": pull_requests,
            },
        }),
    )
}

#[tokio::test]
async fn a_failed_run_on_the_default_branch_needs_attention() {
    let n = announced("workflow_run", run("failure", "trunk", json!([]))).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/ouro-closures</b> · CI failed on trunk\n\
         <a href=\"https://github.com/gagbo/ouro-closures/actions/runs/9\">ci, run #9 (attempt 2)</a>\n\
         alice: fix: retry"
    );
}

#[tokio::test]
async fn failures_on_pull_requests_and_release_tags_count_but_scratch_branches_do_not() {
    announced(
        "workflow_run",
        run("failure", "feature/x", json!([{ "number": 4 }])),
    )
    .await;
    let n = announced("workflow_run", run("timed_out", "v1.2.0", json!([]))).await;
    assert!(n.text.contains("CI timed out on v1.2.0"), "{}", n.text);
    dropped("workflow_run", run("failure", "feature/x", json!([]))).await;
}

#[tokio::test]
async fn successful_runs_are_logged_quietly_only_where_it_matters() {
    let n = announced("workflow_run", run("success", "trunk", json!([]))).await;
    assert_eq!(topic(&n), Topic::Logs);
    assert!(n.silent);
    assert!(n.text.contains("CI passed on trunk"));
    dropped(
        "workflow_run",
        run("success", "feature/x", json!([{ "number": 4 }])),
    )
    .await;
    dropped("workflow_run", run("cancelled", "trunk", json!([]))).await;
    let n = announced("workflow_run", run("action_required", "feature/x", json!([]))).await;
    assert!(n.text.contains("CI needs approval"), "{}", n.text);
}

#[tokio::test]
async fn runs_that_are_not_finished_are_dropped() {
    for action in ["requested", "in_progress"] {
        let mut p = run("failure", "trunk", json!([]));
        p["action"] = action.into();
        dropped("workflow_run", p).await;
    }
}

#[tokio::test]
async fn a_job_waiting_for_approval_is_announced() {
    let body = merged(
        envelope(),
        json!({
            "action": "waiting",
            "workflow_job": {
                "name": "deploy",
                "workflow_name": "release",
                "html_url": "https://github.com/gagbo/ouro-closures/actions/runs/9/job/1",
                "head_branch": "trunk",
            },
        }),
    );
    let n = announced("workflow_job", body).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(n.text.contains("Waiting for approval"), "{}", n.text);
    assert!(n.text.contains("release, deploy"));

    let mut queued = json!({ "action": "queued", "workflow_job": { "name": "x" } });
    queued = merged(envelope(), queued);
    dropped("workflow_job", queued).await;
}

fn deployment(state: &str) -> Value {
    merged(
        envelope(),
        json!({
            "action": "created",
            "deployment": { "ref": "trunk", "sha": "abcdef1234567", "environment": "production" },
            "deployment_status": {
                "state": state,
                "environment": "production",
                "creator": { "login": "alice" },
                "log_url": "https://github.com/gagbo/ouro-closures/actions/runs/9",
            },
        }),
    )
}

#[tokio::test]
async fn deployments_are_logged_and_failures_escalate() {
    let n = announced("deployment_status", deployment("success")).await;
    assert_eq!(topic(&n), Topic::Logs);
    assert!(n.silent);
    assert!(n.text.contains("Deployed to production"), "{}", n.text);
    assert!(n.text.contains("trunk at abcdef1"));

    let n = announced("deployment_status", deployment("failure")).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(!n.silent);
    assert!(n.text.contains("Deployment to production failed"), "{}", n.text);
    assert!(n.text.contains(">View logs</a>"));

    dropped("deployment_status", deployment("in_progress")).await;
}

#[tokio::test]
async fn deployment_approvals_and_broken_pages_are_announced() {
    let n = announced(
        "deployment_protection_rule",
        merged(
            envelope(),
            json!({ "action": "requested", "environment": "production", "event": "push" }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(
        n.text.contains("Deployment to production needs approval"),
        "{}",
        n.text
    );

    let n = announced(
        "page_build",
        merged(
            envelope(),
            json!({ "id": 1, "build": { "status": "errored", "error": { "message": "Jekyll failed" } } }),
        ),
    )
    .await;
    assert!(n.silent);
    assert!(n.text.contains("Pages build failed"), "{}", n.text);
    dropped(
        "page_build",
        merged(envelope(), json!({ "id": 1, "build": { "status": "built" } })),
    )
    .await;
}

#[tokio::test]
async fn published_packages_are_logged() {
    let n = announced(
        "package",
        merged(
            envelope(),
            json!({
                "action": "published",
                "package": { "name": "devy", "package_version": { "version": "1.2.0", "html_url": "https://x/p" } },
            }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::Logs);
    assert!(n.text.contains(">devy 1.2.0</a>"), "{}", n.text);
}

// ------------------------------------------------------------------ security

#[tokio::test]
async fn a_leaked_secret_is_loud_and_never_quoted() {
    let alert = |action: &str| {
        merged(
            envelope(),
            json!({
                "action": action,
                "alert": {
                    "number": 3,
                    "html_url": "https://github.com/gagbo/ouro-closures/security/secret-scanning/3",
                    "secret_type_display_name": "GitHub Personal Access Token",
                    "secret": "ghp_supersecret",
                },
            }),
        )
    };
    let n = announced("secret_scanning_alert", alert("created")).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(!n.silent);
    assert!(n.text.contains("Secret alert detected"), "{}", n.text);
    assert!(n.text.contains("GitHub Personal Access Token, alert #3"));
    assert!(!n.text.contains("supersecret"));

    assert!(announced("secret_scanning_alert", alert("resolved")).await.silent);
}

fn dependabot(action: &str) -> Value {
    merged(
        envelope(),
        json!({
            "action": action,
            "alert": {
                "number": 7,
                "html_url": "https://github.com/gagbo/ouro-closures/security/dependabot/7",
                "dependency": { "package": { "name": "openssl", "ecosystem": "cargo" } },
                "security_advisory": { "severity": "high", "summary": "Buffer overflow in openssl" },
                "security_vulnerability": { "first_patched_version": { "identifier": "0.10.60" } },
            },
        }),
    )
}

#[tokio::test]
async fn dependabot_alerts_say_what_to_upgrade() {
    let n = announced("dependabot_alert", dependabot("created")).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/ouro-closures</b> · Dependabot alert opened (high)\n\
         <a href=\"https://github.com/gagbo/ouro-closures/security/dependabot/7\">Buffer overflow in openssl</a>\n\
         openssl (cargo)\n\
         Fixed in 0.10.60"
    );
    assert!(announced("dependabot_alert", dependabot("fixed")).await.silent);
    assert!(
        announced("dependabot_alert", dependabot("dismissed"))
            .await
            .silent
    );
}

#[tokio::test]
async fn security_advisories_are_loud() {
    let n = announced(
        "security_advisory",
        merged(
            envelope(),
            json!({
                "action": "published",
                "security_advisory": { "ghsa_id": "GHSA-1", "severity": "critical", "summary": "RCE" },
            }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(
        n.text.contains("Security advisory published (critical)"),
        "{}",
        n.text
    );
    assert!(n.text.contains("https://github.com/advisories/GHSA-1"));
}

#[tokio::test]
async fn a_deleted_webhook_is_loud() {
    let hook = load("ping")["hook"].clone();
    let n = announced(
        "meta",
        merged(
            envelope(),
            json!({ "action": "deleted", "hook_id": 42, "hook": hook }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(n.text.contains("Hook #42 was removed"), "{}", n.text);
}

#[tokio::test]
async fn deploy_keys_are_audited_quietly() {
    let n = announced(
        "deploy_key",
        merged(
            envelope(),
            json!({ "action": "created", "key": { "title": "ci key" } }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::Logs);
    assert!(n.silent);
    assert!(
        n.text.contains("Deploy key added\nci key\nby gagbo"),
        "{}",
        n.text
    );
}

// ---------------------------------------------------------------- repository

#[tokio::test]
async fn risky_repository_changes_are_loud_in_notifications() {
    let n = announced("repository", load("repository_deleted")).await;
    assert_eq!(topic(&n), Topic::Notifications);
    assert!(!n.silent);
    assert_eq!(
        n.text,
        "<b>gagbo/otp</b> · Repository deleted\n<a href=\"https://github.com/gagbo/otp\">gagbo/otp</a>\nby gagbo"
    );

    let mut p = load("repository_deleted");
    p["action"] = "privatized".into();
    let n = announced("repository", p).await;
    assert!(n.text.contains("Repository made private"), "{}", n.text);
}

#[tokio::test]
async fn ordinary_repository_changes_stay_in_github() {
    let mut p = load("repository_deleted");
    p["action"] = "archived".into();
    let n = announced("repository", p).await;
    assert_eq!(topic(&n), Topic::Github);

    let mut p = load("repository_deleted");
    p["action"] = "renamed".into();
    p["changes"] = json!({ "repository": { "name": { "from": "otp-old" } } });
    let n = announced("repository", p).await;
    assert!(n.text.contains("previously otp-old"), "{}", n.text);

    let mut p = load("repository_deleted");
    p["action"] = "edited".into();
    p["changes"] = json!({});
    dropped("repository", p).await;
}

#[tokio::test]
async fn membership_changes_are_audited_in_logs() {
    let n = announced(
        "member",
        merged(
            envelope(),
            json!({ "action": "added", "member": { "login": "carol" } }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::Logs);
    assert!(n.silent);
    assert!(
        n.text.contains("Collaborator added\ncarol\nby gagbo"),
        "{}",
        n.text
    );

    let n = announced(
        "organization",
        json!({
            "action": "member_invited",
            "organization": organization(),
            "sender": load("pull_request_opened")["sender"],
            "membership": { "user": { "login": "dave" } },
        }),
    )
    .await;
    assert!(n.text.contains("Organization member invited\ndave"), "{}", n.text);
    assert!(n.text.starts_with("<b>gagbo</b>"), "{}", n.text);
}

// ----------------------------------------------------------------- community

#[tokio::test]
async fn stars_are_quiet_until_a_milestone() {
    let star = |count: u32, action: &str| {
        merged(
            repo(json!({ "stargazers_count": count })),
            json!({ "action": action, "sender": load("pull_request_opened")["sender"] }),
        )
    };
    let n = announced("star", star(42, "created")).await;
    assert_eq!(topic(&n), Topic::General);
    assert!(n.silent);
    assert!(
        n.text.contains("New star\nstarred by gagbo\n42 stars"),
        "{}",
        n.text
    );

    let n = announced("star", star(100, "created")).await;
    assert!(!n.silent);
    assert!(n.text.contains("Reached 100 stars"), "{}", n.text);

    let n = announced("star", star(3000, "created")).await;
    assert!(n.text.contains("Reached 3000 stars"), "{}", n.text);

    dropped("star", star(41, "deleted")).await;
}

#[tokio::test]
async fn forks_are_quiet() {
    let forkee = load("pull_request_opened")["repository"].clone();
    let n = announced("fork", merged(envelope(), json!({ "forkee": forkee }))).await;
    assert_eq!(topic(&n), Topic::General);
    assert!(n.silent);
    assert!(
        n.text
            .contains("Repository forked\n<a href=\"https://github.com/gagbo/ouro-closures\">"),
        "{}",
        n.text
    );
}

#[tokio::test]
async fn discussions_and_sponsors_are_for_general() {
    let discussion = json!({
        "number": 5,
        "title": "Ideas?",
        "html_url": "https://github.com/gagbo/ouro-closures/discussions/5",
        "category": { "name": "Q&A" },
        "body": "What next?",
    });
    let n = announced(
        "discussion",
        merged(
            envelope(),
            json!({ "action": "created", "discussion": discussion }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::General);
    assert!(n.silent);
    assert!(n.text.contains("Discussion opened in Q&amp;A"), "{}", n.text);
    assert!(n.text.contains("#5 Ideas?"));

    let n = announced(
        "sponsorship",
        merged(
            envelope(),
            json!({
                "action": "created",
                "sponsorship": { "sponsor": { "login": "erin" }, "tier": { "name": "$5 a month" } },
            }),
        ),
    )
    .await;
    assert_eq!(topic(&n), Topic::General);
    assert!(!n.silent);
    assert!(
        n.text.contains("New sponsor\nfrom erin\ntier: $5 a month"),
        "{}",
        n.text
    );

    let n = announced(
        "milestone",
        merged(
            envelope(),
            json!({
                "action": "closed",
                "milestone": { "title": "v1.0", "html_url": "https://x/m", "closed_issues": 14, "open_issues": 0 },
            }),
        ),
    )
    .await;
    assert!(n.text.contains("14 closed, 0 still open"), "{}", n.text);
}
