//! Outbound adapter: delivers notifications to Telegram.

mod notifier;

pub use notifier::{TelegramNotifier, TopicRouting};
