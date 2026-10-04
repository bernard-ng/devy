#!/usr/bin/env bash
# Installs (or upgrades) Devy as a sandboxed systemd service.
#
#   curl -fsSL https://raw.githubusercontent.com/bernard-ng/devy/main/deploy/install.sh | sudo bash
#   curl -fsSL .../install.sh | sudo bash -s -- v0.1.0        # a specific release
#   sudo ./deploy/install.sh https://github.com/.../devy-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
set -Eeuo pipefail

REPO=bernard-ng/devy
BINARY_PATH=/usr/local/bin/devy
CONFIG_DIR=/etc/devy
ENV_FILE="${CONFIG_DIR}/devy.env"
SERVICE_NAME=devy.service
UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

have_tty() {
  (: </dev/tty) 2>/dev/null
}

# Reads from the terminal rather than stdin, so the prompts still work when the script is piped from curl.
read_tty() {
  local prompt=$1 secret=${2:-false} value=
  if [[ $secret == true ]]; then
    read -r -s -p "$prompt" value </dev/tty || die "cannot read installer input from /dev/tty (preset the variable instead)"
    printf '\n' >&2
  else
    read -r -p "$prompt" value </dev/tty || die "cannot read installer input from /dev/tty (preset the variable instead)"
  fi
  printf '%s' "$value"
}

# Values land in an env file read by systemd (and dotenv), so refuse anything that would need quoting.
validate_value() {
  [[ $2 =~ ^[^[:space:]\"\'\\\$#]+$ ]] || die "$1 cannot contain whitespace, quotes, backslashes, '\$' or '#'"
}

random_secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n'
  fi
}

resolve_target() {
  case $(uname -m) in
    x86_64 | amd64) printf 'x86_64-unknown-linux-gnu' ;;
    aarch64 | arm64) printf 'aarch64-unknown-linux-gnu' ;;
    *) die "unsupported architecture: $(uname -m) (releases cover x86_64 and aarch64)" ;;
  esac
}

