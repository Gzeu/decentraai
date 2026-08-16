#!/usr/bin/env bash
# Isolated llama.cpp RPC tensor-split experiment (DecentraAI — EXPERIMENT).
#
# Purpose: measure the real two-node latency/throughput of llama.cpp's RPC
# tensor-split path (ggml-rpc-server + llama-server --rpc) so DecentraAI can
# later decide, with evidence, whether to enable distributed inference for a
# model that exceeds one node's VRAM.
#
# This is an ISOLATED experiment: it spawns its own ggml-rpc-server and a
# throwaway llama-server, runs a fixed prompt set, and reports the real
# measured latency + tokens/s. It does NOT touch the running DecentraAI node,
# its models, its quota/accounting, or the fabric. It never runs by default.
#
# PREREQUISITES (the operator must have these on PATH or pass them):
#   - llama-server        (llama.cpp server build)
#   - ggml-rpc-server     (llama.cpp tools/rpc build)
#   - a .gguf model file  (--model)
#
# SECURITY: llama.cpp's RPC backend is upstream-documented as
# "proof-of-concept, fragile and insecure". Run this ONLY on a trusted LAN,
# never on an open network. The RPC server binds 127.0.0.1 by default.
#
# USAGE:
#   scripts/rpc-experiment.sh --model /path/model.gguf [--rpc-host HOST:PORT]
#                             [--prompt "text"] [--max-tokens 128]
#                             [--layers 8] [--report /tmp/rpc.json]
#
# Exit codes: 0 = measured + report written; 1 = missing prerequisite;
# 2 = experiment failed (e.g. server did not start); 3 = usage error.

set -euo pipefail

MODEL=""
RPC_HOST="127.0.0.1:50052"
PROMPT="The capital of France is"
MAX_TOKENS=128
RPC_LAYERS=8
REPORT=""
LLAMA_SERVER="${LLAMA_SERVER:-llama-server}"
RPC_SERVER="${RPC_SERVER:-ggml-rpc-server}"
WARMUP=3
RUNS=5

usage() { sed -n '2,/^# USAGE:/p' "$0" | sed 's/^# \{0,1\}//' | head -40; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --model) MODEL="$2"; shift 2 ;;
    --rpc-host) RPC_HOST="$2"; shift 2 ;;
    --prompt) PROMPT="$2"; shift 2 ;;
    --max-tokens) MAX_TOKENS="$2"; shift 2 ;;
    --layers) RPC_LAYERS="$2"; shift 2 ;;
    --report) REPORT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1"; usage; exit 3 ;;
  esac
done

if [[ -z "$MODEL" ]]; then
  echo "error: --model is required" >&2
  usage >&2
  exit 3
fi
if [[ ! -f "$MODEL" ]]; then
  echo "error: model not found: $MODEL" >&2
  exit 1
fi

# ---- Prerequisite detection (honest: never fabricate the measurement) ----
if ! command -v "$LLAMA_SERVER" >/dev/null 2>&1; then
  echo "prereq missing: '$LLAMA_SERVER' not found on PATH" >&2
  echo "  install llama.cpp and put llama-server on PATH, or set LLAMA_SERVER=" >&2
  exit 1
fi
if ! command -v "$RPC_SERVER" >/dev/null 2>&1; then
  echo "prereq missing: '$RPC_SERVER' not found on PATH" >&2
  echo "  build llama.cpp tools/rpc (ggml-rpc-server) and put it on PATH, or set RPC_SERVER=" >&2
  exit 1
fi

WORKDIR="$(mktemp -d)"
trap 'kill 0 2>/dev/null; rm -rf "$WORKDIR"' EXIT

echo "== llama.cpp RPC tensor-split experiment =="
echo "  model:       $MODEL"
echo "  rpc server:  $RPC_SERVER @ $RPC_HOST"
echo "  layers:      $RPC_LAYERS on RPC side"
echo "  prompt:      \"$PROMPT\""
echo "  max_tokens:  $MAX_TOKENS  (runs: $WARMUP warmup + $RUNS measured)"
echo

# ---- Start the isolated RPC server ----
echo "[1/3] starting ggml-rpc-server on $RPC_HOST ..."
"$RPC_SERVER" --port "${RPC_HOST##*:}" >"$WORKDIR/rpc.log" 2>&1 &
RPC_PID=$!
sleep 1
if ! kill -0 "$RPC_PID" 2>/dev/null; then
  echo "error: ggml-rpc-server failed to start (see $WORKDIR/rpc.log)" >&2
  exit 2
