#!/usr/bin/env bash
# release.sh — version-bump everything in lockstep + (optionally) tag.
#
# release-please usually drives this from CI on every conventional
# commit, but sometimes you want to ship without waiting (or it
# misses an intra-workspace dep pin and you need to fix it locally).
# This script does what release-please does + the extra Cargo.toml
# intra-workspace bumps it can't, then cargo check'ing the result so
# you don't tag a broken tree.
#
# Usage:
#   ./release.sh patch              # 0.2.6 → 0.2.7  (files only, no commit)
#   ./release.sh minor              # 0.2.6 → 0.3.0
#   ./release.sh major              # 0.2.6 → 1.0.0
#   ./release.sh 0.3.0              # explicit
#   ./release.sh patch --tag        # also: commit, tag vX.Y.Z, push
#                                   #   → kicks off .github/workflows/release.yml
#
# What gets updated:
#   - Cargo.toml workspace version (the line with x-release-please-version)
#   - crates/*/Cargo.toml intra-workspace dep pins
#     (any `pylon-*` dep where version matches the current workspace version)
#   - packages/*/package.json version field
#   - .release-please-manifest.json
#
# Then `cargo check --workspace` runs to make sure the bump didn't
# introduce a version-pin mismatch (the failure mode that produced
# 0.2.4 / 0.2.5 emergency releases historically).

set -euo pipefail

cd "$(dirname "$0")"

usage() {
	cat <<'EOF'
Usage: ./release.sh <bump|version> [--tag]

  bump:    patch | minor | major
  version: explicit semver, e.g. 0.3.0

  --tag:   also commit, tag (vX.Y.Z), and push origin
           (kicks off the GitHub Actions release workflow)

Examples:
  ./release.sh patch
  ./release.sh 0.3.0 --tag
EOF
	exit "${1:-1}"
}

# --- args -----------------------------------------------------------------

bump=""
tag=false
for arg in "$@"; do
	case "$arg" in
		--tag) tag=true ;;
		--help|-h) usage 0 ;;
		*) [[ -z "$bump" ]] && bump="$arg" || usage 1 ;;
	esac
done
[[ -z "$bump" ]] && usage 1

# --- read current version -------------------------------------------------

current="$(grep -E '^version = "[0-9]+\.[0-9]+\.[0-9]+".*x-release-please-version' Cargo.toml \
	| head -1 \
	| sed -E 's/.*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/')"
[[ -z "$current" ]] && {
	echo "error: could not find x-release-please-version line in Cargo.toml" >&2
	exit 1
}

# --- compute target version ----------------------------------------------

if [[ "$bump" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
	target="$bump"
else
	IFS='.' read -r maj min pat <<< "$current"
	case "$bump" in
		patch) target="$maj.$min.$((pat + 1))" ;;
		minor) target="$maj.$((min + 1)).0" ;;
		major) target="$((maj + 1)).0.0" ;;
		*) echo "error: bump must be patch | minor | major | X.Y.Z" >&2; exit 1 ;;
	esac
fi

if [[ "$current" == "$target" ]]; then
	echo "error: target version $target is the same as current — nothing to do" >&2
	exit 1
fi

echo "Bumping $current → $target"

# --- preflight checks (only enforced when --tag is set) -------------------

if $tag; then
	if [[ -n "$(git status --porcelain)" ]]; then
		echo "error: working tree is not clean — commit or stash first" >&2
		git status --short >&2
		exit 1
	fi
	branch="$(git rev-parse --abbrev-ref HEAD)"
	if [[ "$branch" != "main" ]]; then
		echo "warning: you're on '$branch', not 'main'. Continue? [y/N]"
		read -r ans
		[[ "$ans" == "y" || "$ans" == "Y" ]] || exit 1
	fi
	if git rev-parse "v$target" >/dev/null 2>&1; then
		echo "error: tag v$target already exists" >&2
		exit 1
	fi
fi

# --- apply edits ----------------------------------------------------------
#
# perl -pi -e is portable across macOS / Linux; sed -i differs.

# 1. Workspace version (the line release-please owns).
perl -pi -e "s/^version = \"\Q$current\E\"(\s*#\s*x-release-please-version)/version = \"$target\"\$1/" Cargo.toml

# 2. Intra-workspace dep pins. Match ANY pylon-* version in the form
#    `version = "X.Y.Z"`, not just the current workspace version —
#    legacy pins (left over from a previous bump where this script's
#    regex only caught the current value) would otherwise silently
#    stay behind. Caused the v0.3.0 release to fail cargo check
#    because pylon-action still had `pylon-kernel = "^0.2.11"` long
#    after the workspace had moved to 0.2.16.
#
#    Restricted to `pylon-*` so a third-party dep that happens to
#    share a version number is unaffected. Restricted to the
#    `version = "X.Y.Z"` shape so it doesn't touch git/path-only
#    deps without versions.
while IFS= read -r -d '' f; do
	perl -pi -e "s/^(\s*pylon-[a-z_-]+\s*=\s*\{[^}]*version\s*=\s*\")[0-9]+\.[0-9]+\.[0-9]+(\"[^}]*\})/\${1}$target\${2}/" "$f"
