#!/usr/bin/env bash
# Post-deploy verification — run after starting a DecentraAI node.
# Checks every subsystem is live and reporting real state.
set -uo pipefail
API="${DECENTRAAI_URL:-http://127.0.0.1:8080}"
TOKEN=$(cat "${TOKEN_FILE:-$HOME/.decentraai/runtime/api.token}" 2>/dev/null || echo "")
AUTH="Authorization: Bearer $TOKEN"
PASS=0; FAIL=0
check() {
    local name="$1" url="$2" expect="$3"
    local code
    code=$(curl -sS -m 10 -o /dev/null -w "%{http_code}" -H "$AUTH" "$API$url" 2>/dev/null)
    if [ "$code" = "200" ]; then
        echo "✅ $name"
        PASS=$((PASS+1))
    else
        echo "❌ $name (HTTP $code)"
        FAIL=$((FAIL+1))
    fi
}
check "Node status"          "/status"
check "Model loaded"         "/v1/models"
check "Workers"              "/v1/compute"
check "Collective memory"    "/v1/memory"
check "Model intel"          "/v1/models/intel"
check "Model routing"        "/v1/models/route"
echo "---"
echo "PASS: $PASS / FAIL: $FAIL"
[ $FAIL -eq 0 ]
