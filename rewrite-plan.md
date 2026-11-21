# MediaElch → Headless Rust Server: Rewrite Plan

**Status:** Approved for implementation
**Baseline:** MediaElch 2.12.1-dev ("Benzar"), commit `1299ce44`
**Target:** Single Rust binary, no display server, accessed through a web browser.

---

## 1. Executive summary

MediaElch is a Qt 5 / C++ desktop media manager for Kodi. The shipped binary is
hard-bound to Qt Widgets and X11, so it cannot run headless. This plan replaces
it with a Rust HTTP server that keeps the product's essential behavior — scan a
media library, read and write Kodi NFO sidecars, fetch metadata and artwork,
rename files, export lists — and exposes that behavior over REST + server-side
rendered HTML (HTMX).

The rewrite is a **clean break on internal state** (no migration of the old
`MediaElch.sqlite`, settings, or Qt job machinery) but **keeps the on-disk NFO
files as the library's source of truth**. Existing libraries therefore keep
their metadata; only MediaElch's internal cache is discarded.

---

## 2. REA evidence baseline

REA (v6.10.1) was used to inspect the shipped artifact. Evidence is retained on
the current MCP connection for this session.

| Fact | Finding | Evidence |
|---|---|---|
| Target | `MediaElch_2.12.0/usr/bin/MediaElch`, ELF64 x86-64 executable, 6,518,792 bytes, SHA-256 `fad57e86e4400da1aa2c9a7fe0b4a19f73019035c3b6b36f2fc3e2f964007d97` | `ev_8f4794…c4a605ece3e4cb8` |
| Runpath | `$ORIGIN/../lib` (self-contained AppImage layout) | `ev_3d80a4…c38319bf` |
| Direct deps (14) | `libQt5Widgets`, `libQt5Gui`, `libQt5Multimedia{,Widgets}`, `libQt5Network`, `libQt5Xml`, `libQt5Sql`, `libQt5Concurrent`, `libQt5Core`, `libz`, `libstdc++`, `libc`, `libdl` | `ev_3d80a4…c38319bf` |
| Imports | 2,092 undefined `.dynsym` imports; 21 version requirements | `ev_8f4794…c4a605ece3e4cb8` |
| Bundled runtime | `libQt5XcbQpa`, X11 libs, GStreamer, `libmediainfo`, `libsqlite3`, `libtinyxml2`, `libcurl-gnutls`, `ffmpeg` | artifact inventory, `MediaElch_2.12.0/usr/lib`, `inspect_artifact` `ev_0653008e…b5a0e` |
| Release exposure | No embedded debug companions, source maps or extractable package payload in the ELF | `ev_d4b70d5f…c7591` |

**Interpretation:** the application requires an X11 display through
`libQt5XcbQpa` + `libQt5Widgets`/`Gui`. A headless rewrite cannot reuse the
existing UI layer at all; it must be deleted, not adapted. Qt Core-only parts
(XML, SQL, networking) are conceptual references only — the Rust port
reimplements their behavior, not their API.

**REA limitations (important):**

- No native analysis provider is configured (`ghidra`, `hopper`, `ida`, `kuna`
  all unavailable). `search_strings` / decompilation returned
  `provider_unavailable`. Static function-level facts about the binary were
  therefore **not** observed; the complete C++ source tree in this repository
  is the ground truth for behavior.
- REA observations are static; they do not prove runtime behavior, endpoint
  reachability or absence of features.
- `rust-anything analyze` classifies this tree as C (not C++) with **441 parse
  errors**, 368 units, `start_here: []`, and maps every Qt dependency as
  `fit: unknown`. Its `bridge`/`gate` automation is **not** used here. The
  existing Catch2 suite is the characterization oracle instead (§10).

---

## 3. Decisions and scope

| Area | Decision |
|---|---|
| Scope | **Core parity**: movie/TV/concert browsing, library scan, NFO read/write, artwork, renamer, CSV/HTML export, main metadata scrapers. |
| Frontend | **Server-rendered HTML + HTMX**, SSE for job progress; no separate JS build. |
| Internal data | **New Rust-native SQLite schema.** No import of `MediaElch.sqlite`, `MediaElch.ini`, `advancedsettings.xml` or old SQL migrations. |
| NFO files | **Read and write.** NFO sidecars remain the source of truth for existing libraries (confirmed with user). |
| Scrapers (first wave) | **TMDB** (movie/TV/concert metadata + images), **TVMaze** (TV + episodes), **fanart.tv** (artwork). |
| Deferred | Kodi JSON-RPC sync, RAR/MakeMKV import, music library, adult-site and IMDb/OMDb/TheTVDB/Discogs/AllMusic/TvTunes scrapers, HD-Trailers. |

