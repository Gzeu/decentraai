#!/usr/bin/env bash
# DecentraAI — local text-to-speech setup (Piper VITS, Romanian-capable).
#
# Installs the Python venv + voice files the node needs for the chat speak
# button (feature P13). Everything lives under <data_dir>/tts/ so the node
# subprocess can find it without sudo:
#   <data_dir>/tts/venv/                  uv-managed virtualenv (cpython 3.13)
#   <data_dir>/tts/models/piper-ro/       Romanian Piper voices (.onnx + .json)
#
# Piper (VITS + embedded espeak-ng) supports Romanian natively with
# diacritics — voices available:
#   ro_RO-raluca-high   female, WER 2.2%  (default)
#   ro_RO-lili-high     female, WER 2.7%
#   ro_RO-mihai-medium  male,  WER ~4%
#
# Idempotent: rerunning only fills gaps (existing venv/deps/models are kept).
#
# Run from the repository root:
#   bash scripts/setup-tts.sh [--data-dir PATH] [--voice ro_RO-raluca-high]
set -euo pipefail

cd "$(dirname "$0")/.."

DATA_DIR="${DECENTRAI_DATA_DIR:-$HOME/.decentraai}"
VOICE="ro_RO-raluca-high"
while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) DATA_DIR="${2:?usage: --data-dir PATH}"; shift 2 ;;
    --voice) VOICE="${2:?usage: --voice ro_RO-raluca-high}"; shift 2 ;;
    --help|-h) echo "usage: bash scripts/setup-tts.sh [--data-dir PATH] [--voice ro_RO-raluca-high]"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

TTS_DIR="$DATA_DIR/tts"
VENV_DIR="$TTS_DIR/venv"
VOICES_DIR="$TTS_DIR/models/piper-ro"

if [ ! -d "$DATA_DIR" ]; then
  echo "error: data dir $DATA_DIR does not exist (run \`decentraai setup\` first)" >&2
  exit 1
fi
mkdir -p "$VOICES_DIR"

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

echo "==> installing piper-tts (embeds espeak-ng — no sudo needed)"
# piper-tts 1.4+ is a single wheel with espeak-ng embedded, so Romanian
# phonemization works out of the box without system packages.
"$UV" pip install --python "$VENV_DIR/bin/python" "piper-tts>=1.4"

MODEL="$VOICES_DIR/$VOICE.onnx"
CONFIG="$VOICES_DIR/$VOICE.onnx.json"
if [ ! -f "$MODEL" ] || [ ! -s "$MODEL" ]; then
  echo "==> downloading $VOICE.onnx"
  curl -sL --fail -o "$MODEL" \
    "https://huggingface.co/eduardem/piper-tts-romanian/resolve/main/voices/$VOICE/$VOICE.onnx" \
    || curl -sL --fail -o "$MODEL" \
      "https://huggingface.co/rhasspy/piper-voices/resolve/main/ro/ro_RO/$VOICE/medium/$VOICE.onnx"
fi
if [ ! -f "$CONFIG" ] || [ ! -s "$CONFIG" ]; then
  echo "==> downloading $VOICE.onnx.json"
  curl -sL --fail -o "$CONFIG" \
    "https://huggingface.co/eduardem/piper-tts-romanian/resolve/main/voices/$VOICE/$VOICE.onnx.json" \
    || curl -sL --fail -o "$CONFIG" \
      "https://huggingface.co/rhasspy/piper-voices/resolve/main/ro/ro_RO/$VOICE/medium/$VOICE.onnx.json"
fi

echo "==> smoke test (Romanian with diacritics)"
VENV_SITE="$VENV_DIR/lib/python3.13/site-packages" \
MODEL_PATH="$MODEL" \
CONFIG_PATH="$CONFIG" \
"$VENV_DIR/bin/python" - <<'PY'
import os, sys
sys.path.insert(0, os.environ["VENV_SITE"])
from piper import PiperVoice
v = PiperVoice.load(os.environ["MODEL_PATH"], config_path=os.environ["CONFIG_PATH"])
chunks = list(v.synthesize("Bună ziua! Fabricul DecentraAI vorbește română corect: ă, â, î, ș, ț."))
dur = sum(len(c.audio_int16_bytes) / (c.sample_rate * c.sample_width) for c in chunks)
print(f"   ok: {dur:.2f}s of Romanian speech at {chunks[0].sample_rate} Hz")
PY

echo
echo "TTS ready. Enable it in $DATA_DIR/node.yaml:"
echo
echo "  tts:"
echo "    enabled: true"
echo "    voice: \"$VOICE\""
echo "    speed: 1.0"
echo
echo "Then restart the node:  systemctl --user restart decentraai-node"