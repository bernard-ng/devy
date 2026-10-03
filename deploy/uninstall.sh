#!/usr/bin/env bash
# Removes the Devy systemd service installed by deploy/install.sh.
#
#   curl -fsSL https://raw.githubusercontent.com/bernard-ng/devy/main/deploy/uninstall.sh | sudo bash
#   sudo ./deploy/uninstall.sh [--yes] [--keep-config] [--delete-webhook]
set -Eeuo pipefail

BINARY_PATH=/usr/local/bin/devy
CONFIG_DIR=/etc/devy
ENV_FILE="${CONFIG_DIR}/devy.env"
SERVICE_NAME=devy.service
UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  die "usage: $0 [--yes] [--keep-config] [--delete-webhook]"
}

confirm_removal() {
  local answer=
  printf 'This will remove the Devy service and binary' >&2
  if [[ $keep_config == true ]]; then
    printf ' (configuration in %s is kept).\n' "$CONFIG_DIR" >&2
  else
    printf ', and permanently delete its configuration in %s.\n' "$CONFIG_DIR" >&2
  fi
  read -r -p 'Continue? [y/N] ' answer </dev/tty || die "cannot read confirmation from /dev/tty (use --yes for unattended removal)"
  [[ $answer == [yY] || $answer == [yY][eE][sS] ]] || {
    printf 'Uninstall cancelled.\n'
    exit 0
  }
}

assume_yes=false
keep_config=false
delete_webhook=false
for arg in "$@"; do
  case $arg in
    --yes | -y) assume_yes=true ;;
    --keep-config) keep_config=true ;;
    --delete-webhook) delete_webhook=true ;;
    *) usage ;;
  esac
done

[[ ${EUID} -eq 0 ]] || die "run this uninstaller as root (for example: curl ... | sudo bash)"
[[ $(uname -s) == Linux ]] || die "this uninstaller supports Linux/systemd hosts"
command -v systemctl >/dev/null 2>&1 || die "required command not found: systemctl"

if [[ $assume_yes != true ]]; then
  confirm_removal
fi

# Needs the binary and the env file, so it has to happen before they are removed.
if [[ $delete_webhook == true ]]; then
  if [[ -x $BINARY_PATH && -f $ENV_FILE ]] && command -v systemd-run >/dev/null 2>&1; then
    printf 'Deleting the Telegram webhook...\n'
    systemd-run --wait --pipe --collect -p "EnvironmentFile=${ENV_FILE}" "$BINARY_PATH" webhook delete \
      || printf 'warning: could not delete the Telegram webhook; run `devy webhook delete` manually.\n' >&2
  else
    printf 'warning: cannot delete the Telegram webhook (binary, config or systemd-run missing).\n' >&2
  fi
fi

printf 'Stopping and disabling devy...\n'
systemctl disable --now "$SERVICE_NAME" >/dev/null 2>&1 || true
rm -f -- "$UNIT_PATH"
systemctl daemon-reload
systemctl reset-failed "$SERVICE_NAME" >/dev/null 2>&1 || true

printf 'Removing the devy binary...\n'
rm -f -- "$BINARY_PATH" "${BINARY_PATH}.previous" "${BINARY_PATH}.new"

if [[ $keep_config == true ]]; then
  printf 'Keeping %s\n' "$CONFIG_DIR"
else
  printf 'Removing the configuration...\n'
  rm -rf -- "$CONFIG_DIR"
fi

printf 'Devy uninstalled.\n'
if [[ $delete_webhook != true ]]; then
  printf 'Telegram will keep calling the webhook URL until it is deleted (devy webhook delete).\n'
fi