### In scope, concretely

- Scan configured media directories; detect movies, TV shows, episodes,
  concerts; parse season/episode numbers, stacking, extras, disc layouts.
- Read existing NFOs into the new DB; write NFOs back in Kodi v17–v22 format.
- Download and store posters, fanart, season images, actor images, episode
  thumbs, using the original file-naming conventions.
- Search and scrape from TMDB, TVMaze; artwork from fanart.tv.
- Rename files and directories with the placeholder grammar, including dry-run.
- Export CSV and HTML (template-based).
- Web UI for all of the above, with live progress for long jobs.

### Explicitly out of scope for v1

Kodi `VideoLibrary.*` JSON-RPC calls; watched-state import; `MakeMkvCon` and
`unrar` ingest; music artists/albums; the remaining scrapers; Qt translations
can be deferred to a second pass.

---

## 4. Target architecture

### 4.1 Crate graph

```
                       mediaelch-server  (axum, Askama, clap, tokio, SSE)
                        /      |       \          |
        mediaelch-web  mediaelch-fs  mediaelch-nfo  mediaelch-export
              |             |            |               |
              +------> mediaelch-core <--+---------------+
                       /      |       \
        mediaelch-db  mediaelch-scrape  mediaelch-image
                                  |            |
                            tmdb / tvmaze / fanarttv
                       mediaelch-renamer
```

Dependency rule: `mediaelch-core` depends on nothing internal and contains no
I/O. All crates are library crates; `mediaelch-server` is the only binary.

| Crate | Responsibility | Original source it replaces |
|---|---|---|
| `mediaelch-core` | Domain structs, value objects, enums, settings model | `src/data/`, `src/globals/Globals.h`, ID wrappers, `src/data/Rating.*`, `Certification.*`, `ResumeTime.*` |
| `mediaelch-db` | New SQLite schema, migrations, repositories, FTS search | `src/database/` (schema and persistence rewritten from scratch) |
| `mediaelch-fs` | Directory scanning, filename parsing, stacking, filters, name formatting | `src/file_search/`, `src/media/{FilenameUtils,FileFilter,NameFormatter,Path}.h` |
| `mediaelch-nfo` | Kodi NFO reader + writer, `DataFile` naming, stream-detail XML | `src/media_center/KodiXml.*`, `src/media_center/kodi/*`, `src/settings/DataFile.*` |
| `mediaelch-scrape` | `Scraper` trait, HTTP client, cache, proxy, rate limits, error mapping | `src/network/`, `src/scrapers/ScraperInterface.*`, `ScraperError.*`, `ScraperUtils.*`, `WebsiteCache.*` |
| `mediaelch-image` | Artwork download queue, resize, file layout, HTTP serving | `src/network/DownloadManager.*`, image parts of `src/media_center/KodiXml.cpp`, `src/media/ImageUtils.*` |
| `mediaelch-renamer` | Placeholder parser/evaluator, sanitization, dry-run, file ops | `src/renamer/` |
| `mediaelch-export` | CSV + HTML template export | `src/export/` |
| `mediaelch-server` | HTTP routes, templates, job scheduler, auth, config | `src/main.cpp`, `src/ui/` (entirely replaced) |
| `mediaelch-web` | Embedded templates, CSS, icons, i18n catalogs | `data/MediaElch.qrc`, `ui.qrc`, `data/i18n/*.ts` |

### 4.2 Technology choices

| Concern | Crate |
|---|---|
| HTTP server / routing | `axum` + `tokio` |
| Templates | `askama` (compile-time checked) |
| Interactivity | HTMX + SSE (`axum` streaming) |
| SQLite | `rusqlite` (bundled SQLite) + own migration runner; `FTS5` for search |
| Serialization | `serde`, `serde_json`, `toml` (config) |
| HTTP client | `reqwest` (rustls-tls), `governor` for rate limiting, `moka` or custom TTL cache |
| XML | `quick-xml` for writing, `roxmltree` for reading NFOs |
| HTML scraping (only if needed) | `scraper` + `html5ever` |
| Images | `image` (resize/encode) |
| CLI/config | `clap`, `figment` or plain TOML |
| Logging | `tracing` + `tracing-subscriber` (JSON logs for headless operation) |
| Auth | Session cookie + password hash (`argon2`); optional reverse proxy / TLS termination |

