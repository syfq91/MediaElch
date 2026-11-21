# Build:  docker build -t mediaelch-server:local .
# Run:    docker run -d --name mediaelch \
#           -p 8080:8080 \
#           -v mediaelch-data:/var/lib/mediaelch \
#           -v /path/to/media:/media:ro \
#           -v $PWD/config.toml:/etc/mediaelch/config.toml:ro \
#           mediaelch-server:local
#
# No Qt, X11 or display libraries are needed at build or runtime.

FROM rust:1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p mediaelch-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 mediaelch \
    && mkdir -p /var/lib/mediaelch /etc/mediaelch \
    && chown -R mediaelch:mediaelch /var/lib/mediaelch

COPY --from=builder /build/target/release/mediaelch-server /usr/local/bin/mediaelch-server

ENV MEDIAELCH_CONFIG=/etc/mediaelch/config.toml \
    MEDIAELCH_LIBRARY__DATA_DIR=/var/lib/mediaelch

VOLUME ["/var/lib/mediaelch", "/media"]
EXPOSE 8080
USER mediaelch
ENTRYPOINT ["/usr/local/bin/mediaelch-server"]
