# Cutover checklist

Goal: retire the Qt desktop application without losing library metadata.

## 1. Prepare

- [ ] Deploy the Rust server on the headless host; confirm
      `cargo build --release` or the packaged binary starts with no Qt/X11 libs.
- [ ] Configure `media_directories`, API keys and `[auth] enabled = true`.
- [ ] Confirm TLS is terminated (built-in or reverse proxy) and that
      `/healthz` works through the proxy.
- [ ] Take a backup (see [backup.md](backup.md)) and rehearse a restore.

## 2. Parallel run

- [ ] Run a full scan on a **copy** of a representative library.
- [ ] Compare item counts and a sample of NFO-derived fields (title, year,
      genres, IDs) against the Qt application.
- [ ] Verify NFO writing on a copy: save from the web UI, re-open in the Qt
      app, and confirm Kodi still reads the file.
- [ ] Verify a scrape job with real API keys, including artwork download.
- [ ] Verify rename dry-run then apply on a small directory.

## 3. Switch

- [ ] Stop scheduling new work in the Qt app; leave it installed but unused.
- [ ] Point users at the server URL and hand them the login credentials.
- [ ] Keep the NFO files as the source of truth; avoid editing them with both
      applications at the same time.

## 4. Watch

- [ ] Check server logs for scrape failures and 429 rate limits for one week.
- [ ] Confirm scheduled/manual scans keep item counts stable.

## 5. Rollback

Triggers: widespread metadata corruption, NFO writes that Kodi cannot read, or
scrapers failing against current provider APIs.

1. Stop the Rust server.
2. Restore the SQLite backup if the index is suspect:
   `./deploy/restore.sh <backup.sqlite>`.
3. Restore NFO files from the media backup if they were rewritten incorrectly.
4. Restart the Qt application against the same media directories; it reads the
   NFO files directly, so no data migration is needed.

Known differences to communicate to users: no Kodi JSON-RPC library sync, no
music library, only TMDB/TVMaze/fanart.tv providers, and NFO byte layout is
not identical to the Qt writer (field values are preserved).