### 4.3 Deployment

- One statically linked Linux binary, assets embedded with `rust-embed` or
  `include_dir!`.
- SQLite file at `$MEDIAELCH_DATA/mediaelch.sqlite` (new file, distinct name so
  the old cache is never touched).
- Config at `$MEDIAELCH_CONFIG/config.toml`: HTTP bind address, media roots,
  API keys, proxy, scan interval, image sizes, log level.
- Dockerfile: `FROM scratch`-ish or `debian:stable-slim`, no Qt/X11 packages.
- Systemd unit example and reverse-proxy (TLS) notes in `docs/deployment/`.

---

## 5. New database schema

Fresh schema; no compatibility with `src/database/schema/*.sql`.

```sql
CREATE TABLE media_directory (
    id          INTEGER PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    kind        TEXT NOT NULL DEFAULT 'auto',   -- auto|movie|tvshow|concert
    recursive   INTEGER NOT NULL DEFAULT 1,
    last_scan   TEXT
);

CREATE TABLE media_item (
    id             INTEGER PRIMARY KEY,
    kind           TEXT NOT NULL CHECK (kind IN ('movie','tvshow','episode','concert')),
    parent_id      INTEGER REFERENCES media_item(id) ON DELETE CASCADE,
    directory      TEXT NOT NULL,
    primary_file   TEXT NOT NULL UNIQUE,
    title          TEXT NOT NULL,
    sort_title     TEXT,
    original_title TEXT,
    year           INTEGER,
    released       TEXT,                  -- ISO 8601 date
    overview       TEXT,
    runtime_min    INTEGER,
    certification  TEXT,
    rating         REAL,
    votes          INTEGER,
    rating_source  TEXT,
    imdb_id        TEXT,
    tmdb_id        INTEGER,
    tvmaze_id      INTEGER,
    season_number  INTEGER,
    episode_number INTEGER,
    nfo_path       TEXT,
    nfo_mtime      INTEGER,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX ix_item_kind ON media_item(kind);
CREATE INDEX ix_item_parent ON media_item(parent_id);
CREATE INDEX ix_item_title ON media_item(sort_title);

CREATE TABLE media_file (
    id         INTEGER PRIMARY KEY,
    item_id    INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    path       TEXT NOT NULL UNIQUE,
    size       INTEGER,
    mtime      INTEGER,
    stack_part INTEGER,
    is_extra   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE stream_detail (
    id          INTEGER PRIMARY KEY,
    file_id     INTEGER NOT NULL REFERENCES media_file(id) ON DELETE CASCADE,
    stream_type TEXT NOT NULL,           -- video|audio|subtitle
    codec       TEXT,
    resolution  TEXT,
    channels    INTEGER,
    language    TEXT
);

CREATE TABLE credit (
    id       INTEGER PRIMARY KEY,
    item_id  INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL,              -- actor|director|writer
    name     TEXT NOT NULL,
    role     TEXT,
    thumb_url TEXT,
    ordering INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE media_tag (item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
                        kind TEXT NOT NULL, value TEXT NOT NULL,
                        PRIMARY KEY (item_id, kind, value));
-- kind ∈ genre|tag|studio|country

CREATE TABLE image (
    id         INTEGER PRIMARY KEY,
    item_id    INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,            -- poster|fanart|banner|season|episode_thumb|actor|logo|cleartag
    season     INTEGER,
    source_url TEXT,
    local_path TEXT NOT NULL,
    language   TEXT,
    width      INTEGER,
    height     INTEGER
);

CREATE TABLE scrape_state (
    item_id    INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    provider   TEXT NOT NULL,
    external_id TEXT,
    last_scrape TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (item_id, provider)
);

CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);  -- JSON values

CREATE VIRTUAL TABLE item_fts USING fts5(title, original_title, overview,
                                         content='media_item', content_rowid='id');
```

Migrations live in `crates/mediaelch-db/migrations/*.sql`, are embedded and
applied in order on startup; a `schema_migrations` table records applied
versions. A migration test asserts the final schema matches a committed
`schema.sql` snapshot.

---

## 6. NFO reader / writer

### 6.1 Why NFO stays