# `releases/latest` redirects to `releases/tag/<tag>`.
latest_tag() {
  local final
  final=$(curl --fail --location --silent --show-error --head --output /dev/null \
    --write-out '%{url_effective}' "https://github.com/${REPO}/releases/latest") \
    || die "could not reach github.com/${REPO}/releases/latest"
  [[ $final == */releases/tag/* ]] || die "no published release found for ${REPO}"
  printf '%s' "${final##*/}"
}

download_release() {
  local url=$1 dir=$2 asset
  asset="${dir}/${url##*/}"
  printf 'Downloading %s...\n' "${url##*/}" >&2
  curl --fail --location --show-error --silent --retry 3 --output "$asset" "$url"

  if curl --fail --location --silent --retry 3 --output "${asset}.sha256" "${url}.sha256"; then
    if command -v sha256sum >/dev/null 2>&1; then
      (cd "$dir" && sha256sum --check --status "${asset##*/}.sha256") || die "checksum mismatch for ${url##*/}"
    else
      printf 'warning: sha256sum not found, skipping checksum verification\n' >&2
    fi
  else
    printf 'warning: no checksum published for this asset, skipping verification\n' >&2
  fi
  printf '%s' "$asset"
}

# Uses the preset environment variable when there is one, otherwise asks until it gets an answer.
ask() {
  local name=$1 prompt=$2 secret=${3:-false} value=${!1:-}
  while [[ -z $value ]]; do
    have_tty || die "$name is required: set it in the environment (no terminal to ask on)"
    value=$(read_tty "$prompt" "$secret")
  done
  validate_value "$name" "$value"
  printf '%s' "$value"
}

validate_chat_id() {
  [[ $1 =~ ^-?[0-9]+$ ]] || die "TELEGRAM_CHAT_ID must be a number (for example -1001234567890)"
}

# Sets the first-run values: from the environment when present, otherwise by asking.
write_initial_config() {
  local token chat_id bot_username webhook_url tg_secret=${TELEGRAM_WEBHOOK_SECRET:-} gh_secret=${GITHUB_WEBHOOK_SECRET:-}

  token=$(ask TELEGRAM_API_TOKEN 'Telegram bot API token (from @BotFather): ' true)
  chat_id=$(ask TELEGRAM_CHAT_ID 'Telegram chat id (for example -1001234567890): ')
  validate_chat_id "$chat_id"
  bot_username=$(ask TELEGRAM_BOT_USERNAME 'Telegram bot username (without @): ')
  webhook_url=$(ask TELEGRAM_WEBHOOK_URL 'Public Telegram webhook URL (https://devy.example.com/webhook/telegram): ')
  [[ $webhook_url == https://* ]] || die "the Telegram webhook URL must be https://"

  if [[ -z $tg_secret ]]; then
    tg_secret=$(random_secret)
  fi
  validate_value TELEGRAM_WEBHOOK_SECRET "$tg_secret"

  if [[ -z $gh_secret ]]; then
    gh_secret=$(random_secret)
    GENERATED_GITHUB_SECRET=$gh_secret
  fi
  validate_value GITHUB_WEBHOOK_SECRET "$gh_secret"

  install -d -m 0755 -o root -g root "$CONFIG_DIR"
  (
    umask 077
    {
      printf 'TELEGRAM_API_TOKEN=%s\n' "$token"
      printf 'TELEGRAM_WEBHOOK_SECRET=%s\n' "$tg_secret"
      printf 'GITHUB_WEBHOOK_SECRET=%s\n' "$gh_secret"
      printf 'TELEGRAM_CHAT_ID=%s\n' "$chat_id"
      printf 'TELEGRAM_BOT_USERNAME=%s\n' "$bot_username"
      printf 'TELEGRAM_WEBHOOK_URL=%s\n' "$webhook_url"
      write_preset_topics
      printf 'RUST_LOG=info\n'
    } >"$ENV_FILE"
  )
  chown root:root "$ENV_FILE"
  chmod 0600 "$ENV_FILE"
}

# Topic thread ids are optional: only the ones set in the environment are written.
write_preset_topics() {
  local topic name value
  for topic in GENERAL GITHUB LOGS NOTIFICATIONS; do
    name=TELEGRAM_TOPIC_${topic}
    value=${!name:-}
    [[ -n $value ]] || continue
    [[ $value =~ ^[0-9]+$ ]] || die "$name must be a number"
    printf '%s=%s\n' "$name" "$value"
  done
}

# Releases that predate a setting would otherwise restart into a configuration error.
ensure_required_config() {
  local name value
  for name in TELEGRAM_CHAT_ID TELEGRAM_BOT_USERNAME; do
    grep -Eq "^${name}=.+" "$ENV_FILE" && continue
    printf '%s is missing from %s (required since this release).\n' "$name" "$ENV_FILE"
    if [[ $name == TELEGRAM_CHAT_ID ]]; then
      value=$(ask "$name" 'Telegram chat id (for example -1001234567890): ')
      validate_chat_id "$value"
    else
      value=$(ask "$name" 'Telegram bot username (without @): ')
    fi
    printf '%s=%s\n' "$name" "$value" >>"$ENV_FILE"
  done
}

# Keep in sync with deploy/systemd/devy.service (used for manual installs).
write_service_unit() {
  local unit_tmp
  unit_tmp=$(mktemp -d)

  cat >"${unit_tmp}/${SERVICE_NAME}" <<'UNIT'
[Unit]
Description=Devy: realtime notifications (GitHub -> Telegram)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/devy serve
EnvironmentFile=/etc/devy/devy.env
# Devy sits behind a TLS reverse proxy, so keep it on loopback unless the env file says otherwise.
Environment=DEVY_ADDR=127.0.0.1:8000

Restart=on-failure
RestartSec=5
# On SIGTERM devy stops accepting webhooks and finishes pending Telegram deliveries.
TimeoutStopSec=30

# Runs as a throwaway unprivileged user; no files of its own are needed.
DynamicUser=yes

# Sandboxing
NoNewPrivileges=yes
CapabilityBoundingSet=
AmbientCapabilities=
PrivateTmp=yes
PrivateDevices=yes
ProtectSystem=strict
ProtectHome=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources

[Install]
WantedBy=multi-user.target
UNIT

  install -m 0644 -o root -g root "${unit_tmp}/${SERVICE_NAME}" "$UNIT_PATH"
  rm -rf "$unit_tmp"
}

# `systemctl restart` succeeds for Type=simple even if the process dies right after, so poll for a stable state.
wait_until_active() {
  local attempt
  for attempt in 1 2 3 4 5; do
    sleep 1
    systemctl is-active --quiet "$SERVICE_NAME" || return 1
  done
}

register_telegram_webhook() {
  if ! command -v systemd-run >/dev/null 2>&1; then
    printf 'systemd-run not found; register the webhook later (see the README).\n' >&2
    return
  fi
  printf 'Registering the Telegram webhook...\n'
  if ! systemd-run --wait --pipe --collect -p "EnvironmentFile=${ENV_FILE}" "$BINARY_PATH" webhook set; then
    printf 'warning: webhook registration failed; the service is installed, retry with:\n' >&2
    printf '  sudo systemd-run --wait --pipe --collect -p EnvironmentFile=%s %s webhook set\n' "$ENV_FILE" "$BINARY_PATH" >&2
  fi
}

[[ ${EUID} -eq 0 ]] || die "run this installer as root (for example: curl ... | sudo bash)"
[[ $(uname -s) == Linux ]] || die "this installer supports Linux/systemd hosts"
require_command curl
require_command systemctl
require_command install
require_command tar
require_command cmp

GENERATED_GITHUB_SECRET=
target=$(resolve_target)
source_arg=${DEVY_VERSION:-${1:-latest}}

case $source_arg in
  https://github.com/*) release_url=$source_arg ;;
  https://*) die "release URL must be an https://github.com/ URL" ;;
  latest)
    tag=$(latest_tag)
    release_url="https://github.com/${REPO}/releases/download/${tag}/devy-${tag}-${target}.tar.gz"
    ;;
  v[0-9]*)
    release_url="https://github.com/${REPO}/releases/download/${source_arg}/devy-${source_arg}-${target}.tar.gz"
    ;;
  *) die "unrecognised release '${source_arg}' (expected 'latest', a tag like v0.1.0, or a github.com URL)" ;;
