# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
# WebAuthn passkey verification needs OpenSSL development headers at build time.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# Pre-compile dependencies first to maximize Docker layer caching
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && printf 'fn main() {}\n' > src/main.rs && \
    cargo build --release --locked && \
    rm -rf src target/release/.fingerprint/beatlab-*

# Build Beatlab release binary (includes embedded web UI assets)
COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        ffmpeg \
        libssl3 \
        nodejs \
        python3 \
    && curl -fsSL https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp \
    && chmod 0755 /usr/local/bin/yt-dlp \
    && yt-dlp --version \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/beatlab /usr/local/bin/beatlab

RUN mkdir -p /data

ENV BEATLAB_ADDR=0.0.0.0:4533 \
    BEATLAB_DATA=/data \
    BEATLAB_CACHE_GB=5.0 \
    XDG_CACHE_HOME=/data/.cache \
    RUST_LOG=info

WORKDIR /data
VOLUME ["/data"]
EXPOSE 4533

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:4533/health >/dev/null || exit 1

CMD ["beatlab"]
