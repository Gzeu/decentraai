#!/usr/bin/env bash
# DecentraAI node backup — db + experiments + node.yaml into a timestamped
# archive with a restore manifest (git rev, world tick, file counts).
#
# Secrets are EXCLUDED by default: runtime/api.token, identity/*,
# db/tokens.json, db/consumer_keys.json (raw dca_ keys) and any *seed*.hex /
# *signer* files NEVER enter the archive. Back those up manually (0600,
# offline) — a backup containing live credentials is a secret-sprawl
# incident waiting to happen.
#
# Usage: scripts/backup-node.sh [dest-dir]   (default: ~/.decentraai-backups)
set -euo pipefail

DATA_DIR="${DECENTRAAI_DATA_DIR:-$HOME/.decentraai}"
DEST_DIR="${1:-$HOME/.decentraai-backups}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$DEST_DIR/decentraai-backup-$STAMP.tar.gz"

mkdir -p "$DEST_DIR"
if [ ! -d "$DATA_DIR/db" ]; then
  echo "error: $DATA_DIR/db not found (is this the node data dir?)" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cp -r "$DATA_DIR/db" "$WORK/db"
mkdir -p "$WORK/experiments"
if [ -d "$DATA_DIR/experiments" ]; then
  cp -r "$DATA_DIR/experiments/." "$WORK/experiments/"
fi
cp "$DATA_DIR/node.yaml" "$WORK/node.yaml" 2>/dev/null || echo "warn: node.yaml absent" >&2

# Strip anything secret-shaped that may have wandered in (belt and braces:
# these paths are excluded by construction above, this is the second net).
find "$WORK" \( -name "api.token" -o -name "*seed*.hex" -o -name "*signer*" \
  -o -name "tokens.json" -o -name "consumer_keys.json" \
  -o -path "*identity*" \) -delete 2>/dev/null || true

# Restore manifest: what, from where, how much.
{
  echo "backup: $STAMP"
  echo "data_dir: $DATA_DIR"
  echo "git_rev: $(git -C "$(dirname "$0")/.." rev-parse HEAD 2>/dev/null || echo unknown)"
  echo "db_files: $(find "$WORK/db" -type f | wc -l)"
  echo "experiment_files: $(find "$WORK/experiments" -type f | wc -l)"
  if command -v python3 >/dev/null 2>&1 && [ -f "$WORK/db/world.json" ]; then
    python3 -c "import json;d=json.load(open('$WORK/db/world.json'));print('world_tick:',d.get('tick'),'mission:',d.get('mission_task_id'))" 2>/dev/null || true
  fi
} > "$WORK/MANIFEST.txt"

tar -czf "$OUT" -C "$WORK" db experiments node.yaml MANIFEST.txt

# Final audit: the archive must not contain secret-shaped names.
if tar -tzf "$OUT" | grep -iE "api\.token|seed.*\.hex|signer|tokens\.json|identity/" >/dev/null; then
  echo "error: archive contains secret-shaped paths — refusing to keep it" >&2
  rm -f "$OUT"
  exit 1
fi
echo "backup: $OUT ($(du -h "$OUT" | cut -f1))"
echo "manifest:"; cat "$WORK/MANIFEST.txt"
echo "note: identity/, runtime/api.token, signer seeds are NOT in this archive (back up 0600 offline, separately)"
