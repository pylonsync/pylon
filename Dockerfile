# syntax=docker/dockerfile:1.7
# Pylon runtime image — generic. Bundles the pylon binary + Bun + the
# @pylonsync/* SDK packages. Doesn't bake any specific app: customer
# code is expected at /app/app.ts at runtime (mounted via volume,
# Fly Machines `files` config, Kubernetes ConfigMap, etc.).
#
# This is what Pylon Cloud's control plane provisions for customer
# projects: one image, mount different code per project. It's also
# what self-hosters use — bind-mount your app.ts and you're up.
#
# Build (from repo root):
#   docker build -t pylon .
#
# Run with mounted code:
#   docker run -p 4321:4321 \
#     -v $(pwd)/myapp:/app \
#     -v $(pwd)/data:/data \
#     -e PYLON_CORS_ORIGIN=https://your-app.example.com \
#     pylon
#
# Self-hosters who want a baked-app image (so the container is
# self-contained and doesn't need a runtime mount) can write a
# trivial wrapper:
#   FROM pylon
#   COPY ./my-app.ts /app/app.ts

ARG RUST_VERSION=1.89

# ---- Studio UI stage --------------------------------------------------------
# `pylon-studio-api`'s build.rs hard-fails the cargo build if
# `crates/studio_api/web/dist/index.html` is missing — and dist/ is
# gitignored. Build it in a small, cacheable bun stage so cargo finds
# the bundle when it gets there. Splitting this out (vs running bun
# inside rust-builder) lets BuildKit cache the studio bundle independently
# of the cargo build, so changes to Rust code don't redo the bun build
# and vice versa.
FROM oven/bun:1.2 AS studio-builder
WORKDIR /studio
COPY crates/studio_api/web/package.json crates/studio_api/web/bun.lock ./
RUN bun install --frozen-lockfile
COPY crates/studio_api/web ./
RUN bun run build

# ---- Rust build stage -------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-bookworm AS rust-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
# Pull the built Studio bundle out of the bun stage so build.rs is
# satisfied. We only need dist/; node_modules is multi-hundred-MB and
# would balloon the image with no runtime value.
COPY --from=studio-builder /studio/dist ./crates/studio_api/web/dist
# BuildKit cache mounts: persist cargo's registry + git index + the
# target dir across builds so unchanged deps don't recompile. Pairs
# with cache-to: type=gha,mode=max in the workflow — the underlying
# tarballs and incremental `target/release` get reused on the next
# run instead of starting from a cold rust-builder layer. Cuts the
# warm-build dep recompile from ~6 min to under 30 seconds for the
# typical "I changed one .rs file" diff.
#
# The final binary needs to be copied OUT of the cache mount before
# the layer ends, otherwise the runtime stage can't find it — the
# cache is unmounted after RUN exits. `cp ... /usr/local/bin/` puts
# it on a real layer that COPY --from picks up.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin pylon \
    && cp /build/target/release/pylon /usr/local/bin/pylon

# ---- Runtime image ----------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL https://bun.sh/install | BUN_INSTALL=/usr/local bash \
    && ln -s /usr/local/bin/bun /usr/bin/bun

COPY --from=rust-builder /usr/local/bin/pylon /usr/local/bin/pylon
COPY --from=rust-builder /build/packages /pylon/packages

# Pre-create /app with the workspace deps wired in so customer code
# dropped at /app/app.ts can `import {entity, ...} from "@pylonsync/sdk"`
# without shipping its own node_modules. The SDK + functions + react +
# sync packages are versioned with this image — they line up with
# whatever pylon binary is bundled.
RUN mkdir -p /app/node_modules/@pylonsync \
    && ln -sfn /pylon/packages/sdk       /app/node_modules/@pylonsync/sdk \
    && ln -sfn /pylon/packages/functions /app/node_modules/@pylonsync/functions \
    && ln -sfn /pylon/packages/react     /app/node_modules/@pylonsync/react \
    && ln -sfn /pylon/packages/sync      /app/node_modules/@pylonsync/sync

RUN groupadd --system --gid 10001 pylon \
    && useradd --system --uid 10001 --gid 10001 --home-dir /app --shell /usr/sbin/nologin pylon \
    && mkdir -p /data \
    && chown -R pylon:pylon /data /app

ENV PYLON_DB_PATH=/data/pylon.db
ENV PYLON_FILES_DIR=/data/uploads
ENV PYLON_SESSION_DB=/data/sessions.db
ENV PYLON_FUNCTIONS_RUNTIME=/pylon/packages/functions/src/runtime.ts
# Default to dev mode so the container boots without forcing operators to
# pre-configure PYLON_CORS_ORIGIN. Lock it down in production by setting
# PYLON_DEV_MODE=false AND PYLON_CORS_ORIGIN=https://your-frontend.example.com
# via `fly secrets set` (or your platform's equivalent).
ENV PYLON_DEV_MODE=true

USER pylon:pylon
WORKDIR /app
# Pylon uses up to four adjacent ports:
#   4321 — HTTP API
#   4322 — WebSocket sync (PYLON_PORT + 1)
#   4323 — SSE fallback /events (PYLON_PORT + 2)
#   4324 — realtime shards (PYLON_PORT + 3)
# Reverse proxies and load balancers (ALB, Caddy, nginx, Fly Machines)
# need to forward all four for full functionality. Apps that only use
# the HTTP API can publish only 4321.
EXPOSE 4321 4322 4323 4324
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:4321/health || exit 1

# /app/app.ts comes from the runtime mount — bind volume, Fly `files`
# config, ConfigMap, etc. Fails fast if nothing's mounted there.
CMD ["pylon", "start", "app.ts"]
