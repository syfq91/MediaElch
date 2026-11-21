#!/usr/bin/env sh
# Consistent backup of the Rust server's SQLite database and config file.
# Requires the sqlite3 CLI. Media/NFO files are backed up separately.
set -eu

DB=${MEDIAELCH_DB:-./data/mediaelch.sqlite}
CONFIG=${MEDIAELCH_CONFIG:-./config.toml}
BACKUP_DIR=${MEDIAELCH_BACKUP_DIR:-./backups}
KEEP_DAYS=${MEDIAELCH_BACKUP_KEEP_DAYS:-30}

if [ ! -f "$DB" ]; then
    echo "database not found: $DB" >&2
    exit 1
fi

mkdir -p "$BACKUP_DIR"
STAMP=$(date -u +%Y%m%dT%H%M%SZ)

DEST="$BACKUP_DIR/mediaelch-$STAMP.sqlite"
if command -v sqlite3 >/dev/null 2>&1; then
    sqlite3 "$DB" ".backup '$DEST'"
else
    python3 - "$DB" "$DEST" <<'PY'
import sqlite3
import sys

source = sqlite3.connect(sys.argv[1])
target = sqlite3.connect(sys.argv[2])
source.backup(target)
target.close()
source.close()
PY
fi

if [ -f "$CONFIG" ]; then
    tar -czf "$BACKUP_DIR/config-$STAMP.tar.gz" -C "$(dirname "$CONFIG")" "$(basename "$CONFIG")"
fi

find "$BACKUP_DIR" -name 'mediaelch-*.sqlite' -mtime "+$KEEP_DAYS" -delete
find "$BACKUP_DIR" -name 'config-*.tar.gz' -mtime "+$KEEP_DAYS" -delete

echo "backup complete: $BACKUP_DIR/mediaelch-$STAMP.sqlite"
