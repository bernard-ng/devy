//! Telegram updates (the bot's chat commands), delivered through the bot webhook.

mod commands;

use async_trait::async_trait;
use teloxide::types::{Message, Update, UpdateKind};
use teloxide::utils::command::{BotCommands, ParseError};

use self::commands::{Command, UNKNOWN_COMMAND_REPLY};
use super::{SourceError, WebhookRequest, WebhookSource, auth};
use crate::config::Secret;
use crate::domain::{Notification, ReplyTo};

pub struct TelegramSource {
    secret: Secret,
    /// Without the `@`; needed to tell `/start@DdevyBot` from `/start@SomeoneElsesBot`.
    bot_username: String,
}

impl TelegramSource {
    pub fn new(secret: Secret, bot_username: impl Into<String>) -> Self {
        Self {
            secret,
            bot_username: bot_username.into(),
        }
    }

    fn respond_to(&self, message: &Message) -> Option<Notification> {
        let text = message.text().filter(|t| t.starts_with('/'))?;

        // Topic ids only exist in forum chats; sending one elsewhere makes Telegram refuse the reply.
        let thread_id = message
            .thread_id
            .filter(|_| message.is_topic_message)
            .map(|t| t.0.0);

        let to = ReplyTo {
            chat_id: message.chat.id.0,
            thread_id,
            message_id: message.id.0,
        };

        match Command::parse(text, &self.bot_username) {
            Ok(command) => Some(Notification::reply(to, command.answer())),
            // Addressed to another bot of the group: none of our business.
            Err(ParseError::WrongBotName(_)) => None,
            Err(_) => Some(Notification::reply(to, UNKNOWN_COMMAND_REPLY).silent()),
        }
    }
}

#[async_trait]
impl WebhookSource for TelegramSource {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn receive(&self, request: &WebhookRequest) -> Result<Vec<Notification>, SourceError> {
        auth::verify_token(&self.secret, request.header("x-telegram-bot-api-secret-token"))?;

        let update: Update = serde_json::from_slice(&request.body)
            .map_err(|e| SourceError::Malformed(format!("telegram update: {e}")))?;

        Ok(match update.kind {
            UpdateKind::Message(message) => self.respond_to(&message).into_iter().collect(),
            _ => vec![],
        })
    }
}
