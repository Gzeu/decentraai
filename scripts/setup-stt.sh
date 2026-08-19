#!/usr/bin/env bash
# DecentraAI — local STT setup (faster-whisper, CTranslate2).
#
# Installs the Python venv the node needs for `/v1/stt` (Tool Runtime STT).
# Everything lives under <data_dir>/tools/stt/ so the node subprocess can find
# it without sudo:
#   <data_dir>/tools/stt/venv/                  uv-managed virtualenv
#   <data_dir>/tools/stt/models/                HF cache for whisper models
#
# faster-whisper uses CTranslate2 (CPU, int8). The model (tiny/base/small/
# medium/large-v3) downloads on first use into the HF cache — point HF_HOME at
# the dir above so everything stays under the data dir.
#
# Idempotent: rerunning only fills gaps (existing venv/deps/models are kept).
#
# Run from the repository root:
#   bash scripts/setup-stt.sh [--data-dir PATH] [--model base]
set -euo pipefail

cd "$(dirname "$0")/.."

DATA_DIR="${DECENTRAI_DATA_DIR:-$HOME/.decentraai}"
MODEL="base"
while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) DATA_DIR="${2:?usage: --data-dir PATH}"; shift 2 ;;
    --model) MODEL="${2:?usage: --model tiny|base|small|medium|large-v3}"; shift 2 ;;
    --help|-h) echo "usage: bash scripts/setup-stt.sh [--data-dir PATH] [--model base]"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

STT_DIR="$DATA_DIR/tools/stt"
VENV_DIR="$STT_DIR/venv"
MODELS_DIR="$STT_DIR/models"

if [ ! -d "$DATA_DIR" ]; then
  echo "error: data dir $DATA_DIR does not exist (run \`decentraai setup\` first)" >&2
  exit 1
fi

UV="$(command -v uv || echo "$HOME/.local/bin/uv")"
if [ ! -x "$UV" ]; then
  echo "error: uv not found (install with: curl -LsSf https://astral.sh/uv/install.sh | sh)" >&2
  exit 1
fi

mkdir -p "$STT_DIR" "$MODELS_DIR"

if [ ! -x "$VENV_DIR/bin/python" ]; then
  echo "==> creating virtualenv"
  "$UV" venv "$VENV_DIR" >/dev/null
fi

echo "==> installing faster-whisper (CTranslate2, CPU int8)"
"$UV" pip install --python "$VENV_DIR/bin/python" "faster-whisper>=1.0"

echo "==> preloading model '$MODEL' (first download may take a while)"
HF_HOME="$MODELS_DIR" "$VENV_DIR/bin/python" - "$MODEL" <<'PY'
import sys
from faster_whisper import WhisperModel
name = sys.argv[1]
_ = WhisperModel(name, device="cpu", compute_type="int8")
print(f"   ok: faster-whisper '{name}' loaded")
PY

echo
echo "STT ready. Enable it in $DATA_DIR/node.yaml:"
echo
echo "  stt:"
echo "    enabled: true"
echo "    model: \"$MODEL\""
echo
echo "Then restart the node:  systemctl --user restart decentraai-node"