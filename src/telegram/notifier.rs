use std::collections::HashMap;

use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::{LinkPreviewOptions, MessageId, ParseMode, ReplyParameters, ThreadId};

use crate::domain::ports::{Notifier, NotifyError};
use crate::domain::{Destination, Notification, Topic};

/// Where each [`Topic`] lives in the main chat.
#[derive(Debug, Clone)]
pub struct TopicRouting {
    pub chat_id: i64,
    /// Topics without a thread are sent to the chat itself.
    pub threads: HashMap<Topic, i32>,
}

pub struct TelegramNotifier {
    bot: Bot,
    routing: TopicRouting,
}

impl TelegramNotifier {
    pub fn new(bot: Bot, routing: TopicRouting) -> Self {
        Self { bot, routing }
    }
}

#[async_trait]
impl Notifier for TelegramNotifier {
    async fn notify(&self, notification: Notification) -> Result<(), NotifyError> {
        let (chat, thread, reply_to) = match notification.destination {
            Destination::Topic(topic) => (
                self.routing.chat_id,
                self.routing.threads.get(&topic).copied(),
                None,
            ),
            Destination::Reply(to) => (to.chat_id, to.thread_id, Some(to.message_id)),
        };

        let mut request = self
            .bot
            .send_message(ChatId(chat), notification.text)
            .parse_mode(ParseMode::Html)
            .disable_notification(notification.silent)
            .link_preview_options(LinkPreviewOptions {
                is_disabled: true,
                url: None,
                prefer_small_media: false,
                prefer_large_media: false,
                show_above_text: false,
            });

        if let Some(thread) = thread {
            request = request.message_thread_id(ThreadId(MessageId(thread)));
        }

        if let Some(message) = reply_to {
            request = request.reply_parameters(ReplyParameters::new(MessageId(message)));
        }

        request.await.map_err(NotifyError::new)?;
        Ok(())
    }
}
