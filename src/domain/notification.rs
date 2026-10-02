use std::fmt;
use std::str::FromStr;

/// Telegram rejects messages longer than this many characters.
pub const MAX_TEXT_CHARS: usize = 4096;

/// Logical channels of the community chat. Mapping a topic to an actual chat/thread is the
/// notifier's concern, so the domain never deals with raw ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    General,
    Github,
    Logs,
    Notifications,
    Documents,
    Assets,
    Sharing,
}

impl Topic {
    pub const ALL: [Topic; 7] = [
        Topic::General,
        Topic::Github,
        Topic::Logs,
        Topic::Notifications,
        Topic::Documents,
        Topic::Assets,
        Topic::Sharing,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Topic::General => "general",
            Topic::Github => "github",
            Topic::Logs => "logs",
            Topic::Notifications => "notifications",
            Topic::Documents => "documents",
            Topic::Assets => "assets",
            Topic::Sharing => "sharing",
        }
    }

    /// Thread ids of the chat.
    pub fn default_thread_id(self) -> i32 {
        match self {
            Topic::General => 1,
            Topic::Github => 4905,
            Topic::Logs => 4919,
            Topic::Notifications => 4998,
            Topic::Documents => 4959,
            Topic::Assets => 4921,
            Topic::Sharing => 4957,
        }
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for Topic {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Topic::ALL
            .into_iter()
            .find(|topic| topic.key().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown topic `{s}`"))
    }
}

/// The message a reply must be attached to (chat commands answer where they were asked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyTo {
    pub chat_id: i64,
    pub thread_id: Option<i32>,
    pub message_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// A well-known channel of the main chat.
    Topic(Topic),
    /// A reply to a message someone sent.
    Reply(ReplyTo),
}

/// Something worth telling humans about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub destination: Destination,
    pub text: String,
    /// Delivered without a sound/vibration.
    pub silent: bool,
}

impl Notification {
    pub fn to_topic(topic: Topic, text: impl Into<String>) -> Self {
        Self::new(Destination::Topic(topic), text)
    }

    pub fn reply(to: ReplyTo, text: impl Into<String>) -> Self {
        Self::new(Destination::Reply(to), text)
    }

    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    fn new(destination: Destination, text: impl Into<String>) -> Self {
        Self {
            destination,
            text: truncate(text.into(), MAX_TEXT_CHARS),
            silent: false,
        }
    }
}

/// Cuts `text` to at most `max` characters, ending with `…` when something was dropped.
fn truncate(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

/// Shortens user-written content (comments, commit messages) so the rest of the message, links
/// included, always fits.
pub fn excerpt(text: &str, max: usize) -> String {
    truncate(text.trim().to_owned(), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_truncated_to_the_limit() {
        let n = Notification::to_topic(Topic::Github, "é".repeat(MAX_TEXT_CHARS + 10));
        assert_eq!(n.text.chars().count(), MAX_TEXT_CHARS);
        assert!(n.text.ends_with('…'));
    }

    #[test]
    fn excerpt_trims_and_shortens() {
        assert_eq!(excerpt("  hi \n", 10), "hi");
        assert_eq!(excerpt("abcdef", 4), "abc…");
    }

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(Notification::to_topic(Topic::Github, "hi").text, "hi");
    }

    #[test]
    fn topics_round_trip_through_their_key() {
        for topic in Topic::ALL {
            assert_eq!(topic.key().parse::<Topic>(), Ok(topic));
        }
        assert!("nope".parse::<Topic>().is_err());
    }
}
