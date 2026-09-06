#!/usr/bin/env bash
# DecentraAI node restore — unpack a backup-node.sh archive over the data dir.
#
# REFUSES to run while the node service is active (sqlite + live json writes
# + a running process = corruption). Stop the node first:
#   systemctl --user stop decentraai-node
#
# Usage: scripts/restore-node.sh <archive.tar.gz> [--force]
#   --dry-run (default): list what WOULD be restored, change nothing.
#   --force:             actually overwrite db/, experiments/, node.yaml.
set -euo pipefail

DATA_DIR="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}"
ARCHIVE="${1:-}"
MODE="${2:---dry-run}"

if [ -z "$ARCHIVE" ] || [ ! -f "$ARCHIVE" ]; then
  echo "usage: $0 <archive.tar.gz> [--dry-run|--force]" >&2
  exit 1
fi

echo "archive contents:"
tar -tzf "$ARCHIVE"

if systemctl --user is-active --quiet decentraai-node 2>/dev/null; then
  if [ "$DATA_DIR" = "$HOME/.decentraai" ]; then
    echo "error: decentraai-node is ACTIVE — stop it first (refusing to restore over a live node)" >&2
    exit 1
  fi
  echo "warn: node is active but target $DATA_DIR is not the live dir — proceeding (scratch restore)"
fi

if [ "$MODE" != "--force" ]; then
  echo "dry-run: nothing changed. Re-run with --force to overwrite db/, experiments/, node.yaml in $DATA_DIR."
  exit 0
fi

BACKUP_PRE="$(mktemp -d)/pre-restore-$RANDOM"
mkdir -p "$BACKUP_PRE"
for d in db experiments; do
  [ -d "$DATA_DIR/$d" ] && cp -r "$DATA_DIR/$d" "$BACKUP_PRE/$d" || true
done
[ -f "$DATA_DIR/node.yaml" ] && cp "$DATA_DIR/node.yaml" "$BACKUP_PRE/node.yaml" || true
echo "pre-restore copy kept at: $BACKUP_PRE (delete manually when satisfied)"

tar -xzf "$ARCHIVE" -C "$DATA_DIR"
echo "restored into $DATA_DIR. Start the node and verify: /v1/world tick + trigger state."
