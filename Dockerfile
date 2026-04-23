# syntax=docker/dockerfile:1.7
# Pylon runtime image. Builds one example app into a production container.
#
# Build (from repo root):
#   docker build --build-arg APP=examples/crm -t pylon-crm .
#
# Run locally:
#   docker run -p 4321:4321 -v $(pwd)/data:/data \
#     -e PYLON_CORS_ORIGIN=https://your-app.vercel.app \
#     pylon-crm

ARG RUST_VERSION=1.89

# ---- Rust build stage -------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-bookworm AS rust-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
RUN cargo build --release --bin pylon

# ---- Runtime image ----------------------------------------------------------
FROM debian:bookworm-slim

ARG APP=examples/crm

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL https://bun.sh/install | BUN_INSTALL=/usr/local bash \
    && ln -s /usr/local/bin/bun /usr/bin/bun

COPY --from=rust-builder /build/target/release/pylon /usr/local/bin/pylon
COPY --from=rust-builder /build/packages /pylon/packages
COPY --from=rust-builder /build/${APP} /app

# Install workspace deps for the example (links @pylonsync/* from /pylon/packages).
WORKDIR /app
RUN ln -s /pylon/packages/functions node_modules/@pylonsync/functions 2>/dev/null || true \
    && ln -s /pylon/packages/sdk node_modules/@pylonsync/sdk 2>/dev/null || true \
    && ln -s /pylon/packages/react node_modules/@pylonsync/react 2>/dev/null || true

RUN groupadd --system --gid 10001 pylon \
    && useradd --system --uid 10001 --gid 10001 --home-dir /app --shell /usr/sbin/nologin pylon \
    && mkdir -p /data \
    && chown -R pylon:pylon /data /app

ENV PYLON_DB_PATH=/data/pylon.db
ENV PYLON_FILES_DIR=/data/uploads
ENV PYLON_SESSION_DB=/data/sessions.db
ENV PYLON_DEV_MODE=false

USER pylon:pylon
EXPOSE 4321
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:4321/health || exit 1

CMD ["pylon", "dev", "app.ts"]