NFO files are Kodi's sidecar metadata. They are user-owned, live next to the
media, and are the only data that survives a MediaElch reinstall. The new
server must read them so an existing library does not need re-scraping, and
write them so Kodi (and other tools) continue to work.

What is **not** read: the old `MediaElch.sqlite`, QSettings INI,
`advancedsettings.xml`, export theme caches.

### 6.2 Reader

- `NfoReader::read(path, kind) -> Result<MediaItemDraft>` using `roxmltree`.
- Support Kodi v17–v22 tag shapes, including both rating formats:
  - v16/v17 `<rating>`, `<votes>`;
  - newer `<rating name="..." default="true" max="10"><value>…</value><votes>…</votes></rating>`.
- Map: `<title>`, `<originaltitle>`, `<sorttitle>`, `<year>`/`<premiered>`,
  `<plot>`, `<runtime>`, `<mpaa>` → certification, `<genre>` (split on `/`),
  `<country>`, `<studio>`, `<tag>`, `<set>`, `<actor><name><role><thumb>`,
  `<uniqueid type="imdb|tmdb|tvmaze">`, `<thumb>`, `<fanart>`, `<resume>`,
  `<streamdetails>`.
- Episode wrapping: use the equivalent of `EpisodeXmlReader::makeValidEpisodeXml`
  when an episode NFO contains only a fragment.
- Unrecognized tags are ignored, not treated as errors; malformed XML yields a
  structured error and the scanner logs and continues.

### 6.3 Writer

- `NfoWriter::write(&MediaItem, kodi_version, options) -> Result<()>`.
- Deterministic output so tests can compare XML structurally: stable tag order,
  fixed indentation, generator tag with app name/version/Kodi version/UTC time.
- File naming follows the original `DataFile` patterns:
  movie `<basename>.nfo`, `tvshow.nfo`, episode `<basename>.nfo`, concert
  `<basename>.nfo`, `artist.nfo`, `album.nfo`; BluRay/DVD discs use
  `poster.jpg` / `fanart.jpg`.
- Image side effects: actor thumbs into `.actors/<name>.jpg`, extra fanart
  `fanart1.jpg…`, season posters `seasonXX-poster.jpg`, episode `thumb.jpg`,
  album booklets into `booklet/bookletNN.jpg`.

### 6.4 Precedence rule

1. Existing NFO values seed the DB and are treated as user data.
2. Scrapers fill **only missing** fields; they never overwrite NFO-sourced
   values by default.
3. An explicit "Refresh metadata from TMDB/TVMaze" action overwrites selected
   fields, records the override in `scrape_state`, and is reflected in the UI.
4. Writing NFOs is an explicit user action (or opt-in automatic write), never a
   silent side effect of scanning.

---

## 7. Library scanner (`mediaelch-fs`)

Port the behavior covered by the original unit tests:

- Movie discovery: directory traversal with exclusion of `.actors`, `extras`,
  `extrafanart`; BluRay `index.bdmv` and DVD `VIDEO_TS.IFO` detection; one movie
  per directory; stacking via `cd/dvd/pt/part/dis[ck]` patterns.
- TV discovery: skip `Extras`/`.actors`, detect disc layouts, group `part/cd`
  files, parse season numbers from `S(\d+)[ ._-]?E`, `(\d+)x\d+`,
  `Season…Episode`, `(\d+)\.\d{2,4}` with the original priority order; parse
  episode numbers from the known pattern list with the ambiguity heuristic.
- Filters: glob + regex excludes from config (replaces AdvancedSettings).
- Name formatting: exclude-word regexes and prettification.
- Scanner is incremental: compare `mtime`/`size` against `media_file` rows;
  skip unchanged items; emit progress events over SSE.

Reference sites in the original:
`src/file_search/movie/MovieFileSearcher.cpp:32-50,179-193`,
`MovieDirectorySearcher.cpp:134-233,273-335,352-398`,
`src/file_search/TvShowFileSearcher.cpp:199-286,301-355,357-449`,
`src/media/FilenameUtils.cpp:12-68`, `src/media/NameFormatter.cpp:19-120`,
`src/media/FileFilter.cpp:5-23`.

---

## 8. Scrapers

### 8.1 Trait and job model

