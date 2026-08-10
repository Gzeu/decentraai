#!/usr/bin/env bash
# DecentraAI installer: builds and installs the `decentraai` CLI and,
# optionally, llama.cpp's llama-server for local inference.
# Run from the repository root:  bash scripts/install.sh [--no-llama]
set -euo pipefail

if [ ! -f crates/node-cli/Cargo.toml ]; then
  echo "error: run this script from the decentraai repository root" >&2
  exit 1
fi

LLAMA_CPP_DIR="${LLAMA_CPP_DIR:-$HOME/llama.cpp}"
BUILD_LLAMA=1
for arg in "$@"; do
  case "$arg" in
    --no-llama) BUILD_LLAMA=0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

echo "==> Checking Rust toolchain"
if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found; installing rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

echo "==> Installing the decentraai CLI"
cargo install --path crates/node-cli

if [ "$BUILD_LLAMA" -eq 1 ]; then
  echo "==> Building llama.cpp (llama-server) in $LLAMA_CPP_DIR"
  if [ ! -d "$LLAMA_CPP_DIR/.git" ]; then
    git clone https://github.com/ggml-org/llama.cpp "$LLAMA_CPP_DIR"
  fi
  cmake -S "$LLAMA_CPP_DIR" -B "$LLAMA_CPP_DIR/build"
  cmake --build "$LLAMA_CPP_DIR/build" --config Release --target llama-server -j
  echo
  echo "Add to your shell profile:"
  echo "  export DECENTRAAI_LLAMA_SERVER=$LLAMA_CPP_DIR/build/bin/llama-server"
fi

echo "==> Initializing node data"
decentraai init

cat <<'EOF'

Done. Next steps:
  decentraai doctor                                  # hardware check
  decentraai registry scan --directory ~/models      # index your GGUFs
  decentraai swarm start                             # share models on the LAN
  decentraai serve start --model <name>.gguf         # inference + dashboard :8080
EOF
