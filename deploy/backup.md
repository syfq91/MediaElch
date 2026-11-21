# Backup and restore

## What is stateful

- **`mediaelch.sqlite`** — the new cache/index database. Rebuildable by a
  re-scan, but back it up anyway so a restore is instant.
- **NFO files and artwork next to your media** — the real source of truth.
  The new server reads and writes them; back them up with the media itself.
- **`config.toml`** — contains API keys and the password hash. Store it
  securely and include it in backups.

The legacy `MediaElch.sqlite` and `MediaElch.ini` are never read by the new
server and do not need to be migrated.

## Backup

```sh
MEDIAELCH_DB=/var/lib/mediaelch/mediaelch.sqlite \
MEDIAELCH_CONFIG=/etc/mediaelch/config.toml \
MEDIAELCH_BACKUP_DIR=/var/backups/mediaelch \
./deploy/backup.sh
```

The script writes a consistent SQLite copy using `.backup` (safe while the
server is running), archives the config, and removes SQLite backups older
than 30 days. Requires the `sqlite3` CLI.

## Restore

```sh
./deploy/restore.sh /var/backups/mediaelch/mediaelch-20261121T020000Z.sqlite
```

The script stops the service, keeps a `*.before-restore` copy of the current
database, installs the backup and restarts the service. Restore the matching
`config-*.tar.gz` separately if the configuration also changed.

## Drill

Before cutover, rehearse on a spare machine or a copy of the data directory:

1. Take a backup.
2. Scan a library and add a few pieces of metadata.
3. Restore the backup.
4. Confirm the pre-backup item count is back and `/healthz` reports `ok`.

A backup that has never been restored is not a backup.
