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
#   ./release.sh patch --tag --allow-red-ci "hotfix: <why>"
#                                   # tag even though ci.yml is not green for
#                                   # HEAD; the reason lands in the commit
#
# `--tag` refuses to run unless:
#   - the working tree is clean (another session's in-flight files would
#     otherwise be swept into the release commit — that published a
#     half-written module under 0.4.28),
#   - ci.yml has completed with success for HEAD (release.yml checks this
#     again server-side; the local check fails faster).
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
           Requires a clean tree and a green ci.yml run for HEAD.

  --allow-red-ci "<reason>":
           tag even though ci.yml is not green for HEAD. The reason is
           recorded in the release commit. release.yml still waits for
           ci.yml on the pushed commit, so this only skips the local check.

Examples:
  ./release.sh patch
  ./release.sh 0.3.0 --tag
  ./release.sh patch --tag --allow-red-ci "hotfix: auth outage, flaky swift job"
EOF
	exit "${1:-1}"
}

# --- args -----------------------------------------------------------------

bump=""
tag=false
allow_red_ci=""
expect_reason=false
for arg in "$@"; do
	if $expect_reason; then
		allow_red_ci="$arg"
		expect_reason=false
		continue
	fi
	case "$arg" in
		--tag) tag=true ;;
		--allow-red-ci) expect_reason=true ;;
		--help|-h) usage 0 ;;
		*) [[ -z "$bump" ]] && bump="$arg" || usage 1 ;;
	esac
done
[[ -z "$bump" ]] && usage 1
if $expect_reason; then
	echo "error: --allow-red-ci needs a reason argument" >&2
	usage 1
fi

# --- read current version -------------------------------------------------

current="$(grep -E '^version = "[0-9]+\.[0-9]+\.[0-9]+".*x-release-please-version' Cargo.toml \
	| head -1 \
	| sed -E 's/.*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/')"
[[ -z "$current" ]] && {
	echo "error: could not find x-release-please-version line in Cargo.toml" >&2
	exit 1
}

# Re-run safety: if a previous invocation crashed mid-flight, the JS
# packages will already carry the new version while Cargo.toml still
# carries the OLD one (or vice versa). Either way, picking up where
# we left off by computing target=$current+1 produces a version that's
# *already in some files*. Detect the mismatch and refuse rather than
# silently double-bumping.
js_current=""
for pkg in packages/sdk/package.json packages/functions/package.json packages/cli/package.json; do
	if [[ -f "$pkg" ]]; then
		v="$(grep -E '"version"\s*:\s*"[0-9]+\.[0-9]+\.[0-9]+"' "$pkg" | head -1 \
			| sed -E 's/.*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/')"
		js_current="$v"
		break
	fi
done
if [[ -n "$js_current" && "$js_current" != "$current" ]]; then
	echo "error: version drift detected — Cargo.toml=$current but JS packages=$js_current." >&2
	echo "       A previous release.sh run probably crashed partway through." >&2
	echo "       Reconcile both files to the same version (git checkout, manual edit)" >&2
	echo "       before re-running, or pass an explicit X.Y.Z target." >&2
	exit 1
fi

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
	echo "Reconciling all workspace versions at $target"
else
	echo "Bumping $current → $target"
fi

# --- preflight checks (only enforced when --tag is set) -------------------

# The files this script edits and stages. The commit is built from this
# list, never from `git add -A`: other sessions work in this tree at the
# same time and their half-written files must not ride along.
release_files() {
	echo Cargo.toml Cargo.lock bun.lock .release-please-manifest.json
	find crates -maxdepth 2 -name Cargo.toml
	find packages -maxdepth 3 -name package.json -not -path '*/node_modules/*'
}

# Is ci.yml green for the commit we are about to release on top of?
# Prints the run URL on stdout; returns non-zero when the run is missing,
# still running, or failed.
ci_state_for_head() {
	local sha run
	sha="$(git rev-parse HEAD)"
	if ! command -v gh >/dev/null 2>&1; then
		echo "gh CLI not installed" >&2
		return 2
	fi
	run="$(gh run list --workflow ci.yml --commit "$sha" \
		--json status,conclusion,url --limit 1 --jq '.[0] // empty' 2>/dev/null)" || {
		echo "gh run list failed (not logged in?)" >&2
		return 2
	}
	if [[ -z "$run" ]]; then
		echo "no ci.yml run for $sha (push HEAD first, or wait for it to start)" >&2
		return 1
	fi
	local status conclusion url
	status="$(jq -r .status <<<"$run")"
	conclusion="$(jq -r .conclusion <<<"$run")"
	url="$(jq -r .url <<<"$run")"
	echo "$url"
	if [[ "$status" != "completed" ]]; then
		echo "ci.yml is still $status for $sha" >&2
		return 1
	fi
	if [[ "$conclusion" != "success" ]]; then
		echo "ci.yml concluded '$conclusion' for $sha" >&2
		return 1
	fi
	return 0
}

