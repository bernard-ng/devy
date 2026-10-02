use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;

use crate::domain::Topic;

/// The chat.
const DEFAULT_CHAT_ID: i64 = -1001789113311;
const DEFAULT_BOT_USERNAME: &str = "DdevyBot";
const DEFAULT_WEBHOOK_URL: &str = "https://devy.ngandu.dev/webhook/telegram";
const DEFAULT_ADDR: &str = "0.0.0.0:8000";

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
    pub webhook_url: String,
    /// Without the leading `@`.
    pub bot_username: String,
    pub chat_id: i64,
    pub topics: HashMap<Topic, i32>,
}

#[derive(Debug, Clone)]
pub struct GithubSettings {
    /// Signs deliveries (`X-Hub-Signature-256`).
    pub webhook_secret: Secret,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub addr: SocketAddr,
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

        Ok(Self {
            addr,
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
        let chat_id = parse_or(env, "TELEGRAM_CHAT_ID", DEFAULT_CHAT_ID)?;

        let mut topics = HashMap::new();
        for topic in Topic::ALL {
            let key = format!("TELEGRAM_TOPIC_{}", topic.key().to_ascii_uppercase());
            topics.insert(topic, parse_or(env, &key, topic.default_thread_id())?);
        }

        Ok(Self {
            token: required_secret(env, "TELEGRAM_API_TOKEN")?,
            webhook_secret: required_secret(env, "TELEGRAM_WEBHOOK_SECRET")?,
            webhook_url: env("TELEGRAM_WEBHOOK_URL")
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| DEFAULT_WEBHOOK_URL.to_owned()),
            bot_username: env("TELEGRAM_BOT_USERNAME")
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| DEFAULT_BOT_USERNAME.to_owned())
                .trim_start_matches('@')
                .to_owned(),
            chat_id,
            topics,
        })
    }
}

/// Secrets are mandatory: an empty one would let anybody through.
fn required_secret(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Result<Secret, ConfigError> {
    match env(key) {
        None => Err(ConfigError::Missing(key.into())),
        Some(v) if v.is_empty() => Err(ConfigError::Empty(key.into())),
        Some(v) => Ok(Secret::new(v)),
    }
}

fn parse_or<T>(env: &dyn Fn(&str) -> Option<String>, key: &str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    match env(key).filter(|v| !v.is_empty()) {
        Some(raw) => raw.parse().map_err(|e: T::Err| ConfigError::Invalid {
            key: key.into(),
            reason: e.to_string(),
        }),
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    const BASE: &[(&str, &str)] = &[
        ("TELEGRAM_API_TOKEN", "t"),
        ("TELEGRAM_WEBHOOK_SECRET", "s"),
        ("GITHUB_WEBHOOK_SECRET", "g"),
    ];

    #[test]
    fn defaults_target_the_chat() {
        let settings = Settings::load(&env(BASE)).unwrap();
        assert_eq!(settings.telegram.chat_id, -1001789113311);
        assert_eq!(settings.telegram.topics[&Topic::Github], 4905);
        assert_eq!(settings.telegram.bot_username, "DdevyBot");
    }

    #[test]
    fn empty_secret_is_rejected() {
        let result = Settings::load(&env(&[
            ("TELEGRAM_API_TOKEN", "t"),
            ("TELEGRAM_WEBHOOK_SECRET", ""),
            ("GITHUB_WEBHOOK_SECRET", "g"),
        ]));
        assert!(matches!(result, Err(ConfigError::Empty(k)) if k == "TELEGRAM_WEBHOOK_SECRET"));
    }

    #[test]
    fn topics_can_be_overridden() {
        let settings = Settings::load(&env(&[
            ("TELEGRAM_API_TOKEN", "t"),
            ("TELEGRAM_WEBHOOK_SECRET", "s"),
            ("GITHUB_WEBHOOK_SECRET", "g"),
            ("TELEGRAM_TOPIC_GITHUB", "42"),
            ("TELEGRAM_BOT_USERNAME", "@Other"),
        ]))
        .unwrap();
        assert_eq!(settings.telegram.topics[&Topic::Github], 42);
        assert_eq!(settings.telegram.bot_username, "Other");
    }

    #[test]
    fn secrets_are_not_debug_printed() {
        assert!(!format!("{:?}", Secret::new("hunter2")).contains("hunter2"));
    }
}
