# Devy: Realtime Notifications

Telegram bot that relays GitHub activity to the Telegram chat and answers a few chat commands.

```
GitHub ──POST /webhook/github──┐                         ┌──> Telegram topic (github, logs, ...)
                               ├─> source ─> Notification ┤
Telegram ─POST /webhook/telegram┘                         └──> Telegram reply (same chat/topic)
```

## Run

```sh
cp .env.example .env        # fill in the required values
cargo run -- serve          # listens on DEVY_ADDR (default 0.0.0.0:8000)
cargo run -- webhook set    # tell Telegram where the bot lives
cargo run -- webhook info | delete
```

GitHub: add a webhook to `https://<host>/webhook/github`, content type `application/json`, secret = `GITHUB_WEBHOOK_SECRET`.
Deliveries are verified with `X-Hub-Signature-256`; unsigned ones are rejected.

## Install

On a Linux server with systemd, run:

```sh
curl -fsSL https://raw.githubusercontent.com/bernard-ng/devy/main/deploy/install.sh | sudo bash
```

It installs the latest release, asks for your Telegram bot token, chat id and bot username, and starts Devy as a service. Run it again to upgrade.

To remove Devy:

```sh
curl -fsSL https://raw.githubusercontent.com/bernard-ng/devy/main/deploy/uninstall.sh | sudo bash
```

Telegram only calls HTTPS webhooks, so put a reverse proxy with TLS (Caddy, nginx, ...) in front of Devy, which listens on `127.0.0.1:8000`. More options are in [`deploy/README.md`](deploy/README.md).

## Architecture

| Layer | Path | Knows about |
|---|---|---|
| Domain | `src/domain` | `Notification`, `Topic`, the `Notifier` port, the GitHub `Activity` model and its wording. No octocrab/teloxide/axum. |
| Inbound adapters | `src/sources` | Authentication and payload typing (octocrab models, teloxide `Update`), mapped into the domain. |
| Outbound adapter | `src/telegram` | `TelegramNotifier` (teloxide) and topic -> chat/thread routing. |
| Application | `src/app.rs` | Looks up a source, interprets the request inline, delivers in the background. |
| HTTP | `src/http` | axum router: `POST /webhook/{source}`, `GET /health`. |

### Extending

- **New webhook integration**: implement `sources::WebhookSource` (`name`, `receive`) and register it with `Sources::with(..)` in `cli.rs`. It is served at `/webhook/{name}`.
- **New GitHub event**: add an `Activity` variant (`domain/github.rs`), map it in `sources/github/mapping.rs`, and add its name to `HANDLED_EVENTS`.
- **New chat command**: add a variant to `sources/telegram/commands.rs::Command` and its answer.
- **New destination** (Slack, Discord, ...): implement `domain::ports::Notifier`.
- **New topic**: add a `Topic` variant; its thread id is configurable with `TELEGRAM_TOPIC_<NAME>`.

## License

[MIT](LICENSE)
