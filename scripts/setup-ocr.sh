#!/usr/bin/env bash
# DecentraAI — local OCR setup (RapidOCR, PP-OCRv4 on onnxruntime).
#
# Installs the Python venv the node needs for `/v1/ocr` (Tool Runtime OCR).
# Everything lives under <data_dir>/tools/ocr/ so the node subprocess can find
# it without sudo:
#   <data_dir>/tools/ocr/venv/                  uv-managed virtualenv
#
# RapidOCR's wheel bundles the PP-OCRv4 ONNX models — no separate download,
# no GPU required (runs on CPU via onnxruntime).
#
# Idempotent: rerunning only fills gaps (existing venv/deps are kept).
#
# Run from the repository root:
#   bash scripts/setup-ocr.sh [--data-dir PATH]
set -euo pipefail

cd "$(dirname "$0")/.."

DATA_DIR="${DECENTRAI_DATA_DIR:-$HOME/.decentraai}"
while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) DATA_DIR="${2:?usage: --data-dir PATH}"; shift 2 ;;
    --help|-h) echo "usage: bash scripts/setup-ocr.sh [--data-dir PATH]"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

OCR_DIR="$DATA_DIR/tools/ocr"
VENV_DIR="$OCR_DIR/venv"

if [ ! -d "$DATA_DIR" ]; then
  echo "error: data dir $DATA_DIR does not exist (run \`decentraai setup\` first)" >&2
  exit 1
fi

UV="$(command -v uv || echo "$HOME/.local/bin/uv")"
if [ ! -x "$UV" ]; then
  echo "error: uv not found (install with: curl -LsSf https://astral.sh/uv/install.sh | sh)" >&2
  exit 1
fi

mkdir -p "$OCR_DIR"

if [ ! -x "$VENV_DIR/bin/python" ]; then
  echo "==> creating virtualenv"
  "$UV" venv "$VENV_DIR" >/dev/null
fi

echo "==> installing rapidocr-onnxruntime (bundles PP-OCRv4 ONNX models)"
"$UV" pip install --python "$VENV_DIR/bin/python" "rapidocr-onnxruntime>=1.4"

echo "==> smoke test"
"$VENV_DIR/bin/python" - <<'PY'
from rapidocr_onnxruntime import RapidOCR
ocr = RapidOCR()
print("   ok: RapidOCR engine loaded (PP-OCRv4)")
PY

echo
echo "OCR ready. Enable it in $DATA_DIR/node.yaml:"
echo
echo "  ocr:"
echo "    enabled: true"
echo "    lang: \"en\""
echo
echo "Then restart the node:  systemctl --user restart decentraai-node"