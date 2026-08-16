#!/usr/bin/env bash
# DecentraAI — desktop application installer (Ubuntu first).
#
# Turns this machine into a DecentraAI node with the normal-user flow:
#   Install -> Open -> Ready.
# It builds/installs the `decentraai` binary, runs the onboarding wizard
# (detect hardware -> identity -> model -> validated config), installs a
# systemd *user* service so the node auto-starts on login and survives
# reboot (with lingering), and adds a desktop launcher.
#
# Run from the repository root:  bash scripts/install-app.sh [--no-llama]
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -f crates/node-cli/Cargo.toml ]; then
  echo "error: run this script from the decentraai repository root" >&2
  exit 1
fi

BUILD_LLAMA=1
for arg in "$@"; do
  case "$arg" in
    --no-llama) BUILD_LLAMA=0 ;;
    --help|-h) echo "usage: bash scripts/install-app.sh [--no-llama]"; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

USER_SYSTEMD="$HOME/.config/systemd/user"
SERVICE=decentraai-node
DATA_DIR="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}"
CONFIG_PATH="$DATA_DIR/node.yaml"
DESKTOP="$HOME/.local/share/applications/decentraai.desktop"

echo "==> Checking Rust toolchain"
if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found; installing rustup... (restart your shell if this adds one)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

if [ "$BUILD_LLAMA" -eq 1 ]; then
  echo "==> Building llama.cpp (llama-server) for local inference"
  LLAMA_CPP_DIR="${LLAMA_CPP_DIR:-$HOME/llama.cpp}"
  if [ ! -d "$LLAMA_CPP_DIR/.git" ]; then
    git clone --depth 1 https://github.com/ggml-org/llama.cpp "$LLAMA_CPP_DIR"
  fi
  cmake -S "$LLAMA_CPP_DIR" -B "$LLAMA_CPP_DIR/build" >/dev/null
  cmake --build "$LLAMA_CPP_DIR/build" --config Release --target llama-server -j >/dev/null
  export DECENTRAAI_LLAMA_SERVER="$LLAMA_CPP_DIR/build/bin/llama-server"
fi

echo "==> Installing the decentraai CLI"
cargo install --path crates/node-cli --force >/dev/null
echo "  (standalone lightweight worker: decentraai-worker --name <n> --model <file.gguf>)"
BIN="$HOME/.cargo/bin/decentraai"

echo "==> Onboarding (detect hardware -> identity -> model -> validated config)"
mkdir -p "$DATA_DIR"
if [ ! -f "$CONFIG_PATH" ]; then
  "$BIN" setup --data-dir "$DATA_DIR" --config "$CONFIG_PATH"
else
  echo "  existing config found at $CONFIG_PATH; leaving it untouched"
fi

echo "==> Installing systemd user service ($SERVICE)"
mkdir -p "$USER_SYSTEMD"
sed "s#%h#$HOME#g" deploy/decentraai-node.service > "$USER_SYSTEMD/$SERVICE.service"
systemctl --user daemon-reload
systemctl --user enable "$SERVICE" >/dev/null 2>&1 || true
systemctl --user restart "$SERVICE" 2>/dev/null || true

echo "==> Enabling auto-start after reboot (user lingering)"
if command -v loginctl >/dev/null 2>&1; then
  sudo loginctl enable-linger "$USER" 2>/dev/null || true
fi

echo "==> Adding desktop launcher"
mkdir -p "$(dirname "$DESKTOP")"
sed "s#@HOME@#$HOME#g" deploy/decentraai.desktop > "$DESKTOP"
chmod +x "$DESKTOP" 2>/dev/null || true

PORT="${DECENTRAAI_PORT:-8080}"
cat <<EOF

=== DecentraAI is installed and running ===
  Node status : systemctl --user status $SERVICE
  Dashboard   : http://127.0.0.1:$PORT/
  Logs        : journalctl --user -u $SERVICE -f
  Stop        : systemctl --user stop $SERVICE
  Start       : systemctl --user start $SERVICE
  Restart     : systemctl --user restart $SERVICE
  Worker      : decentraai-worker --name <worker-name> --model <model.gguf>  (join a fabric without the control plane)
  Uninstall   : bash scripts/uninstall-app.sh

Open the launcher "DecentraAI" from the applications menu, or run:
  decentraai open
EOF