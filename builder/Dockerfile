# pylon-builder — the ephemeral image the Pylon Cloud build pipeline runs on a
# throwaway, isolated Fly machine to turn a customer's source into a prebuilt
# artifact bundle (RFC pylon-cloud/docs/rfc-deploy-build-pipeline.md).
#
# It does NOT run the pylon runtime; it only fetches source (via a signed url —
# no GitHub token), `bun install`s, runs the app build, bundles full-prebuilt,
# and uploads to Tigris. Keep Bun roughly in step with the runtime image so the
# shipped node_modules is ABI-compatible with the runtime's Bun.
#
# Build/publish in CI (release.yml) → ghcr.io/pylonsync/pylon-builder. The
# control plane creates a machine from this image with PYLON_BUILD_* env, lets
# it run once, and destroys it.
FROM oven/bun:1

RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates tar gzip git \
	&& rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY builder/build.ts /usr/local/lib/pylon-build.ts

# One-shot: run the build, upload, exit. The orchestrator destroys the machine.
ENTRYPOINT ["bun", "run", "/usr/local/lib/pylon-build.ts"]
