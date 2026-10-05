use std::fmt;
use std::str::FromStr;

/// Logical channels of the community chat. Mapping a topic to an actual chat/thread is the
/// notifier's concern, so the domain never deals with raw ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    General,
    Github,
    Logs,
    Notifications,
}

impl Topic {
    pub const ALL: [Topic; 4] = [Topic::General, Topic::Github, Topic::Logs, Topic::Notifications];

    pub fn key(self) -> &'static str {
        match self {
            Topic::General => "general",
            Topic::Github => "github",
            Topic::Logs => "logs",
            Topic::Notifications => "notifications",
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
    /// Telegram HTML. Build it with [`Card`](crate::domain::message::Card) or escape it
    /// with [`escape`](crate::domain::message::escape).
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
            text: text.into(),
            silent: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_round_trip_through_their_key() {
        for topic in Topic::ALL {
            assert_eq!(topic.key().parse::<Topic>(), Ok(topic));
        }
        assert!("nope".parse::<Topic>().is_err());
    }
}
