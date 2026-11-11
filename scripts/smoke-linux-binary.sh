#!/usr/bin/env bash
# Starts a Linux `pylon` binary on stock distro images.
#
#   scripts/smoke-linux-binary.sh <path-to-pylon>
#
# Each image is the distro's base image with nothing installed. A binary
# that needs a library or a glibc the distro does not ship fails here the
# same way it fails for a user. almalinux:8 is the oldest supported glibc
# (2.28).
#
# The images come from Google's mirror of Docker Hub. Anonymous Docker Hub
# pulls are rate limited on shared CI runners.
set -euo pipefail

bin="$(cd "$(dirname "${1:?usage: smoke-linux-binary.sh <path-to-pylon>}")" && pwd)/$(basename "$1")"
registry="${SMOKE_REGISTRY:-mirror.gcr.io/library}"

for image in almalinux:8 debian:bookworm-slim debian:trixie-slim ubuntu:22.04 ubuntu:24.04; do
    printf '%-22s ' "$image"
    docker run --rm --quiet -v "$bin:/pylon:ro" "$registry/$image" /pylon --version
done
