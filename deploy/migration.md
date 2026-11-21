# Migration notes for existing MediaElch users

The headless server is a clean rewrite. It does **not** import the Qt
application's internal state, but it does read the NFO files next to your media.

## What carries over

| Old (Qt) | New (Rust server) |
|---|---|
| NFO sidecars (`movie.nfo`, `tvshow.nfo`, episode NFOs, …) | Read on scan. Values win over filename guesses. Rewritten only when you press **Save NFO**. |
| Artwork files (`poster.jpg`, `fanart.jpg`, `.actors/`, …) | Recognised and served; missing artwork can be downloaded from configured providers. |
| Media directories | Re-add them as `[library] media_directories` in `config.toml`. |
| API keys entered in the old settings UI | Re-enter them in `[scrapers]` (do not reuse keys committed in the old source). |
| Watched state / Kodi sync settings | Deferred; not available in the first headless release. |

## What does not carry over

- `MediaElch.sqlite` — the old cache/index database. A fresh database with a
  new schema is created at `data_dir/mediaelch.sqlite`. Re-scan to populate it.
- `MediaElch.ini` / Qt `QSettings` — replaced by `config.toml`; there is no
  automatic import.
- `advancedsettings.xml` — genre/certification mappings and exclude patterns
  are not imported; the web UI uses built-in defaults for now.
- Export themes from the old `export_themes` directory — the built-in HTML
  template is used; custom theme upload is not yet implemented.
- Qt translations — the web UI ships English first.

## Behavioural differences to expect

- Scrape metadata only from TMDB, TVMaze and fanart.tv. IMDb, TheTVDB, OMDb,
  Discogs, MusicBrainz and adult-site providers are not part of v1.
- Music (artists/albums) and Kodi JSON-RPC library sync are not in v1.
- NFO XML layout is not byte-identical to the Qt writer; the important field
  values (title, year, plot, ratings, IDs, genres, credits) are preserved.
- Renaming is dry-run by default and unknown placeholders are an error instead
  of being silently removed.

## First-run sequence

1. Install the binary and write `config.toml` from `config.example.toml`.
2. Add your media directories and API keys; enable `[auth]`.
3. Start the server, sign in and open **Tools → Scan library**.
4. Spot-check a few items against their NFO files before enabling automatic
   NFO writes (`library.write_nfo_automatically`).
