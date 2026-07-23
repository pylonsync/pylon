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
# Trixie (not bookworm) because samael 0.0.20 expects libxmlsec1 1.3.x's
# `size_t`-typed `xmlSecSize`. Bookworm ships libxmlsec1 1.2.37 where
# the type is `unsigned int` — bindgen emits `u32` and samael fails to
# compile with `expected usize, found u32`. Trixie ships 1.3.x.
FROM rust:${RUST_VERSION}-slim-trixie AS rust-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev ca-certificates \
    # libxml2 + xmlsec1: SAML 2.0 XMLDSig signature verification
    # (samael's xmlsec feature). Both -dev for build-time linking; the
    # runtime stage installs the non-dev runtime libraries.
    libxml2-dev libxmlsec1-dev libxmlsec1-openssl \
    libclang-dev clang \
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
# Match the build stage on trixie so the linked libxmlsec1 ABI lines
# up with what the binary was compiled against. Mixing bookworm runtime
# with trixie-built binary would fail at dlopen time.
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl unzip \
    # Runtime shared libs for samael's SAML XMLDSig verification path —
    # the binary dynamically links against these at startup.
    libxml2 libxmlsec1 libxmlsec1-openssl \
    # sqlite3 CLI: Pylon Cloud's getProjectDatabaseStats shells in via
    # the Fly Machines exec API to read table + row counts off the
    # customer's /data/pylon.db. Without this, the dashboard's
    # Database tab silently shows 0 tables / 0 rows.
    sqlite3 \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL https://bun.sh/install | BUN_INSTALL=/usr/local bash \
    && ln -s /usr/local/bin/bun /usr/bin/bun

# OpenCode — the colocated coding agent for cloud dev-mode envs
# (build.pylonsync.com). Installed to /usr/local so the non-root `pylon` user
# can run it; dev-env-boot.sh starts `opencode serve` next to `pylon dev` when
# the control plane grants model access (via pylon-model-proxy, never a raw key).
RUN BUN_INSTALL=/usr/local bun install -g opencode-ai

COPY --from=rust-builder /usr/local/bin/pylon /usr/local/bin/pylon
# Cloud dev-mode env bootstrap. Not used by the default `pylon start` CMD —
# Pylon Cloud's provisionDevEnvironment sets this as a machine's init.cmd to
# turn it into a live, mutable `pylon dev` workspace (seed → bun install →
# pylon dev) that an agent / build.pylonsync.com authors via the file-write API.
COPY --from=rust-builder /build/docker/dev-env-boot.sh /usr/local/bin/dev-env-boot.sh
RUN chmod 0755 /usr/local/bin/dev-env-boot.sh
COPY --from=rust-builder /build/packages /pylon/packages
# The monorepo's root tsconfig.base.json — each @pylonsync/* package
# (react, client, sync, loro, sdk, next, workflows) has a tsconfig.json
# whose "extends" points at "../../tsconfig.base.json". From a customer's
# web/ vite build that walk-up-resolves an @pylonsync/* tsconfig at
# /pylon/packages/<name>/tsconfig.json, "../../" lands at /pylon/. Ship
# the base config there so vite's esbuild plugin can resolve it instead
# of dying with [vite:esbuild] failed to resolve "extends":"../../tsconfig.base.json".
COPY --from=rust-builder /build/tsconfig.base.json /pylon/tsconfig.base.json
# Shared example UI components used across examples/ dogfood apps.
# Lives at examples/_shared in the repo; aliased into /pylon/packages so
# stage_workspace_symlinks (crates/cli/src/bun.rs) discovers it the same
# way as framework packages when an example app deploys with a
# `"@pylonsync/example-ui": "workspace:*"` dep.
COPY --from=rust-builder /build/examples/_shared /pylon/packages/example-ui

# Install example-ui's transitive deps (@radix-ui/*, clsx, lucide-react,
# etc.) so a customer's web/ build can resolve them through Node's
# module-resolution walk. .dockerignore excludes **/node_modules from
# the build context, so the COPY above ships only source — without
# this RUN, the chat dogfood (and any other example using
# `@pylonsync/example-ui`) crashes at vite-build time with
# "Cannot find module @radix-ui/react-slot". Producing the install at
# image-build time is much cheaper than re-doing it on every
# customer-machine boot.
RUN cd /pylon/packages/example-ui && bun install --production

