#!/usr/bin/env bash
# smoke-mobile-bundle.sh — the `mobile` template must bundle for iOS and
# Android with the packages in THIS checkout.
#
# What it verifies:
#   1. `npm create` scaffolds the mobile template.
#   2. `expo install --check` accepts the template's dependency set for
#      its Expo SDK (missing peers, wrong TypeScript range, ...).
#   3. Both apps typecheck against the local @pylonsync packages.
#   4. `expo export` produces an iOS and an Android bundle. Metro resolves
#      every import at build time, so one Node-only module anywhere in the
#      client import graph fails here (the 0.8.3 `defineRoute` re-export
#      pulled the sdk's filesystem discovery into every native build).
#
# The scaffold installs the published @pylonsync packages, then the local
# packages' src/, dist/, and package.json are copied over them, so the
# check runs against the code about to be released.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
SCRIPT="$ROOT/packages/create-pylon/bin/create-pylon.js"
TMP="$(mktemp -d -t pylon-mobile-smoke.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

echo "→ build local package types"
for pkg in sdk sync react; do
	(cd "$ROOT/packages/$pkg" && bun run build >/dev/null)
done

echo "→ scaffold mobile template"
(cd "$TMP" && node "$SCRIPT" smoke-app --template mobile --bun --skip-install --no-skill </dev/null >"$TMP/scaffold.log" 2>&1) || {
	cat "$TMP/scaffold.log" >&2
	exit 1
}
APP="$TMP/smoke-app"

echo "→ bun install"
(cd "$APP" && bun install >"$TMP/install.log" 2>&1) || {
	tail -50 "$TMP/install.log" >&2
	exit 1
}

# Copy a local package over the installed one. Bun links workspace apps
# to a shared store, so follow the symlink to the real directory.
overlay() {
	local app="$1" name="$2" src="$3"
	local link="$APP/$app/node_modules/@pylonsync/$name"
	[[ -e "$link" ]] || return 0
	local real
	real="$(realpath "$link")"
	rm -rf "$real/src" "$real/dist"
	cp -R "$src/src" "$real/src"
	[[ -d "$src/dist" ]] && cp -R "$src/dist" "$real/dist"
	cp "$src/package.json" "$real/package.json"
}
echo "→ overlay local @pylonsync packages"
for app in apps/expo apps/api; do
	overlay "$app" react "$ROOT/packages/react"
	overlay "$app" react-native "$ROOT/packages/react-native"
	overlay "$app" sdk "$ROOT/packages/sdk"
	overlay "$app" sync "$ROOT/packages/sync"
	overlay "$app" functions "$ROOT/packages/functions"
	overlay "$app" revenuecat "$ROOT/packages/plugins/revenuecat"
done

echo "→ expo install --check"
(cd "$APP/apps/expo" && bunx expo install --check)

echo "→ typecheck apps/expo and apps/api"
(cd "$APP/apps/expo" && bun run check)
(cd "$APP/apps/api" && bun run check)

echo "→ expo export (ios + android)"
(cd "$APP/apps/expo" && CI=1 bunx expo export --platform ios --platform android --output-dir .expo-export >"$TMP/export.log" 2>&1) || {
	tail -60 "$TMP/export.log" >&2
	exit 1
}
for platform in ios android; do
	if ! ls "$APP/apps/expo/.expo-export/_expo/static/js/$platform/"*.hbc >/dev/null 2>&1; then
		echo "::error::no $platform bundle in the export" >&2
		tail -60 "$TMP/export.log" >&2
		exit 1
	fi
done

echo
echo "✓ mobile template bundles for iOS and Android with the local packages"
