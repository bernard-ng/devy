//! Pure domain: no HTTP, GitHub or Telegram types in here.
//!
//! Sources translate the outside world into [`Notification`]s, and a [`ports::Notifier`]
//! delivers them. Everything in between is plain data.

pub mod message;
pub mod notification;
pub mod ports;

pub use notification::{Destination, Notification, ReplyTo, Topic};
