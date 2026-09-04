#!/bin/sh
# Pylon installer — served at https://www.pylonsync.com/install.sh
#
#   curl -fsSL https://www.pylonsync.com/install.sh | bash
#
# Options (env vars):
#   PYLON_VERSION=v0.3.309   install a specific release (default: latest)
#   PYLON_INSTALL=/usr/local install prefix; binary lands at $PYLON_INSTALL/bin/pylon
#                            (default: $HOME/.pylon)
#
# Canonical source: https://github.com/pylonsync/pylon/blob/main/scripts/install.sh
# The copy served by www.pylonsync.com lives in the control-plane app's
# public/ dir — keep the two in sync.
set -eu

REPO="pylonsync/pylon"
INSTALL_PREFIX="${PYLON_INSTALL:-$HOME/.pylon}"
BIN_DIR="$INSTALL_PREFIX/bin"

say() { printf '%s\n' "$*"; }
fail() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

# --- Detect platform ---------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"

# `uname -m` reports the architecture of the calling process, so a shell under
# Rosetta answers x86_64 on an Apple Silicon Mac and would send an M-series
# user to the "no prebuilt binary for Intel macOS" dead end below. Ask about
# the hardware instead: hw.optional.arm64 is 1 on every Apple Silicon Mac and
# readable from inside a translated process.
if [ "$os" = "Darwin" ] && [ "$arch" = "x86_64" ]; then
    if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
        arch="arm64"
    fi
fi

case "$os" in
Darwin)
    case "$arch" in
    arm64 | aarch64) target="aarch64-apple-darwin" ;;
    x86_64)
        fail "no prebuilt binary for Intel macOS yet.
  Install from source instead:  cargo install pylon-cli
  Or run via Docker:            docker run -p 4321:4321 ghcr.io/pylonsync/pylon:latest"
        ;;
    *) fail "unsupported macOS architecture: $arch" ;;
    esac
    ;;
Linux)
    case "$arch" in
    x86_64 | amd64)
        # The published Linux binary links glibc; musl systems (Alpine)
        # need Docker or a source build.
        if [ -e /etc/alpine-release ]; then
            fail "the prebuilt Linux binary needs glibc (Alpine ships musl).
  Run via Docker instead:  docker run -p 4321:4321 ghcr.io/pylonsync/pylon:latest
  Or build from source:    cargo install pylon-cli"
        fi
        target="x86_64-unknown-linux-gnu"
        ;;
    aarch64 | arm64)
        fail "no prebuilt binary for Linux arm64 yet.
  Run via Docker instead:  docker run -p 4321:4321 ghcr.io/pylonsync/pylon:latest
  Or build from source:    cargo install pylon-cli"
        ;;
    *) fail "unsupported Linux architecture: $arch" ;;
    esac
    ;;
MINGW* | MSYS* | CYGWIN* | Windows_NT)
    fail "this script installs the unix binary. On Windows, run the PowerShell installer instead:
  powershell -c \"irm https://www.pylonsync.com/install.ps1 | iex\""
    ;;
*)
    fail "unsupported OS: $os
  Run via Docker instead:  docker run -p 4321:4321 ghcr.io/pylonsync/pylon:latest"
    ;;
esac

# --- Resolve version ---------------------------------------------------------
if [ -n "${PYLON_VERSION:-}" ]; then
    version="$PYLON_VERSION"
    case "$version" in
    v*) ;;
    *) version="v$version" ;;
    esac
else
    # /releases/latest redirects to /releases/tag/<tag>; read the tag off the
    # final URL. No GitHub API call, so no rate-limit trouble in CI.
    latest_url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")" ||
        fail "could not resolve the latest release from github.com/$REPO"
    version="${latest_url##*/}"
    [ -n "$version" ] || fail "could not parse the latest release tag"
fi

asset="pylon-$version-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$version/$asset"

say "Installing pylon $version ($target) to $BIN_DIR"

# --- Download + verify -------------------------------------------------------
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL -o "$tmpdir/$asset" "$url" ||
    fail "download failed: $url
  If this is a brand-new release, the binary may still be building — retry in a few minutes."

if curl -fsSL -o "$tmpdir/$asset.sha256" "$url.sha256" 2>/dev/null; then
    expected="$(awk '{print $1}' "$tmpdir/$asset.sha256")"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$tmpdir/$asset" | awk '{print $1}')"
    else
        actual="$(shasum -a 256 "$tmpdir/$asset" | awk '{print $1}')"
    fi
    [ "$expected" = "$actual" ] || fail "checksum mismatch for $asset (expected $expected, got $actual)"
else
    say "warning: no checksum published for $asset; skipping verification"
fi

# --- Install -----------------------------------------------------------------
mkdir -p "$BIN_DIR"
tar -xzf "$tmpdir/$asset" -C "$tmpdir"
[ -f "$tmpdir/pylon" ] || fail "archive did not contain a 'pylon' binary"
mv "$tmpdir/pylon" "$BIN_DIR/pylon"
chmod +x "$BIN_DIR/pylon"

# The macOS archive carries the libxmlsec1 closure the binary links against,
# which it finds at lib/ next to itself (@loader_path). Without this the
# install completes and every run dies in dyld. The Linux archive has no lib/.
if [ -d "$tmpdir/lib" ]; then
    rm -f "$BIN_DIR"/lib/*.dylib 2>/dev/null || true
    mkdir -p "$BIN_DIR/lib"
    for dylib in "$tmpdir"/lib/*; do
        [ -e "$dylib" ] || continue
        mv "$dylib" "$BIN_DIR/lib/"
    done
fi

say "✓ pylon $version installed at $BIN_DIR/pylon"

# --- PATH hint ---------------------------------------------------------------
case ":$PATH:" in
*":$BIN_DIR:"*) ;;
*)
    say ""
    say "Add pylon to your PATH:"
    case "${SHELL:-}" in
    */zsh) say "  echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc" ;;
    */fish) say "  fish_add_path $BIN_DIR" ;;
    *) say "  echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.bashrc && source ~/.bashrc" ;;
    esac
    ;;
esac

say ""
say "Get started:"
say "  npm create @pylonsync/pylon@latest my-app"
say "  cd my-app && pylon dev"