done < <(find crates -maxdepth 2 -name Cargo.toml -print0)

# 3. JS packages.
while IFS= read -r -d '' f; do
	perl -pi -e "s/(\"version\"\s*:\s*\")\Q$current\E(\")/\${1}$target\${2}/" "$f"
done < <(find packages -maxdepth 2 -name package.json -print0)

# 3b. @pylonsync/cli's optionalDependencies pin to exact-version
# strings of the platform sub-packages. The dispatcher uses
# require.resolve at runtime, so a range pin (`^x.y.z`) would let
# install resolve to a future version that doesn't ship the binary
# layout the dispatcher expects. Match `"@pylonsync/cli-*": "X.Y.Z"`
# (no caret/tilde) and bump the X.Y.Z piece. Same regex shape as the
# crates pin bump above.
while IFS= read -r -d '' f; do
	perl -pi -e "s/(\"\@pylonsync\/cli-[a-z0-9_-]+\":\s*\")[0-9]+\.[0-9]+\.[0-9]+(\")/\${1}$target\${2}/g" "$f"
done < <(find packages/cli -maxdepth 2 -name package.json -print0)

# 4. release-please manifest.
perl -pi -e "s/(\"\.\"\s*:\s*\")\Q$current\E(\")/\${1}$target\${2}/" .release-please-manifest.json

# --- validate -------------------------------------------------------------
#
# Run the same gates CI runs, in the same order CI runs them. Catching
# them locally is cheaper than burning a tag + release-workflow run on
# a formatting nit (which has happened — that's why fmt is here).

echo "Running cargo fmt --check…"
if ! cargo fmt --all -- --check; then
	cat >&2 <<'EOF'

error: cargo fmt would change files. Run:
  cargo fmt --all
…then re-run ./release.sh.
EOF
	exit 1
fi

echo "Running cargo check…"
cargo check --workspace --quiet

# --- summary --------------------------------------------------------------

echo
echo "Updated to $target:"
git diff --stat | sed 's/^/  /'

# Refresh bun.lock so the version bumps make it into the lockfile.
#
# We DELETE bun.lock first and reinstall from scratch, NOT just `bun install`.
# Why: `bun install` against an existing lockfile preserves the workspace
# packages' "version" entries from the lockfile, even after we've bumped
# them in package.json. `bun publish` then consults the LOCKFILE — not
# package.json — when rewriting `workspace:*` deps, so the published
# package gets pinned to the stale lockfile version.
#
# Real-world bug we hit on v0.3.0: bun.lock had packages/sdk pinned at
# 0.2.14 (a partial-publish from a prior failed run). After bumping
# package.json to 0.3.0, `bun install` left the lockfile's "0.2.14"
# alone, and `bun publish` of @pylonsync/react@0.3.0 emitted a tarball
# with deps pointing to @pylonsync/sdk@0.2.14 — a version that doesn't
# exist on npm. The graph was unsatisfiable end-to-end.
#
# Removing the lockfile before `bun install` forces a fresh resolution
# that picks up the new workspace versions. The download cost is
# negligible (workspaces are local, third-party deps come from bun's
# global cache).
#
# Skip silently if bun isn't installed locally (CI verifies bun.lock
# is fresh anyway via --frozen-lockfile).
if command -v bun >/dev/null 2>&1; then
	echo "Refreshing bun.lock (clean)…"
	rm -f bun.lock
	bun install --silent || {
		echo "::warning::bun install failed — bun.lock may be out of date." >&2
	}
fi

if ! $tag; then
	cat <<EOF

Done — files updated, tree dirty. Review and commit when happy:
  git diff
  git add -A && git commit -m "chore: release $target"
  git tag -a "v$target" -m "Release v$target" && git push --follow-tags

Note: the -a flag makes the tag annotated. --follow-tags only pushes
annotated tags — a lightweight tag won't make it to the remote and the
release.yml workflow won't fire.
EOF
	exit 0
fi

# --- commit + tag + push --------------------------------------------------

echo
echo "Committing + tagging…"
git add -A
git commit -m "chore: release $target"
# Annotated tag is required for `git push --follow-tags` to actually
# push it. A lightweight tag (`git tag NAME` without -a) is created
# locally but the next `git push --follow-tags` silently leaves it
# behind, the Release workflow never fires, and you wonder why npm
# still has the previous version. Bug we hit on v0.2.14.
git tag -a "v$target" -m "Release v$target"
git push --follow-tags

cat <<EOF

✓ Pushed v$target. CI workflow:
  https://github.com/$(git config --get remote.origin.url | sed -E 's#.*[:/]([^/]+/[^/.]+)(\.git)?$#\1#')/actions
EOF
