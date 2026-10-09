# The cloud dev-workspace image: the runtime image of the same release plus
# the coding agent's browser and web search. Customer apps keep booting the
# lean runtime image; only Stack0 Build workspaces boot this one (the control
# plane's devImage). Built by release.yml right after the runtime image:
#
#   docker build -f docker/dev.Dockerfile \
#     --build-arg PYLON_IMAGE=ghcr.io/pylonsync/pylon:<version> .
#
# Same binary and boot script as the runtime image, so a workspace still
# reproduces the deployed app.
ARG PYLON_IMAGE=ghcr.io/pylonsync/pylon:latest
FROM ${PYLON_IMAGE}

USER root
# A browser for the coding agent: agent-browser (a CLI that drives Chromium over
# CDP) plus Debian's Chromium. The agent uses it to read web pages and to take
# screenshots of the pages it builds. dev-env-boot.sh tells the agent how.
#
# Chromium comes from Debian, not from `agent-browser install` (Chrome for
# Testing): Chrome for Testing has no Linux ARM64 build, a pinned download never
# gets security fixes, and apt pulls in exactly the libraries Chromium links
# against. Fonts: without them pages render with missing glyphs. Liberation
# matches Arial/Times/Courier metrics; Noto covers emoji and CJK text.
#
# agent-browser is pinned like OpenCode, for the same reason: two builds of one
# Pylon release must ship the same agent tooling. The npm tarball holds a native
# binary for every platform (114 MB); keep only this platform's binary and the
# bundled skills that `agent-browser skills get core` prints. The binary must
# stay one directory below skills/ for that lookup to work. Bump the version
# and the checksum together (`curl -fsSL <tarball> | sha256sum`).
ARG AGENT_BROWSER_VERSION=0.39.0
ARG AGENT_BROWSER_SHA256=f2fa3b29b14ec35675af41afe4284faccd135434f18986bfc3226ce934ed87ac
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium \
    fonts-liberation fonts-noto-color-emoji fonts-noto-cjk \
    && rm -rf /var/lib/apt/lists/* \
    && case "$(dpkg --print-architecture)" in \
         amd64) ab_bin=agent-browser-linux-x64 ;; \
         arm64) ab_bin=agent-browser-linux-arm64 ;; \
         *) echo "agent-browser has no build for $(dpkg --print-architecture)" >&2; exit 1 ;; \
       esac \
    && curl -fsSL -o /tmp/agent-browser.tgz \
         "https://registry.npmjs.org/agent-browser/-/agent-browser-${AGENT_BROWSER_VERSION}.tgz" \
    && echo "${AGENT_BROWSER_SHA256}  /tmp/agent-browser.tgz" | sha256sum -c - \
    && mkdir -p /opt/agent-browser \
    && tar -xzf /tmp/agent-browser.tgz -C /opt/agent-browser --strip-components=1 \
         package/LICENSE package/skills package/skill-data "package/bin/${ab_bin}" \
    && mv "/opt/agent-browser/bin/${ab_bin}" /opt/agent-browser/bin/agent-browser \
    && chmod -R a+rX /opt/agent-browser \
    && chmod 0755 /opt/agent-browser/bin/agent-browser \
    && ln -s /opt/agent-browser/bin/agent-browser /usr/local/bin/agent-browser \
    && rm /tmp/agent-browser.tgz
# Use Debian's Chromium instead of searching for a downloaded Chrome.
ENV AGENT_BROWSER_EXECUTABLE_PATH=/usr/bin/chromium
# Chromium's sandbox needs either a setuid helper (Debian does not ship one) or
# unprivileged user namespaces, which a container or Fly machine may not allow.
# agent-browser turns the sandbox off by itself for root and inside Docker, but
# not for the non-root `pylon` user on a Fly machine. Turn it off in every case,
# so the browser starts the same way for every user. The dev env is a
# single-tenant machine; the machine boundary is the isolation.
ENV AGENT_BROWSER_ARGS=--no-sandbox

# `search`: web search for the coding agent, through pylon-model-proxy.
COPY --chmod=0755 docker/search.ts /usr/local/bin/search

USER pylon:pylon
