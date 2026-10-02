# Devy: Realtime Notifications

Telegram bot that relays GitHub activity to the Telegram chat and answers a few chat commands.

```
GitHub ──POST /webhook/github──┐                         ┌──> Telegram topic (github, logs, ...)
                               ├─> source ─> Notification ┤
Telegram ─POST /webhook/telegram┘                         └──> Telegram reply (same chat/topic)
```

## Run

```sh
cp .env.example .env        # fill in the secrets
cargo run -- serve          # listens on DEVY_ADDR (default 0.0.0.0:8000)
cargo run -- webhook set    # tell Telegram where the bot lives
cargo run -- webhook info | delete
```

GitHub: add a webhook to `https://<host>/webhook/github`, content type `application/json`, secret = `GITHUB_WEBHOOK_SECRET`.
Deliveries are verified with `X-Hub-Signature-256`; unsigned ones are rejected.

## Deploy with systemd

Telegram only calls HTTPS webhooks, so run Devy on loopback behind a TLS reverse proxy (Caddy, nginx, ...). The unit in [`deploy/systemd/devy.service`](deploy/systemd/devy.service) runs it as a sandboxed, throwaway user.

```sh
# 1. binary: from a release tarball (or `cargo build --release`)
sudo install -m 755 devy /usr/local/bin/devy

# 2. configuration: readable by root only, systemd passes it to the service
sudo install -d -m 755 /etc/devy
sudo install -m 600 .env.example /etc/devy/devy.env
sudoedit /etc/devy/devy.env

# 3. service
sudo install -m 644 deploy/systemd/devy.service /etc/systemd/system/devy.service
sudo systemctl daemon-reload
sudo systemctl enable --now devy

# 4. register the Telegram webhook (once, and whenever the URL or secret changes)
sudo systemd-run --pty --wait -p EnvironmentFile=/etc/devy/devy.env /usr/local/bin/devy webhook set
```

Reverse proxy, for example with Caddy:

```
devy.example.com {
    reverse_proxy 127.0.0.1:8000
}
```

Operate it with `systemctl status devy`, `journalctl -u devy -f` (set `RUST_LOG=debug` in the env file for more detail) and `systemctl restart devy` after editing the env file.
To upgrade, replace `/usr/local/bin/devy` and restart the service.

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
