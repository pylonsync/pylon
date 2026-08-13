#!/usr/bin/env bash
# Make a macOS `pylon` build self-contained.
#
# The SAML signature verifier links libxmlsec1, which on macOS comes from
# Homebrew. A freshly built binary therefore carries absolute LC_LOAD_DYLIB
# paths into /opt/homebrew, and dyld aborts on any machine that doesn't have
# that exact prefix populated — no Homebrew, Homebrew at the Intel prefix
# (/usr/local, which is what Migration Assistant carries over from an Intel
# Mac), or Homebrew without `brew install libxmlsec1`. Since the binary ships
# through npm, `npm install @pylonsync/cli` otherwise hands people something
# that cannot start, with a dyld error naming a library they never heard of.
#
# This copies the transitive closure of non-system dylibs next to the binary
# and rewrites every reference to @loader_path/lib/<name>, so the whole set
# relocates together and resolves with no dependency on the host.
#
#   <dir>/pylon      → refs @loader_path/lib/libxmlsec1.10311.dylib
#   <dir>/lib/*.dylib
#
# @loader_path (not @executable_path) so the same signed binary works in both
# shipping layouts: the npm package's bin/pylon + bin/lib/, and the release
# tarball's pylon + lib/. dyld resolves it against the real path of whatever
# is doing the loading, so a PATH symlink to the binary is also fine.
#
# Referenced basenames are kept verbatim, including libxmlsec1's point version
# (.10311.). We ship the exact library we linked against, so the compat-version
# check dyld does on load is guaranteed to pass — there is no Homebrew bottle
# to drift out from under it.
#
# Usage: scripts/bundle-macos-dylibs.sh <path-to-binary>

set -euo pipefail

BIN_ARG="${1:?usage: bundle-macos-dylibs.sh <path-to-binary>}"
[ -f "$BIN_ARG" ] || {
	echo "bundle-macos-dylibs: no such file: $BIN_ARG" >&2
	exit 1
}

BIN_DIR="$(cd "$(dirname "$BIN_ARG")" && pwd)"
BIN="$BIN_DIR/$(basename "$BIN_ARG")"
LIB_DIR="$BIN_DIR/lib"

# macOS runners ship bash 3.2 — no associative arrays, no mapfile. The
# visited set is a plain file.
SEEN="$(mktemp -t pylon-dylib-seen)"
PENDING="$(mktemp -t pylon-dylib-pending)"
cleanup() { rm -f "$SEEN" "$PENDING"; }
trap cleanup EXIT

# Direct LC_LOAD_DYLIB entries. Line 1 of otool -L is the filename header;
# for a dylib the first entry after it is the library's own install name,
# which the visited set dedupes.
deps_of() {
	otool -L "$1" 2>/dev/null | tail -n +2 | awk '{print $1}'
}

