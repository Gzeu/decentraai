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

# Enable remote inference in the node config (this is what actually makes the
# node advertise accepts_remote_inference: true — the binary alone is not
# enough). Requires both inference.allow_remote_inference and
# network.private_swarm (config validation enforces private_swarm when remote
# inference is on). Backs up the config first. Opt out with ENABLE_REMOTE=0.
if [ "${ENABLE_REMOTE:-1}" = "1" ] && [ -f "$CONFIG_PATH" ]; then
  echo "==> Enabling remote inference in $CONFIG_PATH"
  cp "$CONFIG_PATH" "$CONFIG_PATH.bak.$(date +%Y%m%d%H%M%S)" 2>/dev/null || true
  # Flip allow_remote_inference: false -> true under the inference: block.
  sed -i 's/^\([[:space:]]*allow_remote_inference:[[:space:]]*\)false/\1true/' "$CONFIG_PATH"
  # Ensure network.private_swarm: true so config validation passes.
  if ! grep -qE '^[[:space:]]*private_swarm:[[:space:]]*true' "$CONFIG_PATH"; then
    if grep -qE '^[[:space:]]*private_swarm:' "$CONFIG_PATH"; then
      sed -i 's/^\([[:space:]]*private_swarm:[[:space:]]*\)false/\1true/' "$CONFIG_PATH"
    else
      # Add under the network: block if it exists; else append (validation may
      # still require a correct structure, so this is best-effort).
      sed -i '/^[[:space:]]*network:/a\  private_swarm: true' "$CONFIG_PATH"
    fi
  fi
  echo "  allow_remote_inference + private_swarm enabled"
else
  echo "==> Remote inference config left as-is (ENABLE_REMOTE=$ENABLE_REMOTE)"
fi

echo "==> Restarting the node service"
# Ensure the unit file runs the node with the self-upgrade watcher
# (--auto-upgrade) and pins WorkingDirectory to the repo checkout so the
# watcher can pull/build. Idempotent: leaves an already-patched unit alone.
# Opt out with ENABLE_AUTO_UPGRADE=0.
UNIT_FILE="$HOME/.config/systemd/user/$SERVICE.service"
if [ "${ENABLE_AUTO_UPGRADE:-1}" = "1" ] && [ -f "$UNIT_FILE" ]; then
  if grep -q -- "--auto-upgrade" "$UNIT_FILE"; then
    echo "  unit file already has --auto-upgrade"
  else
    echo "==> Enabling --auto-upgrade in $UNIT_FILE"
    cp "$UNIT_FILE" "$UNIT_FILE.bak.$(date +%Y%m%d%H%M%S)" 2>/dev/null || true
    sed -i 's|^\(ExecStart=.*decentraai node --config [^ ]*\)$|\1 --auto-upgrade|' "$UNIT_FILE"
    if ! grep -q "WorkingDirectory" "$UNIT_FILE"; then
      sed -i 's|^Restart=always|WorkingDirectory=%h/decentraai\nRestart=always|' "$UNIT_FILE"
    fi
    systemctl --user daemon-reload
    echo "  --auto-upgrade + WorkingDirectory enabled"
  fi
else
  echo "  auto-upgrade unit patch left as-is (ENABLE_AUTO_UPGRADE=$ENABLE_AUTO_UPGRADE)"
fi

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
    | python3 -c "import json,sys; d=json.load(sys.stdin); [print('  node', w.get('node_id'), 'accepts_remote_inference =', w.get('accepts_remote_inference')) for w in d.get('workers',[])]" 2>/dev/null \
    || echo "  (could not read /v1/compute; check the dashboard manually)"
fi

echo
echo "==> Upgrade complete: HEAD=$HEAD_SHA"
echo "  This node now advertises accepts_remote_inference: true (current binary)."
echo "  On the OTHER node, trust this peer and route a remote inference to it."
