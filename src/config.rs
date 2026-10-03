use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;

use crate::domain::Topic;

const DEFAULT_ADDR: &str = "0.0.0.0:8000";
/// GitHub caps webhook payloads at 25 MB.
const DEFAULT_MAX_BODY_BYTES: usize = 25 * 1024 * 1024;
/// Telegram rejects messages longer than this many characters.
const TELEGRAM_MAX_TEXT_CHARS: usize = 4096;
/// Longest user-written body quoted in a message; leaves room for the surrounding text and link.
const DEFAULT_BODY_EXCERPT_CHARS: usize = 1500;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing environment variable {0}")]
    Missing(String),
    #[error("environment variable {0} is empty")]
    Empty(String),
    #[error("invalid value for {key}: {reason}")]
    Invalid { key: String, reason: String },
}

/// A value that must never end up in logs.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

#[derive(Debug, Clone)]
pub struct TelegramSettings {
    pub token: Secret,
    /// Checked against `X-Telegram-Bot-Api-Secret-Token` on incoming updates.
    pub webhook_secret: Secret,
    /// Only needed to register the webhook, so it is checked by `devy webhook set`.
    pub webhook_url: Option<String>,
    /// Without the leading `@`.
    pub bot_username: String,
    pub chat_id: i64,
    /// Thread ids of the topics that are configured; the others are sent to the chat itself.
    pub topics: HashMap<Topic, i32>,
}

#[derive(Debug, Clone)]
pub struct GithubSettings {
    /// Signs deliveries (`X-Hub-Signature-256`).
    pub webhook_secret: Secret,
}

/// Size limits, all tunable with `DEVY_*` variables.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// `DEVY_MAX_BODY_BYTES`: largest webhook payload accepted.
    pub max_body_bytes: usize,
    /// `DEVY_MAX_TEXT_CHARS`: longest message sent to Telegram.
    pub max_text_chars: usize,
    /// `DEVY_BODY_EXCERPT_CHARS`: longest comment/commit message quoted in a notification.
    pub body_excerpt_chars: usize,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub addr: SocketAddr,
    pub limits: Limits,
    pub telegram: TelegramSettings,
    pub github: GithubSettings,
}

impl Settings {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::load(&|key| std::env::var(key).ok())
    }

    pub fn load(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let addr = match env("DEVY_ADDR") {
            Some(raw) => raw
                .parse()
                .map_err(|e: std::net::AddrParseError| ConfigError::Invalid {
                    key: "DEVY_ADDR".into(),
                    reason: e.to_string(),
                })?,
            None => DEFAULT_ADDR.parse().expect("default address is valid"),
        };

        let limits = Limits {
            max_body_bytes: parse_in_range(
                env,
                "DEVY_MAX_BODY_BYTES",
                DEFAULT_MAX_BODY_BYTES,
                1..=usize::MAX,
            )?,
            max_text_chars: parse_in_range(
                env,
                "DEVY_MAX_TEXT_CHARS",
                TELEGRAM_MAX_TEXT_CHARS,
                2..=TELEGRAM_MAX_TEXT_CHARS,
            )?,
            body_excerpt_chars: parse_in_range(
                env,
                "DEVY_BODY_EXCERPT_CHARS",
                DEFAULT_BODY_EXCERPT_CHARS,
                2..=TELEGRAM_MAX_TEXT_CHARS,
            )?,
        };

        Ok(Self {
            addr,
            limits,
            telegram: TelegramSettings::load(env)?,
            github: GithubSettings {
                webhook_secret: required_secret(env, "GITHUB_WEBHOOK_SECRET")?,
            },
        })
    }
}

impl TelegramSettings {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::load(&|key| std::env::var(key).ok())
    }

    pub fn load(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut topics = HashMap::new();
        for topic in Topic::ALL {
            let key = format!("TELEGRAM_TOPIC_{}", topic.key().to_ascii_uppercase());
            if let Some(thread_id) = parse_optional(env, &key)? {
                topics.insert(topic, thread_id);
            }
        }

        Ok(Self {
            token: required_secret(env, "TELEGRAM_API_TOKEN")?,
            webhook_secret: required_secret(env, "TELEGRAM_WEBHOOK_SECRET")?,
            webhook_url: non_empty(env, "TELEGRAM_WEBHOOK_URL"),
            bot_username: required(env, "TELEGRAM_BOT_USERNAME")?
                .trim_start_matches('@')
                .to_owned(),
            chat_id: required_parsed(env, "TELEGRAM_CHAT_ID")?,
            topics,
        })
    }
}

fn non_empty(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    env(key).filter(|v| !v.is_empty())
}

