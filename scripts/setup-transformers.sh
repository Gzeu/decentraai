#!/usr/bin/env bash
# DecentraAI — Transformers inference backend setup.
#
# Installs the Python venv the node needs for the Transformers backend
# (local embeddings and/or LLM inference via sentence-transformers or
# HuggingFace Transformers). Everything lives under
# <data_dir>/tools/transformers/ so the node subprocess can find it
# without sudo:
#   <data_dir>/tools/transformers/venv/    uv-managed virtualenv
#   <data_dir>/tools/transformers/models/  HF cache for models
#
# Models:
#   sentence-transformers/all-MiniLM-L6-v2  — 384-dim embeddings (CPU-friendly)
#
# Model downloads into the HF cache — point HF_HOME at the dir above
# so everything stays under the data dir.
#
# Idempotent: rerunning only fills gaps (existing venv/deps/models are kept).
#
# Run from the repository root:
#   bash scripts/setup-transformers.sh [--data-dir PATH]
set -euo pipefail

cd "$(dirname "$0")/.."

DATA_DIR="${DECENTRAI_DATA_DIR:-$HOME/.decentraai}"
MODEL_ID="sentence-transformers/all-MiniLM-L6-v2"
while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) DATA_DIR="${2:?usage: --data-dir PATH}"; shift 2 ;;
    --model)    MODEL_ID="${2:?usage: --model MODEL_ID}"; shift 2 ;;
    --help|-h) echo "usage: bash scripts/setup-transformers.sh [--data-dir PATH] [--model MODEL_ID]"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

TX_DIR="$DATA_DIR/tools/transformers"
VENV_DIR="$TX_DIR/venv"
MODELS_DIR="$TX_DIR/models"

if [ ! -d "$DATA_DIR" ]; then
  echo "error: data dir $DATA_DIR does not exist (run \`decentraai setup\` first)" >&2
  exit 1
fi

UV="$(command -v uv || echo "$HOME/.local/bin/uv")"
if [ ! -x "$UV" ]; then
  echo "error: uv not found (install with: curl -LsSf https://astral.sh/uv/install.sh | sh)" >&2
  exit 1
fi

mkdir -p "$TX_DIR" "$MODELS_DIR"

if [ ! -x "$VENV_DIR/bin/python" ]; then
  echo "==> creating virtualenv"
  "$UV" venv "$VENV_DIR" >/dev/null
fi

echo "==> installing transformers + torch + sentence-transformers (CPU)"
"$UV" pip install --python "$VENV_DIR/bin/python" \
  "transformers>=4.44" \
  "torch>=2.2" \
  "sentence-transformers>=3.0" \
  "accelerate>=0.30" \
  "sentencepiece>=0.2"

echo "==> downloading model: $MODEL_ID"
HF_HOME="$MODELS_DIR" "$VENV_DIR/bin/python" -c "
import os
os.environ['HF_HOME'] = '$MODELS_DIR'
model_id = '$MODEL_ID'
print(f'   downloading {model_id}...')
from sentence_transformers import SentenceTransformer
m = SentenceTransformer(model_id)
dim = m.get_sentence_embedding_dimension()
print(f'   ok: {model_id} loaded, dim={dim}')
"

echo
echo "Transformers backend ready. Enable it in $DATA_DIR/node.yaml:"
echo
echo "  transformers:"
echo "    enabled: true"
echo "    model: \"$MODEL_ID\""
echo "    device: cpu"
echo ""
echo "  inference:"
echo "    engine: \"transformers\""
echo "    embeddings_backend_url: \"http://127.0.0.1:<transformers-port>\""
echo ""
echo "Then restart the node:  systemctl --user restart decentraai-node"
