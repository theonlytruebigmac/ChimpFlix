# syntax=docker/dockerfile:1.7
#
# ChimpFlix multi-stage build.
#
# Two final targets: `server` (Rust binary + ffmpeg) and `web` (Next.js
# standalone Node server). docker-compose builds both; `docker build
# --target=server` builds just one.

# ---- Rust builder ----------------------------------------------------------
FROM rust:1.85-slim-bookworm AS rust-build
WORKDIR /build
ENV CARGO_INCREMENTAL=0
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock* ./
COPY rust-toolchain.toml ./
COPY crates ./crates

RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --bin chimpflix-server && \
    cp target/release/chimpflix-server /chimpflix-server

# ---- Web builder -----------------------------------------------------------
FROM node:22-bookworm-slim AS web-build
WORKDIR /build
ARG CHIMPFLIX_API_URL=http://server:8080
ENV NEXT_TELEMETRY_DISABLED=1 \
    CHIMPFLIX_API_URL=${CHIMPFLIX_API_URL}
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN mkdir -p public && npm run build

# ---- Server runtime --------------------------------------------------------
FROM debian:bookworm-slim AS server
RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg ca-certificates tini curl && \
    rm -rf /var/lib/apt/lists/*
RUN groupadd --gid 1000 chimpflix && \
    useradd --uid 1000 --gid chimpflix --create-home --shell /usr/sbin/nologin chimpflix && \
    mkdir -p /data && chown -R chimpflix:chimpflix /data
COPY --from=rust-build /chimpflix-server /usr/local/bin/chimpflix-server
USER chimpflix:chimpflix
WORKDIR /home/chimpflix
ENV DATA_DIR=/data \
    BIND_ADDR=0.0.0.0:8080 \
    RUST_LOG=info,sqlx=warn
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health > /dev/null || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/chimpflix-server"]

# ---- Web runtime -----------------------------------------------------------
FROM node:22-bookworm-slim AS web
WORKDIR /app
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0
COPY --from=web-build --chown=node:node /build/public ./public
COPY --from=web-build --chown=node:node /build/.next/standalone ./
COPY --from=web-build --chown=node:node /build/.next/static ./.next/static
USER node
EXPOSE 3000
CMD ["node", "server.js"]
