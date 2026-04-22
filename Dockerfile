# syntax=docker/dockerfile:1.7
# Multi-stage build for the statecraft server binary.
#
# Build:
#   docker build -t statecraft:latest .
#
# Run:
#   docker run -p 4321:4321 -p 4322:4322 -p 4323:4323 -p 4324:4324 \
#       -v $(pwd)/data:/data \
#       -e STATECRAFT_ADMIN_TOKEN=change-me \
#       statecraft:latest dev
#
# Ports:
#   4321  - HTTP + Studio + API
#   4322  - WebSocket (change events)
#   4323  - SSE
#   4324  - Shard WebSocket

ARG RUST_VERSION=1.84
FROM rust:${RUST_VERSION}-slim-bookworm AS builder

# Build dependencies for bundled sqlite + tls.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

# Build the CLI binary only (statecraft).
RUN cargo build --release --bin statecraft

# ---- Runtime image ----------------------------------------------------------
FROM debian:bookworm-slim

# Runtime deps: only need the bun runtime if TypeScript functions are used.
# Users can run without Bun; /api/fn/* will just be unavailable.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL https://bun.sh/install | BUN_INSTALL=/usr/local bash || true

COPY --from=builder /build/target/release/statecraft /usr/local/bin/statecraft

# Copy the TypeScript function runtime so `/api/fn/*` works out of the box
# (only needed if the user mounts a `functions/` directory at /app/functions).
COPY --from=builder /build/packages/functions /usr/local/share/statecraft/packages/functions

ENV STATECRAFT_FUNCTIONS_RUNTIME=/usr/local/share/statecraft/packages/functions/src/runtime.ts
ENV STATECRAFT_DB_PATH=/data/statecraft.db
ENV STATECRAFT_FILES_DIR=/data/uploads
ENV STATECRAFT_DEV_MODE=false

# Run as a non-root user. Anything CAP_NET_BIND_SERVICE-adjacent (binding
# privileged ports) is not needed because 4321-4324 are all unprivileged.
# The UID/GID 10001 leaves the normal 1000-9999 range free for operator
# use when mounting host directories.
RUN groupadd --system --gid 10001 statecraft \
    && useradd --system --uid 10001 --gid 10001 --home-dir /app \
       --shell /usr/sbin/nologin statecraft \
    && mkdir -p /data /app \
    && chown -R statecraft:statecraft /data /app

WORKDIR /app
USER statecraft:statecraft

EXPOSE 4321 4322 4323 4324

VOLUME ["/data"]

# Healthcheck hits /health. `wget` is lighter than `curl` if we were to
# strip curl; keeping curl for now because the install script and TS
# runtime fetchers use it.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://localhost:4321/health || exit 1

ENTRYPOINT ["statecraft"]
CMD ["dev"]
