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
# NEXT_PUBLIC_* vars are baked into the client bundle at build time, so
# they need to be ARG-passed here — runtime env_file changes in compose
# won't update the served JS until the image is rebuilt. Defaults match
# the values in .env.example.
ARG NEXT_PUBLIC_BRAND_NAME=ChimpFlix
ARG NEXT_PUBLIC_BRAND_NAME_UPPER=
ARG APP_PUBLIC_ORIGIN=
ENV NEXT_TELEMETRY_DISABLED=1 \
    CHIMPFLIX_API_URL=${CHIMPFLIX_API_URL} \
    NEXT_PUBLIC_BRAND_NAME=${NEXT_PUBLIC_BRAND_NAME} \
    NEXT_PUBLIC_BRAND_NAME_UPPER=${NEXT_PUBLIC_BRAND_NAME_UPPER} \
    APP_PUBLIC_ORIGIN=${APP_PUBLIC_ORIGIN}
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN mkdir -p public && npm run build

# ---- Server runtime --------------------------------------------------------
FROM debian:bookworm-slim AS server
# Package set covers all three hardware-accel paths in one image:
#
#   * ffmpeg                          — universal media tooling.
#   * mesa-va-drivers                 — VAAPI userspace for AMD radeonsi
#                                       + older Intel i965. Activated when
#                                       the operator mounts /dev/dri into
#                                       the container and picks "vaapi".
#   * intel-media-va-driver-non-free  — Intel iHD driver. Required for
#                                       Quick Sync on 5th-gen+ Intel iGPUs
#                                       with the full codec set (HEVC
#                                       Main10 decode, HEVC encode). Lives
#                                       in Debian's non-free-firmware
#                                       component, so we have to enable
#                                       that component before apt sees it.
#   * i965-va-driver-shaders          — fallback for old (pre-5th-gen)
#                                       Intel iGPUs.
#   * libva-drm2 libva2               — the libva loader the drivers plug
#                                       into. Pulled transitively by
#                                       mesa-va-drivers but listed
#                                       explicitly so a slimmed-down
#                                       rebuild doesn't accidentally drop
#                                       them.
#   * NVENC/NVDEC                     — no package needed. The NVIDIA
#                                       Container Toolkit bind-mounts
#                                       libcuda.so.1 + the encoder/decoder
#                                       libs from the host driver at
#                                       runtime. ffmpeg's h264_nvenc /
#                                       hevc_nvenc are compiled in by the
#                                       Debian build via stub libraries.
#
# Bookworm splits non-free hardware support out into the
# `non-free-firmware` component — we enable it inline so the Intel iHD
# driver becomes installable. The bookworm-slim base uses deb822-format
# sources with signing-key pinning at /etc/apt/sources.list.d/debian.sources;
# we extend the Components line of that file in-place so the existing
# Signed-By value stays valid. No effect on systems that don't have an
# Intel iGPU.
RUN sed -i 's/^Components: main$/Components: main contrib non-free non-free-firmware/' \
        /etc/apt/sources.list.d/debian.sources && \
    apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    ca-certificates \
    tini \
    curl \
    mesa-va-drivers \
    intel-media-va-driver-non-free \
    i965-va-driver-shaders \
    libva-drm2 \
    libva2 \
    && rm -rf /var/lib/apt/lists/*
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
