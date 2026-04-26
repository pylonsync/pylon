#!/usr/bin/env bash
# pylon installer
#
# Usage:
#   curl -fsSL https://pylonsync.com/install.sh | bash
#
# Honors:
#   PYLON_VERSION   release tag to install (default: latest)
#   PYLON_INSTALL_DIR  install location (default: $HOME/.local/bin)

set -euo pipefail

REPO="pylonsync/pylon-releases"
INSTALL_DIR="${PYLON_INSTALL_DIR:-$HOME/.local/bin}"

err() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
info() { printf '\033[32m==>\033[0m %s\n' "$*"; }

# Detect OS + arch
uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s" in
  Linux)  os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  *) err "unsupported OS: $uname_s" ;;
esac

case "$uname_m" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *) err "unsupported architecture: $uname_m" ;;
esac

target="${arch}-${os}"

# Resolve version
if [ -z "${PYLON_VERSION:-}" ]; then
  info "Resolving latest release..."
  PYLON_VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | grep '"tag_name"' \
    | head -n1 \
    | sed -E 's/.*"tag_name"[^"]*"([^"]+)".*/\1/')
  [ -n "$PYLON_VERSION" ] || err "could not resolve latest version"
fi

archive="pylon-${PYLON_VERSION}-${target}.tar.gz"
url="https://github.com/$REPO/releases/download/${PYLON_VERSION}/${archive}"

info "Downloading $url"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fSL "$url" -o "$tmp/$archive" || err "download failed: $url"

info "Extracting..."
tar -xzf "$tmp/$archive" -C "$tmp"

# Find the pylon binary (top of archive or in a subdir)
binary="$(find "$tmp" -name pylon -type f -perm -u+x | head -n1)"
[ -n "$binary" ] || err "no pylon binary found in archive"

mkdir -p "$INSTALL_DIR"
install -m 0755 "$binary" "$INSTALL_DIR/pylon"

info "Installed pylon $PYLON_VERSION to $INSTALL_DIR/pylon"

if ! printf '%s' ":$PATH:" | grep -q ":$INSTALL_DIR:"; then
  printf '\n\033[33mwarning:\033[0m %s is not in your PATH\n' "$INSTALL_DIR" >&2
  printf '  Add this to your shell profile:\n'
  printf '    export PATH="%s:$PATH"\n' "$INSTALL_DIR"
fi

"$INSTALL_DIR/pylon" version || true
