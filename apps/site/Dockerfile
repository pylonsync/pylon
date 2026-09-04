# syntax=docker/dockerfile:1.7
#
# pylonsync.com — the Pylon framework marketing site.
#
# Layers app.ts + functions/ + web/ + the shared UI package on top of the
# generic Pylon runtime image, which already carries the pylon binary, Bun, and
# the @pylonsync/* SDK packages symlinked into /app/node_modules.
#
# Build (context is the REPO ROOT, not this directory):
#   docker build -f apps/pylonsync-site/Dockerfile -t pylonsync-site .
#
# Unlike the control plane this image needs no DATABASE_URL, no volume, and no
# secrets: the app declares zero entities. The only runtime knobs are optional
# (PYLON_CONTROL_PLANE_URL, REVTRAIL_BASE_URL), both defaulted in code.
#
# Pinned rather than :latest so the FROM-layer cache busts on a bump and the new
# runtime is actually pulled — :latest can resolve to a cached old digest on a
# remote builder and silently ship a stale @pylonsync/functions.
#
# Keep this version equal to the exact @pylonsync versions in package.json.
# >= 0.3.356 is REQUIRED, not merely preferred: every earlier release refuses to
# boot an app with no entities ("SCHEMA_EMPTY — Schema has no entities"). This
# site is exactly that shape — SSR pages, no database — so on 0.3.355 or older
# the container crash-loops at startup.
ARG PYLON_IMAGE=ghcr.io/pylonsync/pylon:0.5.0
FROM ${PYLON_IMAGE}

# We're already at WORKDIR=/app in the base image, owned by the pylon user.
# COPY explicitly under that user so we don't trip the non-root permissions.
USER root
COPY --chown=pylon:pylon apps/pylonsync-site/app.ts /app/app.ts
COPY --chown=pylon:pylon apps/pylonsync-site/tsconfig.json /app/tsconfig.json
COPY --chown=pylon:pylon apps/pylonsync-site/functions /app/functions

# The SSR route tree (web/app/**/page.tsx) + globals.css. MUST be present
# before the codegen below, or the manifest ships zero SSR routes and every URL
# 404s. tsconfig.json (above) carries the `@/* → ./web/*` path map the SSR
# render and the client bundler resolve against.
COPY --chown=pylon:pylon apps/pylonsync-site/web /app/web

# The shared UI package. Its destination is load-bearing in two ways:
#   1. `workspace:*` in package.json resolves against /app/packages/* (see the
#      workspaces field there, which exists for this build).
#   2. web/app/globals.css declares `@source "../../packages/ui/src/**"` so
#      Tailwind scans it — nearly all of this site's markup lives in the
#      package, and without that scan every class is purged and the site
#      renders unstyled.
COPY --chown=pylon:pylon packages/ui /app/packages/ui

# Static assets. Pylon serves these verbatim from <cwd>/public/<path>, so
# /app/public/brand/pylon-icon.svg answers GET /brand/pylon-icon.svg. This is
# also where /install.sh (the `curl … | sh` framework installer) and
# /pylon-skill.md come from — both are advertised URLs, not decoration.
COPY --chown=pylon:pylon apps/pylonsync-site/public /app/public

# /pylon-skill.md is the agent skill we tell people to load ("Teach Claude to
# write Pylon"), and it LIVES in the framework repo at skills/pylon/SKILL.md.
# The committed copy under public/ is a hand-sync, and hand-syncs rot — the
# control plane's was found 363 lines behind canonical. Refresh from the source
# of truth at build time. Deliberately best-effort: `|| true` keeps a GitHub
# blip from failing a deploy, and the committed copy remains as the fallback —
# stale at worst, never missing.
RUN curl -fsSL --max-time 20 \
      https://raw.githubusercontent.com/pylonsync/pylon/main/skills/pylon/SKILL.md \
      -o /tmp/pylon-skill.md \
    && test -s /tmp/pylon-skill.md \
    && install -o pylon -g pylon -m 0644 /tmp/pylon-skill.md /app/public/pylon-skill.md \
    && echo "[build] pylon-skill.md refreshed from canonical" \
    || echo "[build] pylon-skill.md: keeping the committed copy"

# Install frontend deps (react, react-dom, radix, lucide, next-themes,
# @tailwindcss/cli, …). The base image symlinks @pylonsync/* into node_modules
# but NOT these, and there is no workspace hoisting inside the image — so
# without an install here BOTH the SSR render (react-dom/server) and the client
# bundler (Bun.build resolving react/radix/etc., Tailwind via @tailwindcss/cli)
# fail, and the site 500s or ships dead HTML. Full install (not --production):
# @tailwindcss/cli lives in devDependencies but is needed at RUNTIME to compile
# web/app/globals.css.
COPY --chown=pylon:pylon apps/pylonsync-site/package.json /app/package.json
RUN cd /app && bun install && chown -R pylon:pylon /app/node_modules

# Generate the manifest INSIDE the build, not from the repo. It is gitignored
# (a build artifact derived from app.ts + the discovered web/app routes), so a
# clone + deploy would otherwise miss it. Generating here keeps the deployed
# manifest in lockstep with the source it was built from. It also must exist
# BEFORE first render: without it the client bundler logs "building the client
# bundle WITHOUT manifest-derived features" and the declared fonts are silently
# dropped from every page.
RUN pylon codegen app.ts --out pylon.manifest.json && \
    chown pylon:pylon pylon.manifest.json
USER pylon:pylon

# Production: stricter CORS, no studio inspector by default.
ENV PYLON_DEV_MODE=false

# Let the image-pinned @pylonsync/* packages resolve their react / react-dom
# peer deps. The base image symlinks @pylonsync/* into /app/node_modules →
# /pylon/packages/*; those declare react as a peerDependency, but react only
# lives in /app/node_modules and /pylon/packages/react can't reach it by upward
# traversal (it is outside /app). Without NODE_PATH the SSR render dies with
# "Cannot find package 'react'" and every route 500s.
ENV NODE_PATH=/app/node_modules

# Pylon's default CMD ["pylon", "start", "app.ts"] is inherited.
