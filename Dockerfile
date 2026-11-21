# Build:
#   docker build -t mediaelch-server:local .
# Run:
#   docker run -d --name mediaelch \
#     -p 8080:8080 \
#     -v mediaelch-data:/var/lib/mediaelch \
#     -v /path/to/media:/media:ro \
#     -v "$PWD/config.toml:/etc/mediaelch/config.toml:ro" \
#     mediaelch-server:local
# Then open http://localhost:8080
#
# No Qt, X11 or display libraries are needed at build or runtime.

FROM rust:1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p mediaelch-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 mediaelch \
    && mkdir -p /var/lib/mediaelch /etc/mediaelch /media \
    && chown -R mediaelch:mediaelch /var/lib/mediaelch

COPY --from=builder /build/target/release/mediaelch-server /usr/local/bin/mediaelch-server

# The app defaults to 127.0.0.1:8080, which is unreachable from outside the
# container. Binding to all interfaces makes `-p 8080:8080` work.
ENV MEDIAELCH_CONFIG=/etc/mediaelch/config.toml \
    MEDIAELCH_LIBRARY__DATA_DIR=/var/lib/mediaelch \
    MEDIAELCH_SERVER__BIND=0.0.0.0:8080

# Graceful shutdown in the binary is triggered by SIGINT (tokio ctrl_c), while
# `docker stop` sends SIGTERM by default. Align the signal so shutdown drains.
STOPSIGNAL SIGINT

VOLUME ["/var/lib/mediaelch", "/media"]
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${MEDIAELCH_SERVER__BIND##*:}/healthz" >/dev/null || exit 1

USER mediaelch
ENTRYPOINT ["/usr/local/bin/mediaelch-server"]