```rust
#[async_trait]
pub trait MetadataScraper: Send + Sync {
    fn id(&self) -> &'static str;
    fn supported_kinds(&self) -> &'static [MediaKind];
    async fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>>;
    async fn load(&self, id: &ExternalId, details: &DetailSet,
                  lang: &Locale) -> Result<MediaItemDraft>;
}

#[async_trait]
pub trait ImageProvider: Send + Sync {
    async fn images(&self, id: &ExternalId, kind: MediaKind)
        -> Result<Vec<ImageCandidate>>;
}
```

Each REST scrape request creates a **job** with an id, status
(`queued|running|done|failed`), progress counter and result summary. Jobs run on
a bounded `tokio` worker pool; the browser follows them via SSE. This replaces
the original self-contained `QObject` search/scrape jobs, which were introduced
specifically to avoid stale-settings and use-after-free bugs — the Rust model
keeps the same isolation by making each job own its parameters.

### 8.2 Providers (first wave)

| Provider | Base URL | Auth | Scope |
|---|---|---|---|
| TMDB | `https://api.themoviedb.org/3` | API key from config/env | Movie + TV + concert search/load, `append_to_response`, `/find` by IMDb id, image CDN `https://image.tmdb.org/t/p/` |
| TVMaze | `https://api.tvmaze.com` | none | TV search, show + episodes, `embed[]` expansion; respect its rate limit by loading full episode lists |
| fanart.tv | `https://webservice.fanart.tv/v3/{movies,tv,music}` | API key + optional personal client key from config/env | Poster/fanart/logo/clearart/season images |

The original source contains hardcoded keys for TMDB, TheTVDB, fanart.tv and
TheAudioDb. **Do not reuse or re-embed them.** The user supplies their own keys
in config; keys are redacted in logs.

### 8.3 Shared infrastructure

- `reqwest` client with UA `MediaElch/<version>`, 10 s timeout, 5 redirects,
  optional HTTP/SOCKS5 proxy; Kodi/local traffic bypasses proxy.
- TTL cache (240 s default) keyed by method + URL + relevant headers, excluding
  UA — mirrors `WebsiteCache` semantics.
- HTTP 429 → `RateLimitReached` with backoff; no proactive limiter except
  TVMaze's full-list strategy.
- Structured errors mapping network/404/429/empty/parse failures to an API error
  enum, exposed as JSON with a stable code and human message.
- JSON parsed with `serde_json`; XML (MusicBrainz path, deferred) with
  `quick-xml`. HTML fixtures only if a deferred provider is added.

---

## 9. Artwork (`mediaelch-image`)

- Provider returns `ImageCandidate { thumb_url, original_url, language, season, hint }`.
- Download queue with 5 parallel requests and 3 retries on timeout; local
  paths (`file://`) supported for testing.
- Resize backdrops to configured max dimensions with `image`; do not upscale.
- Store using the NFO naming conventions in §6.3; invalidate old files when an
  image is replaced and update the `image` table transactionally.
- Serve through `/api/images/{id}` with caching headers, so the HTMX UI can
  render posters without exposing the filesystem.

---

## 10. Renamer and export

### 10.1 Renamer

Port the original grammar (see `src/renamer/PlaceholderParser.cpp`,
`docs/concepts/renamer.md`):

- Variables `<title>`, `<year>`, `<tmdbId>`, `<imdbId>`, `<extension>`, …;
  case-sensitive names; unknown variables are a **hard error** (the original
  silently removed them, which hid typos).
- Conditionals `{cond}…{/cond}`: body emitted only when truthy (non-empty string,
  non-zero number).
- Index/slice operator `<actors[0].name>`, `<str[:3]>`; field access with `.`.
- Number formats `{rating:2,3}`, `{year:6}`, `{year:06}`; floats use a dot as
  decimal separator and two decimals by default; NaN/∞ → `NaN`/`inf`/`-inf`.
- Functions `lower`, `upper`.
- Sanitize illegal filename characters; platform-aware length limits;
  case-insensitive collision detection.
- Always append the correct extension automatically.
- Dry-run first: `POST /api/rename/preview` returns old → new paths and errors;
  `POST /api/rename/apply` performs `rename` with rollback on partial failure.

### 10.2 Export

- CSV: separate column sets for movies, TV shows, episodes, concerts, with
  configurable separator and quoting (port `src/export/CsvExport.*`).
- HTML: simple `{{ VAR }}` + block template engine (port `SimpleEngine`), with
  themes uploaded as ZIP and verified by SHA-256 before extraction; block
  zip-slip paths.
- Exports are generated into a server-side directory and offered as downloads.

---

