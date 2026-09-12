#!/usr/bin/env bash
# Secret scan for the DecentraAI repo (response to the dsk_ incident).
#
# Fails (exit 1) when live-looking credentials are found in tracked or
# staged files. NEVER prints secret values: matches are reported as
# file:line + prefix + length only.
#
# Usage: bash scripts/secret-scan.sh [--staged]
#   --staged : scan staged changes only (pre-commit friendly)
set -euo pipefail

PATTERN='dsk_[A-Za-z0-9]{8,}|dca_[A-Za-z0-9]{8,}|sk[-_]live[-_][A-Za-z0-9]+|xox[bap]-[A-Za-z0-9-]+|ghp_[A-Za-z0-9]+|BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|adAccessCode[^A-Za-z0-9]{0,5}[A-Za-z0-9]{8,}'

# Known-safe fixtures that must never trip the gate.
ALLOW='0000000000|dca_alpha|dca_demo|dca_other|abc123|abcdef|call_1|sk-live-abcdef|dca_validkey123456'

if [ "${1:-}" = "--staged" ]; then
  FILES=$(git diff --cached --name-only --diff-filter=ACM)
  [ -z "$FILES" ] && { echo "secret-scan: nothing staged"; exit 0; }
  SCAN=$(echo "$FILES" | xargs grep -rEn -- "$PATTERN" 2>/dev/null || true)
else
  SCAN=$(grep -rEn --exclude-dir=target --exclude-dir=.git --exclude='*.lock' -- "$PATTERN" . 2>/dev/null || true)
fi

HITS=$(echo "$SCAN" | grep -vE "$ALLOW" | grep -v "^$" || true)
if [ -z "$HITS" ]; then
  echo "secret-scan: clean"
  exit 0
fi

echo "secret-scan: POSSIBLE LIVE CREDENTIALS (values redacted):"
echo "$HITS" | while IFS= read -r line; do
  fileline=$(echo "$line" | cut -d: -f1-2)
  secret=$(echo "$line" | grep -oE "$PATTERN" | head -n 1)
  echo "  $fileline :: ${secret:0:7}… (${#secret} chars)"
done
exit 1