fn required(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Result<String, ConfigError> {
    match env(key) {
        None => Err(ConfigError::Missing(key.into())),
        Some(v) if v.is_empty() => Err(ConfigError::Empty(key.into())),
        Some(v) => Ok(v),
    }
}

/// Secrets are mandatory: an empty one would let anybody through.
fn required_secret(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Result<Secret, ConfigError> {
    required(env, key).map(Secret::new)
}

fn parse<T>(key: &str, raw: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    raw.parse().map_err(|e: T::Err| ConfigError::Invalid {
        key: key.into(),
        reason: e.to_string(),
    })
}

fn required_parsed<T>(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    parse(key, &required(env, key)?)
}

fn parse_optional<T>(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    non_empty(env, key).map(|raw| parse(key, &raw)).transpose()
}

fn parse_in_range(
    env: &dyn Fn(&str) -> Option<String>,
    key: &str,
    default: usize,
    range: std::ops::RangeInclusive<usize>,
) -> Result<usize, ConfigError> {
    let value = parse_optional(env, key)?.unwrap_or(default);
    if range.contains(&value) {
        Ok(value)
    } else {
        Err(ConfigError::Invalid {
            key: key.into(),
            reason: format!("must be between {} and {}", range.start(), range.end()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &[(&str, &str)] = &[
        ("TELEGRAM_API_TOKEN", "t"),
        ("TELEGRAM_WEBHOOK_SECRET", "s"),
        ("GITHUB_WEBHOOK_SECRET", "g"),
        ("TELEGRAM_CHAT_ID", "-100123"),
        ("TELEGRAM_BOT_USERNAME", "TestBot"),
    ];

    fn without(key: &str) -> Vec<(&'static str, &'static str)> {
        BASE.iter().copied().filter(|(k, _)| *k != key).collect()
    }

    fn load(pairs: &[(&str, &str)]) -> Result<Settings, ConfigError> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Settings::load(&|key| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
    }

    #[test]
    fn only_secrets_and_the_chat_are_required() {
        let settings = load(BASE).unwrap();
        assert_eq!(settings.telegram.chat_id, -100123);
        assert_eq!(settings.telegram.bot_username, "TestBot");
        assert_eq!(settings.telegram.webhook_url, None);
        assert!(settings.telegram.topics.is_empty());
        assert_eq!(settings.limits.max_text_chars, 4096);
        assert_eq!(settings.limits.max_body_bytes, 25 * 1024 * 1024);
        assert_eq!(settings.limits.body_excerpt_chars, 1500);
    }

    #[test]
    fn missing_or_empty_values_are_rejected() {
        for key in ["TELEGRAM_API_TOKEN", "TELEGRAM_CHAT_ID", "TELEGRAM_BOT_USERNAME"] {
            let result = load(&without(key));
            assert!(
                matches!(result, Err(ConfigError::Missing(k)) if k == key),
                "{key}"
            );
        }

        let mut pairs = without("TELEGRAM_WEBHOOK_SECRET");
        pairs.push(("TELEGRAM_WEBHOOK_SECRET", ""));
        let result = load(&pairs);
        assert!(matches!(result, Err(ConfigError::Empty(k)) if k == "TELEGRAM_WEBHOOK_SECRET"));
    }

    #[test]
    fn invalid_chat_id_is_rejected() {
        let mut pairs = without("TELEGRAM_CHAT_ID");
        pairs.push(("TELEGRAM_CHAT_ID", "my-chat"));
        let result = load(&pairs);
        assert!(matches!(result, Err(ConfigError::Invalid { key, .. }) if key == "TELEGRAM_CHAT_ID"));
    }

    #[test]
    fn only_configured_topics_are_routed() {
        let mut pairs = BASE.to_vec();
        pairs.extend([("TELEGRAM_TOPIC_GITHUB", "42"), ("TELEGRAM_TOPIC_LOGS", "")]);
        let settings = load(&pairs).unwrap();
        assert_eq!(settings.telegram.topics.get(&Topic::Github), Some(&42));
        assert_eq!(settings.telegram.topics.len(), 1);
    }

    #[test]
    fn bot_username_drops_the_at_sign() {
        let mut pairs = without("TELEGRAM_BOT_USERNAME");
        pairs.push(("TELEGRAM_BOT_USERNAME", "@Other"));
        assert_eq!(load(&pairs).unwrap().telegram.bot_username, "Other");
    }

    #[test]
    fn limits_can_be_tuned_within_bounds() {
        let mut pairs = BASE.to_vec();
        pairs.extend([
            ("DEVY_MAX_TEXT_CHARS", "1000"),
            ("DEVY_BODY_EXCERPT_CHARS", "200"),
        ]);
        let limits = load(&pairs).unwrap().limits;
        assert_eq!((limits.max_text_chars, limits.body_excerpt_chars), (1000, 200));

        for (key, value) in [
            ("DEVY_MAX_TEXT_CHARS", "5000"),
            ("DEVY_MAX_TEXT_CHARS", "1"),
            ("DEVY_MAX_BODY_BYTES", "0"),
            ("DEVY_BODY_EXCERPT_CHARS", "lots"),
        ] {
            let mut pairs = BASE.to_vec();
            pairs.push((key, value));
            let result = load(&pairs);
            assert!(
                matches!(result, Err(ConfigError::Invalid { key: k, .. }) if k == key),
                "{key}={value}"
            );
        }
    }

    #[test]
    fn secrets_are_not_debug_printed() {
        assert!(!format!("{:?}", Secret::new("hunter2")).contains("hunter2"));
    }
}