## 11. Web frontend (HTMX)

- All pages rendered with Askama: library list, item detail, search results,
  scrape job status, settings, renamer preview, export.
- HTMX attributes drive partial updates (`hx-get`, `hx-post`, `hx-target`,
  `hx-swap`); no SPA router, no bundled JS framework.
- Long jobs report progress over SSE (`text/event-stream`) and update a
  progress region; on completion HTMX triggers a refresh of the affected list.
- Static assets (CSS, small icons extracted from `data/MediaElch.qrc`) are
  embedded in the binary. Replace the aspect-ratio/codec flag SVG table with
  inline symbols or a small sprite.
- i18n: convert `data/i18n/MediaElch_*.ts` to JSON catalogs at build time;
  `t(key, lang)` used in templates. Defer full parity of all 19 locales to a
  later pass if needed.
- Accessibility: semantic HTML, keyboard-focusable controls, visible focus
  rings; target the same browsers the old Qt widgets effectively supported.

---

## 12. Verification and "gate"

`rust-anything` cannot gate this port (§2), so each unit is proven as follows:

1. **Characterization vectors** — translate the existing Catch2 pure-logic
   tests into golden JSONL cases and keep them under `tests/golden/`:
   - season/episode extraction: `test/unit/tv_shows/testEpisodeNumberExtraction.cpp`,
     `testSeasonNumberExtraction.cpp`
   - filename utilities: `test/unit/file/testStackedBaseName.cpp`,
     `testNameFormatter.cpp`, `test/unit/media/testFilenameUtils.cpp`,
     `testFileFilter.cpp`
   - helper/disc detection: `test/unit/globals/testHelper.cpp`
   - renamer: `test/unit/renamer/testPlaceholderParser.cpp`, `testRenamer.cpp`
   - export: `test/unit/export/testCsvExport.cpp`, `testTableWriter.cpp`
   - value objects: `test/unit/data/testRating.cpp`, `testCertification.cpp`,
     ID tests, `testResumeTime.cpp`
2. **NFO reader characterization** — parse the reference files under
   `test/resources/{movie,show,concert,music}/` and assert the extracted fields
   against the expectations encoded in
   `test/integration/media_center/testKodi_v18_*.cpp` and `testKodi_v20_show.cpp`.
3. **NFO writer characterization** — write the same items and compare XML
   structurally against those reference files (tag values and order, ignoring
   the generator timestamp).
4. **Schema test** — assert the final migrated schema matches a committed
   `crates/mediaelch-db/schema.sql` snapshot.
5. **Scraper fixtures** — saved JSON responses for TMDB/TVMaze and saved HTML
   for fanart.tv error/edge cases; no live network in CI. A separate manual
   smoke test with real keys validates endpoints.
6. **Per-unit gate** — a unit is done only when: `cargo fmt --check`,
   `cargo clippy -- -D warnings`, `cargo test`, fixture/golden comparisons all
   pass, no `unsafe` (or SAFETY-commented minimal unsafe), no
   `unimplemented!`/`unwrap` in fallible paths, and the HTTP API contract test
   for its routes passes.

---

## 13. Roadmap

| Phase | Deliverables | Exit criterion |
|---|---|---|
| **0 — Foundation** (1–2 wk) | Rust workspace, config/CLI, axum skeleton, `/healthz`, schema migrations, Docker + systemd examples, CI pipeline | Binary runs on a machine with no Qt or X11; DB migrations apply cleanly |
| **1 — Read-side** (2–4 wk) | Core + DB + scanner + NFO reader, REST list/get, HTMX library browser with posters | Can browse a real scanned library in a browser; golden scan/NFO-read tests pass |
| **2 — Write-side** (4–6 wk) | NFO writer, image download/layout + serving, renamer (preview then apply), CSV/HTML export | Full CRUD + rename + export through the UI; NFO write parity tests pass |
| **3 — Scraping** (6–10 wk) | `Scraper` trait, TMDB → TVMaze → fanart.tv, job queue + SSE, cache/proxy/backoff, configurable keys | Search and scrape from the browser; fixture tests pass; real-key smoke test passes |
| **4 — Hardening & cutover** (10–12 wk) | Auth/TLS, performance tuning on a large library, deployment docs, backup/rollback procedure, user migration notes | Old app can be retired; rollback path documented and rehearsed |

Each phase ends with a demo against a copy of a real library and a review of
remaining gaps before the next phase starts.

