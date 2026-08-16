#!/usr/bin/env bash
# DecentraAI — clean uninstall of the desktop application.
#
# Stops and disables the node service, removes the systemd unit, the desktop
# launcher, and (optionally, with --purge) the node's data dir. By default it
# keeps ~/.decentraai (identity, models, config) so re-install is instant.
set -euo pipefail

cd "$(dirname "$0")/.."

SERVICE=decentraai-node
USER_SYSTEMD="$HOME/.config/systemd/user"
DATA_DIR="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}"
DESKTOP="$HOME/.local/share/applications/decentraai.desktop"

PURGE=0
for arg in "$@"; do
  case "$arg" in
    --purge) PURGE=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

echo "==> Stopping and disabling the node service"
systemctl --user stop "$SERVICE" 2>/dev/null || true
systemctl --user disable "$SERVICE" 2>/dev/null || true

echo "==> Removing systemd unit and launcher"
rm -f "$USER_SYSTEMD/$SERVICE.service"
rm -f "$DESKTOP"
systemctl --user daemon-reload 2>/dev/null || true
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

echo "==> Removing binaries"
rm -f "$HOME/.cargo/bin/decentraai" "$HOME/.cargo/bin/decentraai-worker"

if [ "$PURGE" -eq 1 ]; then
  echo "==> Purging node data ($DATA_DIR)"
  rm -rf "$DATA_DIR"
fi

if command -v loginctl >/dev/null 2>&1; then
  sudo loginctl disable-linger "$USER" 2>/dev/null || true
fi

echo "DecentraAI uninstalled."
if [ "$PURGE" -ne 1 ]; then
  echo "Your node data (identity, models, config) was kept at $DATA_DIR."
  echo "Re-run bash scripts/install-app.sh to re-install instantly, or run"
  echo "bash scripts/uninstall-app.sh --purge to remove all data too."
fi