# Anything under /usr/lib or /System comes from the OS and is present on
# every Mac. Already-relocated refs (@loader_path, @rpath, @executable_path)
# are ours and need no further work.
is_bundled_dep() {
	case "$1" in
	/usr/lib/* | /System/* | @* | "") return 1 ;;
	*) return 0 ;;
	esac
}

seen() { grep -qxF "$1" "$SEEN" 2>/dev/null; }

echo "→ resolving dylib closure for $BIN"
: >"$PENDING"
printf '%s\n' "$BIN" >>"$PENDING"
printf '%s\n' "$BIN" >>"$SEEN"

# Breadth-first walk. PENDING is consumed from the top and appended to, so a
# newly discovered dylib gets its own dependencies walked too.
while [ -s "$PENDING" ]; do
	current="$(head -n 1 "$PENDING")"
	tail -n +2 "$PENDING" >"$PENDING.rest" && mv "$PENDING.rest" "$PENDING"

	for dep in $(deps_of "$current"); do
		is_bundled_dep "$dep" || continue
		seen "$dep" && continue
		printf '%s\n' "$dep" >>"$SEEN"
		printf '%s\n' "$dep" >>"$PENDING"
	done
done

# Drop the binary itself; what remains is the set to vendor.
CLOSURE="$(grep -vxF "$BIN" "$SEEN" || true)"

if [ -z "$CLOSURE" ]; then
	echo "→ no non-system dylibs — binary is already self-contained, nothing to do"
	exit 0
fi

mkdir -p "$LIB_DIR"

echo "→ vendoring $(printf '%s\n' "$CLOSURE" | wc -l | tr -d ' ') dylibs into $LIB_DIR"
for src in $CLOSURE; do
	name="$(basename "$src")"
	# Resolve symlinks: /opt/homebrew/opt/... points into ../Cellar/... and we
	# want the regular file, stored under the name the binary references.
	if [ ! -e "$src" ]; then
		echo "bundle-macos-dylibs: dependency not found on disk: $src" >&2
		exit 1
	fi
	cp -L "$src" "$LIB_DIR/$name"
	chmod u+w "$LIB_DIR/$name"
	echo "   $src → lib/$name"
done

# Rewrite references. Every vendored dylib gets an install name of
# @loader_path/lib/<name> so that a dylib loaded BY the binary looks for its
# own siblings in the same lib/ dir the binary loaded it from — @loader_path
# for a dylib resolves against that dylib's directory, so the path from
# lib/foo.dylib to lib/bar.dylib is plain @loader_path/bar.dylib.
echo "→ rewriting load paths"
for src in $CLOSURE; do
	name="$(basename "$src")"
	target="$LIB_DIR/$name"

	install_name_tool -id "@loader_path/lib/$name" "$target"

	# Inter-dylib references, e.g. libxmlsec1-openssl → libcrypto.
	for dep in $(deps_of "$target"); do
		is_bundled_dep "$dep" || continue
		install_name_tool -change "$dep" "@loader_path/$(basename "$dep")" "$target"
	done
done

for dep in $(deps_of "$BIN"); do
	is_bundled_dep "$dep" || continue
	install_name_tool -change "$dep" "@loader_path/lib/$(basename "$dep")" "$BIN"
	echo "   pylon: $dep → @loader_path/lib/$(basename "$dep")"
done

# install_name_tool invalidates LC_CODE_SIGNATURE, and arm64 macOS refuses to
# load a Mach-O whose signature doesn't match. Re-sign ad hoc, dependencies
# before the binary. CLI binaries aren't notarized (the docs tell users to
# clear the quarantine attribute), so ad hoc is the right level.
echo "→ re-signing"
for src in $CLOSURE; do
	codesign --force --sign - "$LIB_DIR/$(basename "$src")" 2>/dev/null
done
codesign --force --sign - "$BIN" 2>/dev/null

# Verify nothing outside the OS is still referenced by absolute path. A miss
# here means a user hits "Library not loaded" instead of us hitting it now.
echo "→ verifying"
leaked=0
for f in "$BIN" "$LIB_DIR"/*.dylib; do
	for dep in $(deps_of "$f"); do
		if is_bundled_dep "$dep"; then
			echo "bundle-macos-dylibs: $f still references $dep" >&2
			leaked=1
		fi
	done
done
[ "$leaked" -eq 0 ] || exit 1

# Actually load it. A botched re-sign produces a binary that passes every
# static check above and is killed on exec, which is the one failure the
# reference rewriting can introduce. Skipped when the binary isn't built for
# this machine, since then a failure would mean nothing.
if [ "$(lipo -archs "$BIN" 2>/dev/null || echo unknown)" = "$(uname -m)" ]; then
	if ! "$BIN" --version >/dev/null; then
		echo "bundle-macos-dylibs: binary fails to run after rewriting" >&2
		exit 1
	fi
	echo "→ runs: $("$BIN" --version)"
fi

echo "✓ self-contained: $BIN + $(ls "$LIB_DIR" | wc -l | tr -d ' ') dylibs in lib/"