---

## 14. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Original UI/core coupling (ADR: "UI and Core are tightly coupled"; `Settings.h` includes `RenamerDialog.h`) | Treat C++ source as a behavioral specification, not code to translate mechanically; rely on tests and recorded fixtures. |
| NFO tag variants across Kodi versions | Version-aware reader/writer; keep the Kodi v18/v20 reference fixtures as acceptance tests. |
| External API changes, HTML drift | Pin saved fixtures; scrapers are best-effort with clear error reporting; keep provider code isolated behind traits. |
| Hardcoded API keys in original source | All keys come from config/env; redact in logs; document how to obtain them. |
| Large libraries and slow scans | Incremental scan via mtime/size; bounded worker pool; SSE progress; SQLite indexes + FTS. |
| Lost user data on overwrite | NFO values win by default; scraper refresh is explicit; renamer is dry-run by default; NFO writes are explicit/opt-in. |
| No Qt i18n parity at launch | Build-time `.ts → JSON`; ship English first, add locales incrementally. |
| Stream details came from libmediainfo/ffmpeg in the original | v1 stores what NFO `<streamdetails>` provides; native probing can be added later via `ffprobe` subprocess. |

---

## 15. Cutover

1. Run both systems against a **copy** of a representative library; compare
   scanned item counts, NFO reads and scraped field coverage.
2. Back up any NFO directories before enabling NFO writes (files are rewritten
   in place).
3. Point users at the server URL; the old Qt app remains installed but unused
   for one release cycle as a rollback option.
4. Never open the old `MediaElch.sqlite` from the Rust server — the new DB has
   a different filename and schema.

---

## Appendix A — Source-to-Rust mapping (quick reference)

| Original | Destination |
|---|---|
| `src/main.cpp`, `src/ui/**`, `src/model/**` | `mediaelch-server`, `mediaelch-web` (rewritten) |
| `src/data/**`, `src/globals/Globals.h` | `mediaelch-core` |
| `src/database/**` | `mediaelch-db` (new schema) |
| `src/file_search/**`, `src/media/{FilenameUtils,FileFilter,NameFormatter,Path}.*` | `mediaelch-fs` |
| `src/media_center/KodiXml.*`, `src/media_center/kodi/**`, `src/settings/DataFile.*` | `mediaelch-nfo` |
| `src/network/**`, `src/scrapers/{ScraperInterface,ScraperError,ScraperUtils,ScraperInfos}.*` | `mediaelch-scrape` |
| `src/scrapers/tmdb/**`, `src/scrapers/tv_show/tvmaze/**`, `src/scrapers/image/FanartTv*` | provider modules in `mediaelch-scrape` |
| `src/network/DownloadManager.*`, artwork parts of `KodiXml.cpp` | `mediaelch-image` |
| `src/renamer/**` | `mediaelch-renamer` |
| `src/export/**` | `mediaelch-export` |
| `src/settings/**` | config in `mediaelch-server` (no QSettings) |
| `src/import/**`, `src/ui/media_center/KodiSync.cpp`, other scrapers | deferred / dropped |

## Appendix B — REA tool/evidence log

| Tool | Result |
|---|---|
| `binary_session({})` | REA 6.10.1 connected; no native provider configured |
| `open_binary(MediaElch_2.12.0/usr/bin/MediaElch)` | ELF x86-64 target bound; SHA-256 `fad57e86…007d97` |
| `inspect_elf_dependencies(detail=summary)` | 14 DT_NEEDED, 2,092 imports, runner `$ORIGIN/../lib` — parent `ev_8f4794…c4a605ece3e4cb8` |
| `inspect_analysis_view(kind=elf-dependencies, collection=needed)` | full dependency list — `ev_3d80a4…c38319bf` |
| `inspect_artifact(detail=summary)` | root-only inventory; deep format analysis requires a native provider — `ev_0653008e…b5a0e` |
| `audit_release_exposure` | no debug companions/source maps/packaged payload — `ev_d4b70d5f…c7591` |
| `search_strings("themoviedb")` | rejected `provider_unavailable` (Ghidra/Hopper/IDA/Kuna not configured) |
| `rust-anything analyze` | language detected as C; 441 parse errors; no `start_here`; Qt deps `fit: unknown` — automation unsuitable for this C++ tree |

Close the REA session with `close_binary` when analysis is no longer needed;
evidence IDs remain available on the current connection for follow-ups.
