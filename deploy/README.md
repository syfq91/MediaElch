# Deployment examples (Phase 0)

These files are examples for the headless Rust server. They are intentionally
minimal; production hardening (TLS, backups, monitoring) is part of Phase 4.

## Docker

```sh
cp config.example.toml config.toml
docker build -t mediaelch-server:local .
docker run -d --name mediaelch -p 8080:8080 \
  -v mediaelch-data:/var/lib/mediaelch \
  -v /path/to/media:/media:ro \
  -v "$PWD/config.toml:/etc/mediaelch/config.toml:ro" \
  mediaelch-server:local
curl http://127.0.0.1:8080/healthz
```

The image contains only the Rust binary plus CA certificates and timezone data.
It needs no Qt, X11 or display server.

## systemd

```sh
useradd --system --create-home mediaelch
install -d -o mediaelch -g mediaelch /var/lib/mediaelch /etc/mediaelch
install -m 0640 config.toml /etc/mediaelch/config.toml
install -m 0755 target/release/mediaelch-server /usr/local/bin/mediaelch-server
install -m 0644 deploy/mediaelch.service /etc/systemd/system/mediaelch.service
systemctl daemon-reload
systemctl enable --now mediaelch
```

If your media directories are under `/home`, adjust `ProtectHome` and
`ReadWritePaths` in the unit file.

## Configuration precedence

Built-in defaults < TOML file < `MEDIAELCH_*` environment variables < CLI flags.
Nested settings use `__` in environment variables, for example
`MEDIAELCH_SERVER__BIND=0.0.0.0:8080`.
Validate a configuration without starting the server with `--check`.
