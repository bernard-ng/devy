//! How a notification looks: one layout for every message, so wording changes stay local and
//! user-written text can never break the markup.
//!
//! Notifications are Telegram HTML. [`Card`] takes raw text, escapes it and enforces the size
//! limits *before* adding tags, because a message cut in the middle of a tag is rejected.

use crate::domain::{Notification, Topic};

/// Longest single-line field (subject, detail) before it is cut.
const LINE_MAX_CHARS: usize = 200;
/// Quotes longer than this are collapsed behind an "expand" toggle.
const EXPANDABLE_QUOTE_CHARS: usize = 300;
/// Headroom for separators so the visible text stays under the limit.
const SLACK_CHARS: usize = 16;

/// The size limits a rendered message has to respect.
#[derive(Debug, Clone, Copy)]
pub struct TextBudget {
    /// Longest quoted body (comment, commit message, release notes).
    pub excerpt_chars: usize,
    /// Longest message Telegram accepts.
    pub max_text_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Link {
    text: String,
    url: String,
}

/// ```text
/// owner/repo · Pull request merged        <- scope and headline
/// #42 feat: retry deliveries              <- subject (linked)
/// alice merged feature/retry into main    <- details
/// > body excerpt                          <- quote
/// Compare changes                         <- action (linked)
/// ```
#[derive(Debug, Clone)]
pub struct Card {
    topic: Topic,
    scope: String,
    headline: String,
    subject: Option<Link>,
    details: Vec<String>,
    quote: Option<String>,
    action: Option<Link>,
    silent: bool,
}

impl Card {
    /// `scope` is what the event is about (a repository, an organization).
    pub fn new(topic: Topic, scope: impl Into<String>, headline: impl Into<String>) -> Self {
        Self {
            topic,
            scope: scope.into(),
            headline: headline.into(),
            subject: None,
            details: Vec::new(),
            quote: None,
            action: None,
            silent: false,
        }
    }

    pub fn subject(mut self, text: impl AsRef<str>, url: impl Into<String>) -> Self {
        self.subject = Some(Link {
            text: text.as_ref().to_owned(),
            url: url.into(),
        });
        self
    }

    pub fn detail(mut self, line: impl AsRef<str>) -> Self {
        self.details.push(line.as_ref().to_owned());
        self
    }

    /// User-written text shown as a quote; blank text is ignored.
    pub fn quote(mut self, text: impl AsRef<str>) -> Self {
        let text = text.as_ref().trim();
        self.quote = (!text.is_empty()).then(|| text.to_owned());
        self
    }

    /// A trailing link, for events without a natural subject.
    pub fn action(mut self, label: impl Into<String>, url: impl Into<String>) -> Self {
        self.action = Some(Link {
            text: label.into(),
            url: url.into(),
        });
        self
    }

    pub fn topic(&self) -> Topic {
        self.topic
    }

    /// Delivered without a sound or vibration.
    pub fn quiet(mut self) -> Self {
        self.silent = true;
        self
    }

