#!/usr/bin/env bash
# source-hash.sh — print a hash of what the WebTransport plugin is built
# from: this crate's sources and the dependency versions it resolves to.
# build-plugins.sh records it in plugins.sha256; CI compares.
set -euo pipefail
cd "$(dirname "$0")/../.."
{
	git ls-files -co --exclude-standard crates/shard-client-ffi/Cargo.toml crates/shard-client-ffi/src | sort | xargs cat
	# The resolved dependency tree, without paths (they differ per machine).
	cargo tree --locked --target all -p pylon-shard-client-ffi -e normal --prefix none --format '{p}' 2>/dev/null |
		sed 's/ (.*)$//' | sort -u
	# The plugin profile.
	sed -n '/^\[profile.plugin\]/,/^\[/p' Cargo.toml
} | shasum -a 256 | cut -d' ' -f1