esac

work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

asset=$(download_release "$release_url" "$work_dir")
extract_dir="${work_dir}/release"
mkdir "$extract_dir"
tar -xzf "$asset" -C "$extract_dir" --strip-components=1 || die "could not unpack ${asset##*/}"

candidate_binary="${extract_dir}/devy"
[[ -f $candidate_binary ]] || die "release archive does not contain the devy binary"
chmod 0755 "$candidate_binary"
"$candidate_binary" --version >/dev/null 2>&1 || die "devy is not executable on this host; check the architecture ($(uname -m))"

fresh_config=false
if [[ ! -f $ENV_FILE ]]; then
  fresh_config=true
  write_initial_config
else
  printf 'Keeping the existing configuration at %s\n' "$ENV_FILE"
  ensure_required_config
fi

systemctl stop "$SERVICE_NAME" 2>/dev/null || true
backup_created=false
if [[ -f $BINARY_PATH ]] && ! cmp -s "$candidate_binary" "$BINARY_PATH"; then
  cp -p "$BINARY_PATH" "${BINARY_PATH}.previous"
  backup_created=true
fi
install -m 0755 -o root -g root "$candidate_binary" "${BINARY_PATH}.new"
mv -f "${BINARY_PATH}.new" "$BINARY_PATH"

write_service_unit
systemctl daemon-reload
systemctl enable "$SERVICE_NAME" >/dev/null 2>&1
if ! systemctl restart "$SERVICE_NAME" || ! wait_until_active; then
  if [[ $backup_created == true && -f ${BINARY_PATH}.previous ]]; then
    printf 'Service failed to start; restoring the previous binary.\n' >&2
    install -m 0755 -o root -g root "${BINARY_PATH}.previous" "$BINARY_PATH"
    systemctl restart "$SERVICE_NAME" || true
  fi
  die "devy failed to start; inspect it with: journalctl -u devy -e"
fi

if [[ $fresh_config == true ]]; then
  register_telegram_webhook
fi

printf '\nInstalled %s to %s\n' "$("$BINARY_PATH" --version)" "$BINARY_PATH"
printf 'Configuration: %s\n' "$ENV_FILE"
printf 'Status:        systemctl status devy\n'
printf 'Logs:          journalctl -u devy -f\n'
if [[ -n $GENERATED_GITHUB_SECRET ]]; then
  printf '\nGitHub webhook: https://<host>/webhook/github, content type application/json, secret:\n  %s\n' "$GENERATED_GITHUB_SECRET"
fi
printf '\nDevy listens on 127.0.0.1:8000: put a TLS reverse proxy (Caddy, nginx, ...) in front of it.\n'