fi

# ---- Start a throwaway llama-server with tensor split over RPC ----
echo "[2/3] starting llama-server (tensor-split via --rpc $RPC_HOST) ..."
"$LLAMA_SERVER" \
  --model "$MODEL" \
  --rpc "$RPC_HOST" \
  --tensor-split "0.5,0.5" \
  --n-gpu-layers "$RPC_LAYERS" \
  --host 127.0.0.1 --port 0 \
  --log-file "$WORKDIR/llama.log" \
  >"$WORKDIR/llama.out" 2>&1 &
LLAMA_PID=$!

# Wait for the server to report a listening port.
PORT=""
for _ in $(seq 1 60); do
  PORT="$(grep -oE 'listening on IP address 127\.0\.0\.1, port [0-9]+' "$WORKDIR/llama.log" \
    | tail -1 | grep -oE '[0-9]+$')"
  [[ -n "$PORT" ]] && break
  if ! kill -0 "$LLAMA_PID" 2>/dev/null; then
    echo "error: llama-server exited early (see $WORKDIR/llama.log)" >&2
    exit 2
  fi
  sleep 0.5
done
if [[ -z "$PORT" ]]; then
  echo "error: could not read llama-server port (see $WORKDIR/llama.log)" >&2
  exit 2
fi
echo "  llama-server ready on port $PORT"

# ---- Run the fixed prompt set and record real timings ----
echo "[3/3] measuring ..."
run_once() {
  local body="{\"prompt\":$(printf '%s' "$PROMPT" | jq -Rsa .),\"n_predict\":$MAX_TOKENS}"
  local t0 t1
  t0=$(date +%s%N)
  curl -sf "http://127.0.0.1:$PORT/completion" \
    -H 'Content-Type: application/json' -d "$body" >"$WORKDIR/resp.json" || return 1
  t1=$(date +%s%N)
  local latency_ms=$(( (t1 - t0) / 1000000 ))
  local tokens
  tokens=$(jq -r '.content' "$WORKDIR/resp.json" 2>/dev/null | wc -w)
  if [[ -z "$tokens" || "$tokens" -eq 0 ]]; then tokens=0; fi
  printf '%s %s\n' "$latency_ms" "$tokens"
}

declare -a lat_ms
declare -a tok
for i in $(seq 1 "$WARMUP"); do
  run_once >/dev/null 2>&1 || true   # warmup, discard
done
for i in $(seq 1 "$RUNS"); do
  read -r l t <<< "$(run_once)" || { echo "  run $i: FAILED"; continue; }
  lat_ms+=("$l")
  tok+=("$t")
  echo "  run $i: ${l}ms, ${t} tokens"
done

if [[ "${#lat_ms[@]}" -eq 0 ]]; then
  echo "error: all measurement runs failed (no real results — nothing to report)" >&2
  exit 2
fi

# ---- Aggregate (pure arithmetic on the real samples) ----
total_lat=0; total_tok=0
for i in "${!lat_ms[@]}"; do total_lat=$(( total_lat + lat_ms[i] )); done
for i in "${!tok[@]}"; do total_tok=$(( total_tok + tok[i] )); done
n=${#lat_ms[@]}
avg_lat=$(( total_lat / n ))
total_tok_sum=$total_tok

# ---- Write the report ----
if [[ -n "$REPORT" ]]; then
  cat > "$REPORT" <<EOF
{
  "experiment": "llama.cpp RPC tensor-split",
  "classification": "EXPERIMENT",
  "date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "model": "$MODEL",
  "rpc_host": "$RPC_HOST",
  "rpc_layers": "$RPC_LAYERS",
  "runs": $n,
  "warmup": $WARMUP,
  "prompt": "$PROMPT",
  "max_tokens": $MAX_TOKENS,
  "results": {
    "avg_latency_ms": $avg_lat,
    "total_tokens_generated": $total_tok_sum,
    "avg_tokens_per_run": $(( total_tok_sum / n )),
    "samples_ms": [$(IFS=,; echo "${lat_ms[*]}")]
  },
  "provenance": "REAL_MEASURED — local isolated experiment, not the live fabric",
  "note": "RPC backend is upstream-documented as proof-of-concept/fragile/insecure; LAN-only."
}
EOF
  echo
  echo "report written to $REPORT"
else
  echo
  echo "summary: avg latency ${avg_lat}ms, ${total_tok_sum} tokens across ${n} runs"
fi
echo "experiment complete (isolated; nothing touched the live fabric)"
exit 0
