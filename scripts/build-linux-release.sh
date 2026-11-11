#!/usr/bin/env bash
# Builds the published Linux `pylon` binary.
#
#   scripts/build-linux-release.sh [--test]
#
# The build runs in the docker/linux-release image: glibc 2.28 with static
# libxml2, xmlsec1 and OpenSSL. The binary lands at
#   target/linux-release/<triple>/release/pylon
# for the architecture of the Docker host, and is checked with
# scripts/verify-linux-binary.sh.
#
# Environment:
#   PYLON_LINUX_RELEASE_IMAGE       use this image instead of building one
#   PYLON_LINUX_RELEASE_CARGO_HOME  cargo registry cache
#                                   (default: target/linux-release/cargo-home)
#
# --test also runs the pylon-auth tests against the static libraries,
# including the tests that sign and verify a SAML response through xmlsec1.
#
# crates/studio_api/web/dist must exist (bun run build in crates/studio_api/web).
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
image="${PYLON_LINUX_RELEASE_IMAGE:-pylon-linux-release}"
run_tests=0
[ "${1:-}" = "--test" ] && run_tests=1

if [ ! -f "$repo/crates/studio_api/web/dist/index.html" ]; then
    echo "error: crates/studio_api/web/dist is missing. Run: (cd crates/studio_api/web && bun install && bun run build)" >&2
    exit 1
fi

if [ -z "${PYLON_LINUX_RELEASE_IMAGE:-}" ]; then
    docker build -t "$image" "$repo/docker/linux-release"
fi

# The cargo registry and the target directory live on the host, so a second
# run is incremental and the files belong to the calling user.
cargo_home="${PYLON_LINUX_RELEASE_CARGO_HOME:-$repo/target/linux-release/cargo-home}"
mkdir -p "$cargo_home" "$repo/target/linux-release"
docker run --rm \
    --user "$(id -u):$(id -g)" \
    -v "$repo:/work" \
    -e HOME=/tmp \
    -v "$cargo_home:/cargo-home" \
    -e CARGO_HOME=/cargo-home \
    -e CARGO_TARGET_DIR=/work/target/linux-release \
    -e RUN_TESTS="$run_tests" \
    "$image" bash -euo pipefail -c '
        triple="$(rustc -vV | sed -n "s/^host: //p")"
        cargo build --release --locked --bin pylon --target "$triple"
        if [ "$RUN_TESTS" = 1 ]; then
            # --include-ignored with the opt-in runs saml_cert_trust: it signs
            # a SAML response and verifies it, both through xmlsec1.
            PYLON_RUN_XMLSEC_SIGN_TESTS=1 cargo test --release --locked \
                -p pylon-auth --target "$triple" -- --include-ignored
        fi
        scripts/verify-linux-binary.sh "target/linux-release/$triple/release/pylon"
    '
