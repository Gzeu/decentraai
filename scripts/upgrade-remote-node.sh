#!/usr/bin/env bash
# DecentraAI — upgrade a REMOTE node (the laptop i5) from the Desktop.
#
# MENȚIUNE IMPORTANTĂ (citește înainte):
#   Acest script rulează comenzi pe laptop prin SSH. Este o operație de
#   administrare manuală făcută de operator (George) pe propria mașină —
#   NU este "remote shell" din aplicație: DecentraAI nu rulează niciodată
#   shell remote sau push de binare prin mesh. Aici SSH-ul e doar canalul
#   de ops dintre cele două mașini ale tale.
#
#   Cerințe pe laptopul i5 (192.168.1.132):
#     1. sshd activ:   sudo systemctl enable --now ssh
#     2. user `i5` cu cheie publică a Desktop-ului în ~/.ssh/authorized_keys
#        (sau parolă — scriptul folosește BatchMode, deci cheie recomandată)
#     3. repo-ul decentraai clonat în ~/decentraai
#     4. node-ul instalat via scripts/install-app.sh (systemd user service)
#
#   Ce face, pas cu pas:
#     1. verifică SSH către laptop (fail fast dacă nu e deschis)
#     2. pe laptop: git pull --rebase (aduce build-ul nou, inclusiv
#        protocol_version + accepts_remote_inference)
#     3. pe laptop: bash scripts/upgrade-node.sh (build release, swap binar,
#        restart serviciu, enable remote inference)
#     4. de pe Desktop: verifică /v1/fabric că laptopul apare ca nod nou
#
# Usage (din repo-ul Desktop):
#   bash scripts/upgrade-remote-node.sh                 # default: i5@192.168.1.132:22
#   bash scripts/upgrade-remote-node.sh i5@10.0.0.5     # alt host
#   REMOTE_PORT=2222 bash scripts/upgrade-remote-node.sh
#
# Exit codes: 0 = upgrade + verificare OK; 1 = SSH/remote fail; 2 = verificare fail.

set -euo pipefail

REMOTE_HOST="${1:-i5@192.168.1.132}"
REMOTE_PORT="${REMOTE_PORT:-22}"
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=8 -o StrictHostKeyChecking=accept-new -p "$REMOTE_PORT")

echo "==> [1/4] Verific SSH către $REMOTE_HOST (port $REMOTE_PORT)"
if ! ssh "${SSH_OPTS[@]}" "$REMOTE_HOST" 'echo ok' >/dev/null 2>&1; then
  echo "error: SSH nu răspunde pe $REMOTE_HOST:$REMOTE_PORT" >&2
  echo "  Pe laptop rulează: sudo systemctl enable --now ssh" >&2
  echo "  Și asigură-te că Desktop-ul are cheia în ~/.ssh/authorized_keys" >&2
  exit 1
fi
echo "  SSH ok"

echo "==> [2/4] git pull --rebase pe laptop"
ssh "${SSH_OPTS[@]}" "$REMOTE_HOST" 'cd ~/decentraai && git pull --rebase origin main && git log --oneline -1'

echo "==> [3/4] upgrade-node.sh pe laptop (build + swap + restart)"
ssh "${SSH_OPTS[@]}" "$REMOTE_HOST" 'cd ~/decentraai && bash scripts/upgrade-node.sh'

echo "==> [4/4] Verific de pe Desktop că laptopul apare în fabric"
sleep 10
TOKEN_FILE="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}/runtime/api.token"
API_PORT="$(grep -E '^[[:space:]]*api_port:' "${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}/node.yaml" 2>/dev/null | awk '{print $2}' || echo 8080)"
if [ -f "$TOKEN_FILE" ]; then
  TOKEN="$(cat "$TOKEN_FILE")"
  curl -s -m 8 -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/v1/fabric" \
    | python3 -c "
import json,sys
d=json.load(sys.stdin)
nodes=d.get('nodes',[])
print('  noduri în fabric:', len(nodes))
for n in nodes:
    print(f\"    {n.get('node_id','?')[:20]:20} {n.get('node_name','?'):20} lifecycle={n.get('lifecycle')} version={n.get('node_version')} trusted={n.get('trusted')}\")
" 2>/dev/null || echo "  (nu am putut citi /v1/fabric; verifică dashboard-ul manual)"
else
  echo "  (fără token API; verifică dashboard-ul manual)"
fi

echo
echo "==> Upgrade remote complet. Dacă laptopul apare ca nod + worker, e integrat."