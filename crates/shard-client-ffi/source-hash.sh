#!/usr/bin/env bash
# source-hash.sh — print a hash of what the WebTransport plugin is built
# from: this crate's sources, its build script, and the dependency
# versions it resolves to.
# build-plugins.sh records it in plugins.sha256; CI compares.
set -euo pipefail
# One collation everywhere: the hash must match between macOS and Linux.
export LC_ALL=C
cd "$(dirname "$0")/../.."

files="$(git ls-files -co --exclude-standard crates/shard-client-ffi/Cargo.toml crates/shard-client-ffi/build-plugins.sh crates/shard-client-ffi/src | sort)"
[ -n "$files" ] || { echo "source-hash.sh: no crate sources found" >&2; exit 1; }
# The resolved dependency tree, without paths (they differ per machine).
tree="$(cargo tree --locked --color never --target all -p pylon-shard-client-ffi -e normal --prefix none --format '{p}' | sed 's/ (.*)$//' | sort -u)"
[ -n "$tree" ] || { echo "source-hash.sh: cargo tree printed nothing" >&2; exit 1; }
# The plugin profile.
profile="$(sed -n '/^\[profile.plugin\]/,/^\[/p' Cargo.toml)"

# shasum on macOS and Linux; Git Bash on the Windows runner has only sha256sum.
# Both print "<hash>  -".
if command -v shasum >/dev/null 2>&1; then sha256() { shasum -a 256; }; else sha256() { sha256sum; }; fi

{
	printf '%s\n' "$files" | xargs cat
	printf '%s\n' "$tree"
	printf '%s\n' "$profile"
} | sha256 | cut -d' ' -f1
