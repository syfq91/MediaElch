#!/usr/bin/env sh
# Restore a MediaElch SQLite backup. Stops the systemd service if present.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <backup.sqlite>" >&2
    exit 1
fi

BACKUP=$1
DB=${MEDIAELCH_DB:-/var/lib/mediaelch/mediaelch.sqlite}
SERVICE=${MEDIAELCH_SERVICE:-mediaelch}

if [ ! -f "$BACKUP" ]; then
    echo "backup not found: $BACKUP" >&2
    exit 1
fi

if command -v systemctl >/dev/null 2>&1; then
    systemctl stop "$SERVICE" || true
fi

mkdir -p "$(dirname "$DB")"
if [ -f "$DB" ]; then
    cp -p "$DB" "$DB.before-restore"
fi
cp "$BACKUP" "$DB"

if command -v systemctl >/dev/null 2>&1; then
    systemctl start "$SERVICE" || true
fi

echo "restored $BACKUP to $DB"
