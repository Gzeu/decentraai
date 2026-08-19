#!/usr/bin/env bash
# DecentraAI — upgrade a REMOTE node over SSH, from the coordinating machine.
#
# MENȚIUNE IMPORTANTĂ (citește înainte):
#   Acest script rulează comenzi pe o altă mașină prin SSH. Este o operație de
#   administrare manuală făcută de operator (George) pe propriile mașini —
#   NU este "remote shell" din aplicație: DecentraAI nu rulează niciodată
#   shell remote sau push de binare prin mesh. Aici SSH-ul e doar canalul
#   de ops dintre mașinile tale.
#
#   Direcții suportate:
#     - de pe Laptop → Desktop:  bash scripts/upgrade-remote-node.sh dca@192.168.1.138
#     - de pe Desktop → Laptop:  bash scripts/upgrade-remote-node.sh i5@192.168.1.132
#
#   Cerințe pe mașina țintă:
#     1. sshd activ:   sudo systemctl enable --now ssh
#     2. user cu cheia publică a mașinii locale în ~/.ssh/authorized_keys
#        (scriptul folosește BatchMode, deci cheie recomandată)
#     3. repo-ul decentraai clonat în ~/decentraai
#     4. node-ul instalat via scripts/install-app.sh (systemd user service)
#
#   Ce face, pas cu pas:
#     1. verifică SSH către țintă (fail fast dacă nu e deschis)
#     2. pe țintă: git fetch + checkout main + pull --rebase (aduce build-ul
#        nou, inclusiv accepts_remote_inference)
#     3. pe țintă: bash scripts/upgrade-node.sh (build release, swap binar,
#        enable remote inference, restart serviciu)
#     4. de pe mașina locală: verifică /v1/compute că ținta apare ca worker
#        remote trusted cu remote_ok (și rulează validate-lan.sh dacă
#        VALIDATE_LAN=1 și mașina locală e coordinator)
#
# Usage (din repo-ul mașinii locale):
#   bash scripts/upgrade-remote-node.sh [user@host]
#   REMOTE_PORT=2222 bash scripts/upgrade-remote-node.sh i5@10.0.0.5
#   VALIDATE_LAN=1 bash scripts/upgrade-remote-node.sh dca@192.168.1.138
#
# Exit codes: 0 = upgrade + verificare OK; 1 = SSH/remote fail; 2 = verificare fail.

set -euo pipefail

REMOTE_HOST="${1:-dca@192.168.1.138}"
REMOTE_PORT="${REMOTE_PORT:-22}"
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=8 -o StrictHostKeyChecking=accept-new -p "$REMOTE_PORT")

echo "==> [1/4] Verific SSH către $REMOTE_HOST (port $REMOTE_PORT)"
if ! ssh "${SSH_OPTS[@]}" "$REMOTE_HOST" 'echo ok' >/dev/null 2>&1; then
  echo "error: SSH nu răspunde pe $REMOTE_HOST:$REMOTE_PORT" >&2
  echo "  Pe țintă rulează: sudo systemctl enable --now ssh" >&2
  echo "  Și asigură-te că cheia publică a mașinii locale e în ~/.ssh/authorized_keys" >&2
  exit 1
fi
echo "  SSH ok"

echo "==> [2/4] git fetch + checkout main + pull --rebase pe țintă"
ssh "${SSH_OPTS[@]}" "$REMOTE_HOST" 'cd ~/decentraai && git fetch origin && git checkout main && git pull --rebase origin main && git log --oneline -1'

echo "==> [3/4] upgrade-node.sh pe țintă (build + swap + restart)"
ssh "${SSH_OPTS[@]}" "$REMOTE_HOST" 'cd ~/decentraai && bash scripts/upgrade-node.sh'

echo "==> [4/4] Verific de pe mașina locală că ținta apare ca worker remote"
sleep 10
TOKEN_FILE="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}/runtime/api.token"
API_PORT="$(grep -E '^[[:space:]]*api_port:' "${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}/node.yaml" 2>/dev/null | awk '{print $2}' || echo 8080)"
if [ -f "$TOKEN_FILE" ]; then
  TOKEN="$(cat "$TOKEN_FILE")"
  REMOTE_WORKERS="$(curl -s -m 8 -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/v1/compute")"
  LOCAL_PEER="$(echo "$REMOTE_WORKERS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('local_peer',''))" 2>/dev/null || true)"
  echo "$REMOTE_WORKERS" | python3 -c "
import sys,json
d=json.load(sys.stdin); local=d.get('local_peer','')
found=False
for w in d.get('workers',[]):
    if w.get('peer_id')!=local:
        found=True
        print(f\"  remote: {w.get('node_name','?')} | peer {(w.get('peer_id') or '')[:16]} | trusted: {w.get('trusted')} | remote_ok: {w.get('accepts_remote_inference')}\")
        print(f\"    models: {[m.get('file_name') for m in (w.get('served_models') or [])]}\")
if not found:
    print('  ATENȚIE: niciun worker remote vizibil! Verifică firewall/trust pe țintă.')
" 2>/dev/null || echo "  (nu am putut citi /v1/compute; verifică dashboard-ul manual)"
else
  echo "  (fără token API local; verifică dashboard-ul manual)"
fi

if [ "${VALIDATE_LAN:-0}" = "1" ]; then
  echo "==> [extra] Rulez validate-lan.sh (routing remote end-to-end)"
  bash scripts/validate-lan.sh
fi

echo
echo "==> Upgrade remote complet. Dacă ținta apare ca worker remote trusted + remote_ok, e integrat."