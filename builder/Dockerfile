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

# Trixie, and Bun installed on top of it, rather than `FROM oven/bun:1`.
#
# The build runs `pylon build`, which executes the npm-shipped
# @pylonsync/cli-linux-x64 binary. Since 0.5.0 that binary dynamically links
# samael's SAML XMLDSig path against libxmlsec1, so it needs those libraries
# present or it dies before doing anything:
#
#   pylon: error while loading shared libraries: libxmlsec1-openssl.so.1:
#   cannot open shared object file: No such file or directory
#   error: script "build" exited with code 127
#
# That took every Smallware deploy down — and because the builder self-destructs
# and the control plane only watches for a COMPLETE marker, a failure four
# seconds in surfaced as "Artifact build timed out" fifteen minutes later.
#
# The release matters as much as the packages. The binary is compiled against
# libxmlsec1 1.3.x, and bookworm (which oven/bun:1 is built on) ships 1.2.37 —
# the mismatch the runtime Dockerfile already warns about, which fails at dlopen
# time rather than at install time. Keep this base in step with the runtime
# image's `debian:trixie-slim`.
FROM debian:trixie-slim

RUN apt-get update \
	&& apt-get install -y --no-install-recommends \
		ca-certificates curl unzip tar gzip git \
		# Runtime shared libs for samael's SAML XMLDSig verification path.
		# The pylon CLI links against these at startup, so a builder without
		# them cannot run `pylon build` at all.
		libxml2 libxmlsec1 libxmlsec1-openssl \
	&& rm -rf /var/lib/apt/lists/* \
	&& curl -fsSL https://bun.sh/install | BUN_INSTALL=/usr/local bash \
	&& ln -s /usr/local/bin/bun /usr/bin/bun

WORKDIR /build
COPY builder/build.ts /usr/local/lib/pylon-build.ts

# Fail the image build if the CLI's shared libraries are missing, rather than
# discovering it as a fifteen-minute deploy timeout in production.
RUN ldconfig -p | grep -q 'libxmlsec1-openssl\.so\.1' \
	&& ldconfig -p | grep -q 'libxml2\.so' \
	&& bun --version

# One-shot: run the build, upload, exit. The orchestrator destroys the machine.
ENTRYPOINT ["bun", "run", "/usr/local/lib/pylon-build.ts"]
