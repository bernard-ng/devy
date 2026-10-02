use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use teloxide::prelude::*;
use teloxide::types::AllowedUpdate;
use tokio::net::TcpListener;

use crate::app::App;
use crate::config::{Settings, TelegramSettings};
use crate::sources::Sources;
use crate::sources::github::GithubSource;
use crate::sources::telegram::TelegramSource;
use crate::telegram::{TelegramNotifier, TopicRouting};

#[derive(Debug, Parser)]
#[command(name = "devy", version, about = "Devy Notifier")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the webhook server
    Serve,
    /// Manage the Telegram bot webhook
    #[command(subcommand)]
    Webhook(WebhookCommand),
}

#[derive(Debug, Subcommand)]
enum WebhookCommand {
    /// Point Telegram at this bot (defaults to TELEGRAM_WEBHOOK_URL)
    Set { url: Option<String> },
    /// Show what Telegram currently knows
    Info,
    /// Stop Telegram from calling the webhook
    Delete,
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::Serve => serve().await,
            Command::Webhook(command) => webhook(command).await,
        }
    }
}

async fn serve() -> anyhow::Result<()> {
    let settings = Settings::from_env()?;
    let Settings {
        addr,
        telegram,
        github,
    } = settings;

    let notifier = TelegramNotifier::new(
        Bot::new(telegram.token.expose()),
        TopicRouting {
            chat_id: telegram.chat_id,
            threads: telegram.topics.clone(),
        },
    );

    let sources = Sources::new()
        .with(GithubSource::new(github.webhook_secret))
        .with(TelegramSource::new(
            telegram.webhook_secret,
            telegram.bot_username,
        ));

    let app = Arc::new(App::new(sources, Arc::new(notifier)));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!(%addr, "devy is listening");

    axum::serve(listener, crate::http::router(Arc::clone(&app)))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("draining pending notifications");
    app.drain().await;
    Ok(())
}

async fn webhook(command: WebhookCommand) -> anyhow::Result<()> {
    let settings = TelegramSettings::from_env()?;
    let bot = Bot::new(settings.token.expose());

    match command {
        WebhookCommand::Set { url } => {
            let url = url.unwrap_or(settings.webhook_url);
            let parsed = url
                .parse()
                .with_context(|| format!("`{url}` is not a valid URL"))?;
            bot.set_webhook(parsed)
                .drop_pending_updates(true)
                .secret_token(settings.webhook_secret.expose())
                .allowed_updates([AllowedUpdate::Message])
                .await
                .context("setWebhook failed")?;
            println!("Webhook : {url}");
        }
        WebhookCommand::Info => {
            let info = bot.get_webhook_info().await.context("getWebhookInfo failed")?;
            println!("url: {}", info.url.map(|u| u.to_string()).unwrap_or_default());
            println!("pending updates: {}", info.pending_update_count);
            if let Some(error) = info.last_error_message {
                println!("last error: {error}");
            }
        }
        WebhookCommand::Delete => {
            bot.delete_webhook().await.context("deleteWebhook failed")?;
            println!("Webhook deleted");
        }
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl-c");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