# Pre-install the non-workspace runtime deps of the @pylonsync/* packages
# (clsx for client, loro-crdt for loro) into /pylon/node_modules/ so a
# customer's web build can resolve them through Node's walk-up from
# /pylon/packages/<name>/src/. Without this, vite/rollup dies with
# "Rollup failed to resolve import 'loro-crdt' from
# /pylon/packages/loro/src/registry.ts" — the package is imported but
# /pylon/node_modules/ only has the @pylonsync/* symlinks; no real npm
# deps. Keep this dependency list in sync with the `dependencies` blocks
# across packages/*/package.json; new non-workspace transitive deps go
# here.
RUN <<'EOF'
cat > /pylon/package.json <<'JSON'
{
  "name": "pylon-framework-image-deps",
  "version": "0.0.0",
  "private": true,
  "dependencies": {
    "clsx": "*",
    "loro-crdt": "*",
    "satori": "*",
    "@resvg/resvg-wasm": "*"
  }
}
JSON
cd /pylon && bun install --production
EOF

# Pre-create /app with the workspace deps wired in so customer code
# dropped at /app/app.ts can `import {entity, ...} from "@pylonsync/sdk"`
# without shipping its own node_modules. The SDK + functions + react +
# sync packages are versioned with this image — they line up with
# whatever pylon binary is bundled.
RUN mkdir -p /app/node_modules/@pylonsync \
    && ln -sfn /pylon/packages/sdk         /app/node_modules/@pylonsync/sdk \
    && ln -sfn /pylon/packages/functions   /app/node_modules/@pylonsync/functions \
    && ln -sfn /pylon/packages/react       /app/node_modules/@pylonsync/react \
    && ln -sfn /pylon/packages/sync        /app/node_modules/@pylonsync/sync \
    && ln -sfn /pylon/packages/client      /app/node_modules/@pylonsync/client \
    && ln -sfn /pylon/packages/loro        /app/node_modules/@pylonsync/loro \
    && ln -sfn /pylon/packages/next        /app/node_modules/@pylonsync/next \
    && ln -sfn /pylon/packages/plugins     /app/node_modules/@pylonsync/plugins \
    && ln -sfn /pylon/packages/workflows   /app/node_modules/@pylonsync/workflows \
    && ln -sfn /pylon/packages/example-ui  /app/node_modules/@pylonsync/example-ui

# Same symlink set under /pylon/node_modules/@pylonsync/ so vite + Node
# module resolution can walk UP from any file inside /pylon/packages/<name>/
# and still find sibling @pylonsync/* deps. Concretely: when a customer's
# web build bundles /pylon/packages/react/src/index.ts, rollup walks
# /pylon/packages/react/src → /pylon/packages/react → /pylon/packages →
# /pylon/node_modules/@pylonsync/sdk. Without these symlinks, rollup
# died with "Rollup failed to resolve import @pylonsync/sdk from
# /pylon/packages/react/src/index.ts". Mirror the /app/ set verbatim so
# any cross-package import (react → sdk, client → react, etc.) resolves.
RUN mkdir -p /pylon/node_modules/@pylonsync \
    && ln -sfn /pylon/packages/sdk         /pylon/node_modules/@pylonsync/sdk \
    && ln -sfn /pylon/packages/functions   /pylon/node_modules/@pylonsync/functions \
    && ln -sfn /pylon/packages/react       /pylon/node_modules/@pylonsync/react \
    && ln -sfn /pylon/packages/sync        /pylon/node_modules/@pylonsync/sync \
    && ln -sfn /pylon/packages/client      /pylon/node_modules/@pylonsync/client \
    && ln -sfn /pylon/packages/loro        /pylon/node_modules/@pylonsync/loro \
    && ln -sfn /pylon/packages/next        /pylon/node_modules/@pylonsync/next \
    && ln -sfn /pylon/packages/plugins     /pylon/node_modules/@pylonsync/plugins \
    && ln -sfn /pylon/packages/workflows   /pylon/node_modules/@pylonsync/workflows \
    && ln -sfn /pylon/packages/example-ui  /pylon/node_modules/@pylonsync/example-ui

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
