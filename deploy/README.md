# Deployment guide (headless server)

The rewrite is a single Rust binary with no Qt/X11 runtime. Provide a TOML
configuration file, a writable data directory and access to your media files.

## Configuration

Start from `config.example.toml`. Important sections:

- `[server] bind` — `127.0.0.1:8080` by default. Set `tls_cert` + `tls_key`
  together to let the binary terminate HTTPS itself, or leave them unset and
  terminate TLS at a reverse proxy.
- `[library] data_dir` — holds `mediaelch.sqlite` and `exports/`. Use a
  different directory from the legacy Qt application's data; the old cache is
  never opened.
- `[library] media_directories` — scan roots. A directory name containing
  `movie`, `show`/`tv`, or `concert` is classified accordingly; otherwise the
  scanner infers from filenames.
- `[scrapers]` — supply your own TMDB and fanart.tv API keys. Missing keys
  produce a clear job error, not a crash.
- `[auth] enabled = true` with a `password_hash` — **enable this before
  exposing the server to any untrusted network.**

Generate a hash:

```sh
mediaelch-server --hash-password 'choose-a-password'
```

Validate a config without starting the server:

```sh
mediaelch-server --check --config /etc/mediaelch/config.toml
```

Environment overrides use `MEDIAELCH_` prefixes with `__` for nesting, e.g.
`MEDIAELCH_SERVER__BIND=0.0.0.0:8080`,
`MEDIAELCH_SCRAPERS__TMDB_API_KEY=...`.

## Reverse proxy with TLS (recommended)

Bind the server to loopback and let a proxy terminate TLS.

Caddy:

```caddyfile
mediaelch.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

nginx:

```nginx
server {
    listen 443 ssl http2;
    server_name mediaelch.example.com;
    ssl_certificate     /etc/letsencrypt/live/mediaelch.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/mediaelch.example.com/privkey.pem;
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

The SSE endpoint (`/api/scrape/events`) needs proxy buffering disabled; nginx
users should add `proxy_buffering off;` for that location.

## systemd

See `deploy/mediaelch.service`. It runs as `mediaelch`, uses a hardened unit
and reads `/etc/mediaelch/config.toml`. If media lives under `/home`, adjust
`ProtectHome` / `ReadWritePaths`.

## Docker

`docker build -t mediaelch-server:local .` then mount a config file, a data
volume and media directories read-only. See the comment block at the top of
the root `Dockerfile`.

## Backups and cutover

See [backup.md](backup.md) for backup/restore commands and
[cutover.md](cutover.md) for the retirement checklist for the old Qt app.
