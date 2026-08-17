#!/usr/bin/env bash
# DecentraAI — two-node LAN validation.
#
# Verifies that a remote peer (the Desktop) is visible, trusted and accepts
# remote inference, then routes a real inference request to it so the request
# is served by the remote node's local model — proving end-to-end remote
# execution through the fabric (M18+/Collective Intelligence).
#
# Usage (on the coordinating laptop, after the Desktop is upgraded):
#   bash scripts/validate-lan.sh
#
# Exits non-zero if any check fails, so it can gate a live validation run.

set -euo pipefail

DATA_DIR="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}"
API="http://127.0.0.1:8080"
TOKEN_FILE="$DATA_DIR/runtime/api.token"

if [ ! -f "$TOKEN_FILE" ]; then
  echo "error: no API token at $TOKEN_FILE (is the node running?)" >&2
  exit 1
fi
TOKEN="$(cat "$TOKEN_FILE")"
AUTH=(-H "Authorization: Bearer $TOKEN")

echo "==> Checking API is up"
if ! curl -sf -m 5 "${AUTH[@]}" "$API/status" >/dev/null; then
  echo "error: node API not reachable at $API" >&2
  exit 1
fi

echo "==> Checking for a remote worker (Desktop)"
WORKERS="$(curl -sf -m 10 "${AUTH[@]}" "$API/v1/compute")"

# A remote worker = one whose peer_id != the local peer.
LOCAL_PEER="$(curl -sf -m 10 "${AUTH[@]}" "$API/v1/compute" | python3 -c "import sys,json; print(json.load(sys.stdin).get('local_peer',''))")"

REMOTE_COUNT="$(echo "$WORKERS" | python3 -c "
import sys,json
d=json.load(sys.stdin)
local=d.get('local_peer','')
rs=[w for w in d.get('workers',[]) if w.get('peer_id')!=local]
print(len(rs))
")"

if [ "$REMOTE_COUNT" -eq 0 ]; then
  echo "error: no remote workers visible. Is the Desktop upgraded to current HEAD and running?" >&2
  exit 1
fi

echo "==> Remote worker(s):"
echo "$WORKERS" | python3 -c "
import sys,json
d=json.load(sys.stdin); local=d.get('local_peer','')
for w in d.get('workers',[]):
    if w.get('peer_id')!=local:
        print('  -', w.get('node_name'), '| peer', (w.get('peer_id') or '')[:16],
              '| trusted:', w.get('trusted'), '| remote_ok:', w.get('accepts_remote_inference'))
        print('    models:', [m.get('file_name') for m in (w.get('served_models') or [])])
"

# Pick a remote model that this node does NOT serve, so routing is forced
# remote (a model present only on the Desktop proves remote execution).
REMOTE_MODEL="$(echo "$WORKERS" | python3 -c "
import sys,json
d=json.load(sys.stdin); local=d.get('local_peer','')
local_models=set()
for w in d.get('workers',[]):
    if w.get('peer_id')==local:
        for m in (w.get('served_models') or []): local_models.add(m.get('file_name'))
for w in d.get('workers',[]):
    if w.get('peer_id')==local: continue
    if not w.get('trusted') or not w.get('accepts_remote_inference'): continue
    for m in (w.get('served_models') or []):
        if m.get('file_name') not in local_models:
            print(m.get('file_name')); raise SystemExit(0)
raise SystemExit('no remote-only model on a trusted, remote-ok worker')
")"

echo "==> Routing a real request to remote model: $REMOTE_MODEL"
RESP="$(curl -sf -m 180 "${AUTH[@]}" -H 'Content-Type: application/json' \
  -d "{\"model\":\"$REMOTE_MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"Say OK in one word.\"}],\"max_tokens\":16}" \
  "$API/v1/chat/completions")"

REPLY="$(echo "$RESP" | python3 -c "import sys,json; print((json.load(sys.stdin).get('choices') or [{}])[0].get('message',{}).get('content',''))")"

echo "==> Remote reply: '$REPLY'"
if [ -z "$REPLY" ]; then
  echo "error: empty remote reply" >&2
  exit 1
fi
echo "==> OK: two-node remote inference verified end-to-end."
