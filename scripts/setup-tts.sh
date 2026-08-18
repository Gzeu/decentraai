#!/usr/bin/env bash
# DecentraAI — local text-to-speech setup (Kokoro-82M ONNX).
#
# Installs the Python venv + model files the node needs for the chat speak
# button (feature P13). Everything lives under <data_dir>/tts/ so the node
# subprocess can find it without sudo:
#   <data_dir>/tts/venv/           uv-managed virtualenv (cpython 3.13)
#   <data_dir>/tts/models/         kokoro-v1.0.onnx + voices-v1.0.bin
#
# Idempotent: rerunning only fills gaps (existing venv/deps/models are kept).
#
# Run from the repository root:  bash scripts/setup-tts.sh [--data-dir PATH]
set -euo pipefail

cd "$(dirname "$0")/.."

DATA_DIR="${DECENTRAI_DATA_DIR:-$HOME/.decentraai}"
for arg in "$@"; do
  case "$arg" in
    --data-dir) DATA_DIR="${2:?usage: --data-dir PATH}"; shift 2 ;;
    --help|-h) echo "usage: bash scripts/setup-tts.sh [--data-dir PATH]"; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

TTS_DIR="$DATA_DIR/tts"
VENV_DIR="$TTS_DIR/venv"
MODELS_DIR="$TTS_DIR/models"

if [ ! -d "$DATA_DIR" ]; then
  echo "error: data dir $DATA_DIR does not exist (run \`decentraai setup\` first)" >&2
  exit 1
fi
mkdir -p "$MODELS_DIR"

UV="$(command -v uv || echo "$HOME/.local/bin/uv")"
if [ ! -x "$UV" ]; then
  echo "error: uv not found. Install it (https://docs.astral.sh/uv) then rerun." >&2
  exit 1
fi

echo "==> TTS dir: $TTS_DIR"

if [ ! -x "$VENV_DIR/bin/python" ]; then
  echo "==> creating virtualenv"
  "$UV" venv "$VENV_DIR" >/dev/null
fi

echo "==> installing kokoro-onnx + onnxruntime + soundfile + espeakng-loader"
# espeakng-loader ships the espeak-ng binaries inside the wheel, so no system
# apt install / sudo is needed for Kokoro's G2P phonemizer.
"$UV" pip install --python "$VENV_DIR/bin/python" \
  kokoro-onnx onnxruntime soundfile espeakng-loader

MODEL="$MODELS_DIR/kokoro-v1.0.onnx"
VOICES="$MODELS_DIR/voices-v1.0.bin"
if [ ! -f "$MODEL" ] || [ ! -s "$MODEL" ]; then
  echo "==> downloading kokoro-v1.0.onnx (~325 MB)"
  curl -sL --fail -o "$MODEL" \
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx"
fi
if [ ! -f "$VOICES" ] || [ ! -s "$VOICES" ]; then
  echo "==> downloading voices-v1.0.bin (~28 MB)"
  curl -sL --fail -o "$VOICES" \
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin"
fi

echo "==> smoke test"
VENV_SITE="$VENV_DIR/lib/python3.13/site-packages" \
MODEL_PATH="$MODEL" \
VOICES_PATH="$VOICES" \
"$VENV_DIR/bin/python" - <<'PY'
import os, sys
sys.path.insert(0, os.environ["VENV_SITE"])
from kokoro_onnx import Kokoro
k = Kokoro(os.environ["MODEL_PATH"], os.environ["VOICES_PATH"])
samples, sr = k.create("DecentraAI voice online.", voice="af_heart", speed=1.0)
print(f"   ok: {len(samples)/sr:.2f}s of speech at {sr} Hz")
PY

echo
echo "TTS ready. Enable it in $DATA_DIR/node.yaml:"
echo
echo "  tts:"
echo "    enabled: true"
echo "    voice: \"af_heart\""
echo "    speed: 1.0"
echo
echo "Then restart the node:  systemctl --user restart decentraai-node"