if $tag; then
	if [[ -n "$(git status --porcelain)" ]]; then
		echo "error: working tree is not clean." >&2
		echo "       Commit your own work first. If these files belong to another" >&2
		echo "       session, do not stash or commit them — wait for that session." >&2
		git status --short >&2
		exit 1
	fi
	if [[ -n "$(git log --oneline "@{upstream}..HEAD" 2>/dev/null)" ]]; then
		echo "error: HEAD has commits not on the remote; push first so ci.yml can run on them." >&2
		exit 1
	fi
	echo "Checking ci.yml for HEAD…"
	if ci_url="$(ci_state_for_head)"; then
		echo "  ci.yml green: $ci_url"
	elif [[ -n "$allow_red_ci" ]]; then
		echo "warning: ci.yml is NOT green for HEAD; continuing because --allow-red-ci was given:" >&2
		echo "         $allow_red_ci" >&2
	else
		echo "error: refusing to tag on a commit without a green ci.yml run." >&2
		echo "       Wait for CI, fix it, or pass --allow-red-ci \"<reason>\" for an emergency." >&2
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

# 3. JS packages. Depth 3 so nested `packages/plugins/*/package.json`
# get bumped too — earlier `maxdepth 2` silently missed every plugin
# package (stripe, feature-flags, webhooks) and the publish workflow
# would then refuse to start because of version drift across the
# workspace. Real-world bug: v0.3.84 cut after v0.3.83 shipped with
# the plugin packages stranded at 0.3.83.
while IFS= read -r -d '' f; do
	[[ "$f" == *node_modules* ]] && continue
	perl -pi -e "s/(\"version\"\s*:\s*\")[0-9]+\.[0-9]+\.[0-9]+(\")/\${1}$target\${2}/" "$f"
done < <(find packages -maxdepth 3 -name package.json -print0)

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

# 4. release-please manifest. Bumps the `"." : "X.Y.Z"` line REGARDLESS
# of what its current value is — matching against $current would silently
# no-op when the manifest had drifted (it sat at 0.2.11 through the
# 0.3.x line of releases because of this exact bug; the published
# packages and the manifest disagreed for months). Now: any well-formed
# semver in that slot gets rewritten to $target.
perl -pi -e "s/(\"\.\"\s*:\s*\")[0-9]+\.[0-9]+\.[0-9]+(\")/\${1}$target\${2}/" .release-please-manifest.json

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
# The bun that regenerates this lockfile MUST match the bun CI installs
# (.github/workflows/*.yml `bun-version`). bun 1.3 writes a "configVersion"
# field that 1.2 doesn't understand, so a lockfile written here by a newer bun
# resolved differently under CI's older one — @types/react stopped being
# visible to examples/acme and `bun run check` failed. Because only a release
# rewrites the lockfile, CI went red on every `chore: release` commit while
# staying green on feature commits, for five releases running.
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

# Refresh Cargo.lock so the workspace lockfile matches the just-bumped
# crate versions. Without this, CI's `cargo build --locked` rejects the
# release with "cannot update the lock file because --locked was
# passed" and the binary build (which feeds @pylonsync/cli) fails
# silently while the npm packages still ship. v0.3.91 hit this: 10/11
# packages shipped, the CLI dispatcher stayed at the previous version.
#
# `cargo update -p pylon-cli --workspace` walks the whole intra-
# workspace dependency graph from the CLI crate, so every pylon-*
# entry in Cargo.lock gets bumped to $target in one pass. Faster
# than `cargo build` and doesn't require a successful compile.
if command -v cargo >/dev/null 2>&1; then
	echo "Refreshing Cargo.lock…"
	cargo update -p pylon-cli --workspace --quiet || {
		echo "::warning::cargo update failed — Cargo.lock may be out of date and the CI binary build will fail." >&2
	}
fi

# Refresh the locally-installed pylon binary so downstream projects
# (yapless, internal tooling) that invoke `pylon` from ~/.cargo/bin
# pick up the just-bumped version without waiting for the CI publish
# to upload tarballs and `bun install` to pull the new prebuilt.
#
# Why this matters: we hit this on v0.3.87 (provider-routing fix) and
# v0.3.88 (Stack0 /v1 path fix) — yapless was running the binary
# straight from ~/.cargo/bin (an older cargo-installed copy) instead
# of going through node_modules/.bin/pylon, so my "shipped" fixes
# were invisible to the dev environment until I manually rebuilt.
# Embedding this here means a future me can't forget.
#
# Skip if cargo isn't on PATH (CI runners don't need a local
# binary). `--offline` keeps the install fast — workspace deps
# are already vendored, no need to refetch.
if command -v cargo >/dev/null 2>&1; then
	echo "Installing pylon $target to ~/.cargo/bin…"
	cargo install --path crates/cli --offline --quiet || {
		echo "::warning::cargo install failed — ~/.cargo/bin/pylon may be stale." >&2
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
# Stage only the release files. Anything else in the tree was created after
# the preflight by another session; leave it alone and say so.
release_files | xargs git add --
stray="$(git status --porcelain | grep -v '^[MARC]  ' || true)"
if [[ -n "$stray" ]]; then
	echo "error: files outside the release set changed while this script ran:" >&2
	echo "$stray" >&2
	echo "       Not committing them. Re-run once the tree holds only your work." >&2
	git reset --quiet
	exit 1
fi
commit_msg="chore: release $target"
if [[ -n "$allow_red_ci" ]]; then
	commit_msg="$commit_msg

Released with --allow-red-ci: $allow_red_ci"
fi
git commit -m "$commit_msg"
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