    pub fn build(self, budget: TextBudget) -> Notification {
        let scope = one_line(&self.scope, LINE_MAX_CHARS);
        let headline = one_line(&self.headline, LINE_MAX_CHARS);
        let subject = self.subject.map(|l| Link {
            text: one_line(&l.text, LINE_MAX_CHARS),
            url: l.url,
        });
        let details: Vec<String> = self.details.iter().map(|d| one_line(d, LINE_MAX_CHARS)).collect();
        let action = self.action;

        let fixed_chars = scope.chars().count()
            + headline.chars().count()
            + subject.as_ref().map_or(0, |l| l.text.chars().count())
            + details.iter().map(|d| d.chars().count()).sum::<usize>()
            + action.as_ref().map_or(0, |l| l.text.chars().count())
            + details.len()
            + SLACK_CHARS;
        let quote_budget = budget
            .max_text_chars
            .saturating_sub(fixed_chars)
            .min(budget.excerpt_chars);
        let quote = self
            .quote
            .filter(|_| quote_budget >= 2)
            .map(|q| truncate(&q, quote_budget));

        let mut text = format!("<b>{}</b> · {}", escape(&scope), escape(&headline));
        if let Some(link) = subject {
            text.push_str(&format!("\n{}", anchor(&link)));
        }
        for detail in &details {
            text.push_str(&format!("\n{}", escape(detail)));
        }
        if let Some(quote) = quote {
            let tag = if quote.chars().count() > EXPANDABLE_QUOTE_CHARS {
                "blockquote expandable"
            } else {
                "blockquote"
            };
            text.push_str(&format!("\n<{tag}>{}</blockquote>", escape(&quote)));
        }
        if let Some(link) = action {
            text.push_str(&format!("\n{}", anchor(&link)));
        }

        let notification = Notification::to_topic(self.topic, text);
        if self.silent {
            notification.silent()
        } else {
            notification
        }
    }
}

fn anchor(link: &Link) -> String {
    format!("<a href=\"{}\">{}</a>", escape(&link.url), escape(&link.text))
}

/// Escapes the characters Telegram's HTML mode treats as markup.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Cuts `text` to at most `max` characters, ending with `…` when something was dropped.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Collapses whitespace (titles and commit subjects must stay on one line) and shortens.
fn one_line(text: &str, max: usize) -> String {
    truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: TextBudget = TextBudget {
        excerpt_chars: 1500,
        max_text_chars: 4096,
    };

    fn card() -> Card {
        Card::new(Topic::Github, "owner/repo", "Pull request merged")
    }

    #[test]
    fn lays_out_every_part_in_order() {
        let n = card()
            .subject("#42 feat: retry", "https://x/pr/42")
            .detail("alice merged a into main")
            .quote("looks good")
            .action("Compare", "https://x/c")
            .build(BUDGET);
        assert_eq!(
            n.text,
            "<b>owner/repo</b> · Pull request merged\n\
             <a href=\"https://x/pr/42\">#42 feat: retry</a>\n\
             alice merged a into main\n\
             <blockquote>looks good</blockquote>\n\
             <a href=\"https://x/c\">Compare</a>"
        );
        assert_eq!(n.destination, crate::domain::Destination::Topic(Topic::Github));
        assert!(!n.silent);
    }

    #[test]
    fn user_text_cannot_inject_markup() {
        let n = card()
            .subject("<script>&\"x\"", "https://x/?a=1&b=\"2\"")
            .quote("a < b && c > d")
            .build(BUDGET);
        assert!(n.text.contains("&lt;script&gt;&amp;&quot;x&quot;"), "{}", n.text);
        assert!(
            n.text.contains("href=\"https://x/?a=1&amp;b=&quot;2&quot;\""),
            "{}",
            n.text
        );
        assert!(
            n.text
                .contains("<blockquote>a &lt; b &amp;&amp; c &gt; d</blockquote>")
        );
    }

    #[test]
    fn titles_stay_on_one_line_and_are_cut() {
        let n = card()
            .subject(format!("a\n\nb {}", "x".repeat(500)), "u")
            .build(BUDGET);
        let subject = n.text.lines().nth(1).unwrap();
        assert!(subject.starts_with("<a href=\"u\">a b xxx"), "{subject}");
        assert!(subject.ends_with("…</a>"));
    }

    #[test]
    fn long_quotes_are_cut_before_tags_are_added_and_collapse() {
        let n = card().quote("é".repeat(10_000)).build(BUDGET);
        assert!(n.text.contains("<blockquote expandable>"));
        assert!(n.text.ends_with("…</blockquote>"));
        assert!(n.text.chars().count() < 1700);
    }

    #[test]
    fn the_quote_gives_way_to_the_rest_of_the_message_under_a_small_limit() {
        let small = TextBudget {
            excerpt_chars: 1500,
            max_text_chars: 100,
        };
        let n = card()
            .subject("a title", "u")
            .quote("x".repeat(1000))
            .build(small);
        let visible = n.text.replace("<b>", "").replace("</b>", "");
        assert!(visible.chars().count() < 200, "{}", n.text);
        assert!(n.text.contains("<blockquote>"));
    }

    #[test]
    fn blank_quotes_are_skipped() {
        let n = card().quote("  \n ").build(BUDGET);
        assert!(!n.text.contains("blockquote"));
    }

    #[test]
    fn quiet_cards_are_silent() {
        assert!(card().quiet().build(BUDGET).silent);
    }

    #[test]
    fn truncate_ends_with_an_ellipsis() {
        let cut = truncate(&"é".repeat(110), 100);
        assert_eq!(cut.chars().count(), 100);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("short", 100), "short");
    }
}
