#!/usr/bin/env bash
# DecentraAI — local HF skills setup (small transformers pipelines).
#
# Installs the Python venv the node needs for `/v1/skills/<id>` (Tool Runtime
# HF skills). Everything lives under <data_dir>/tools/skills/ so the node
# subprocess can find it without sudo:
#   <data_dir>/tools/skills/venv/                  uv-managed virtualenv
#   <data_dir>/tools/skills/models/                HF cache for pipeline models
#
# Skills (all CPU-friendly, tiny models):
#   sentiment       distilbert-base-uncased-finetuned-sst-2-english
#   ner             dslim/bert-base-NER
#   summarize       sshleifer/distilbart-cnn-12-6
#   translate_ro_en Helsinki-NLP/opus-mt-ro-en
#   translate_en_ro Helsinki-NLP/opus-mt-en-ro
#
# Models download on first use into the HF cache — point HF_HOME at the dir
# above so everything stays under the data dir.
#
# Idempotent: rerunning only fills gaps (existing venv/deps/models are kept).
#
# Run from the repository root:
#   bash scripts/setup-skills.sh [--data-dir PATH]
set -euo pipefail

cd "$(dirname "$0")/.."

DATA_DIR="${DECENTRAI_DATA_DIR:-$HOME/.decentraai}"
while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) DATA_DIR="${2:?usage: --data-dir PATH}"; shift 2 ;;
    --help|-h) echo "usage: bash scripts/setup-skills.sh [--data-dir PATH]"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

SKILLS_DIR="$DATA_DIR/tools/skills"
VENV_DIR="$SKILLS_DIR/venv"
MODELS_DIR="$SKILLS_DIR/models"

if [ ! -d "$DATA_DIR" ]; then
  echo "error: data dir $DATA_DIR does not exist (run \`decentraai setup\` first)" >&2
  exit 1
fi

UV="$(command -v uv || echo "$HOME/.local/bin/uv")"
if [ ! -x "$UV" ]; then
  echo "error: uv not found (install with: curl -LsSf https://astral.sh/uv/install.sh | sh)" >&2
  exit 1
fi

mkdir -p "$SKILLS_DIR" "$MODELS_DIR"

if [ ! -x "$VENV_DIR/bin/python" ]; then
  echo "==> creating virtualenv"
  "$UV" venv "$VENV_DIR" >/dev/null
fi

echo "==> installing transformers + torch (CPU)"
"$UV" pip install --python "$VENV_DIR/bin/python" "transformers>=4.44" "torch>=2.2" "sentencepiece>=0.2"

echo "==> smoke test: sentiment pipeline (first download may take a while)"
HF_HOME="$MODELS_DIR" "$VENV_DIR/bin/python" - <<'PY'
from transformers import pipeline
p = pipeline("sentiment-analysis", model="distilbert-base-uncased-finetuned-sst-2-english", device=-1)
print("   ok:", p("DecentraAI is working great!")[0])
PY

echo
echo "HF skills ready. Enable them in $DATA_DIR/node.yaml:"
echo
echo "  skills:"
echo "    enabled: true"
echo "    list: [\"sentiment\", \"ner\", \"summarize\", \"translate_ro_en\", \"translate_en_ro\"]"
echo
echo "Then restart the node:  systemctl --user restart decentraai-node"