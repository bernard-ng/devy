# Deploy on Linux

Install or upgrade (the installer picks the right build for your machine):

```bash
curl -fsSL https://raw.githubusercontent.com/bernard-ng/devy/main/deploy/install.sh | sudo bash
```

On the first run it asks for your Telegram bot token, chat id, bot username and webhook URL, creates the `devy` service, starts it, and registers the Telegram webhook. It prints the secret to use when adding the GitHub webhook. Your settings live in `/etc/devy/devy.env` and are kept on upgrades.

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/bernard-ng/devy/main/deploy/install.sh | sudo bash -s -- v0.1.0
```

Check on it:

```bash
systemctl status devy
journalctl -u devy -f
```

Remove it (add `-s -- --keep-config` to keep your settings):

```bash
curl -fsSL https://raw.githubusercontent.com/bernard-ng/devy/main/deploy/uninstall.sh | sudo bash
```
