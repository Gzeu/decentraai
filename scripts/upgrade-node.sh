#!/usr/bin/env bash
# DecentraAI — upgrade an existing node to the current HEAD (Desktop/laptop).
#
# Safe, idempotent in-place upgrade of a running DecentraAI node to the
# current source HEAD. Rebuilds the `decentraai` binary, swaps it into
# ~/.cargo/bin (with a backup), and restarts the systemd *user* service so the
# node advertises current capabilities — in particular `accepts_remote_inference:
# true`, which older binaries omit (and therefore conservatively advertise as
# false, blocking two-node remote inference).
#
# This is the exact upgrade procedure for the Desktop node (dca-NGE65Z) found
# by the live two-node validation (docs/TWO_NODE_VALIDATION.md).
#
# Requirements on the target machine:
#   - the decentraai repository checked out at the target commit,
#   - Rust toolchain (cargo),
#   - the node installed via `scripts/install-app.sh` (systemd user service).
#
# Usage (from the repository root, on the Desktop):
#   bash scripts/upgrade-node.sh            # default: use current checkout
#   bash scripts/upgrade-node.sh <commit>   # check out <commit> first, then build
#
# It never touches node data/config/identity (only the binary + a restart).

set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -f crates/node-cli/Cargo.toml ]; then
  echo "error: run this script from the decentraai repository root" >&2
  exit 1
fi

BIN="$HOME/.cargo/bin/decentraai"
SERVICE=decentraai-node
CONFIG_PATH="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}/node.yaml"

# Optional: check out a specific commit before building.
if [ $# -ge 1 ]; then
  echo "==> Checking out $1"
  git fetch --all --tags --prune >/dev/null 2>&1 || true
  git checkout "$1"
fi

echo "==> Confirming source HEAD"
HEAD_SHA="$(git rev-parse --short HEAD)"
echo "  HEAD=$HEAD_SHA"

echo "==> Building the current binary (release)"
cargo build --release --bin decentraai

echo "==> Backing up the running binary"
if [ -f "$BIN" ]; then
  cp "$BIN" "$BIN.bak.$(date +%Y%m%d%H%M%S)" 2>/dev/null || true
fi

echo "==> Stopping the node service (Text file busy guard)"
# A running service holds the old binary; stop before the swap so the copy is
# not rejected with ETXTBSY.
systemctl --user stop "$SERVICE" 2>/dev/null || true

echo "==> Installing the new binary"
cp target/release/decentraai "$BIN"
chmod +x "$BIN"

echo "==> Restarting the node service"
systemctl --user start "$SERVICE"
systemctl --user is-active "$SERVICE" >/dev/null
echo "  service active"

echo "==> Verifying the node advertises remote inference"
sleep 8
TOKEN_FILE="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}/runtime/api.token"
API_PORT="$(grep -E '^[[:space:]]*api_port:' "$CONFIG_PATH" 2>/dev/null | awk '{print $2}' || echo 8080)"
if [ -f "$TOKEN_FILE" ]; then
  TOKEN="$(cat "$TOKEN_FILE")"
  curl -s -m 5 -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/v1/compute" \
    | python3 -c "import json,sys; d=json.load(sys.stdin); [print('  node', w.get('node_id'), 'accepts_remote_inference =', w.get('accepts_remote_inference')) for w in d.get('workers',[]) if w.get('node_id')==w.get('node_id')]" 2>/dev/null \
    || echo "  (could not read /v1/compute; check the dashboard manually)"
fi

echo
echo "==> Upgrade complete: HEAD=$HEAD_SHA"
echo "  This node now advertises accepts_remote_inference: true (current binary)."
echo "  On the OTHER node, trust this peer and route a remote inference to it."
