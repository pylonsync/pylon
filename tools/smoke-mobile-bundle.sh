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
# Each package's `types` points at dist/, so the overlay below needs a
# fresh build or the scaffold typechecks against src (which needs dev
# types the app does not install).
for pkg in sdk sync realtime react functions; do
	(cd "$ROOT/packages/$pkg" && bun run build >/dev/null)
done

echo "→ scaffold mobile template"
(cd "$TMP" && node "$SCRIPT" smoke-app --template mobile --bun --skip-install --no-skill </dev/null >"$TMP/scaffold.log" 2>&1) || {
	cat "$TMP/scaffold.log" >&2
	exit 1
}
APP="$TMP/smoke-app"

# The scaffold pins @pylonsync/* to this checkout's version, which is not
# on npm yet when CI runs on a release commit. Install whatever is
# published; the local packages replace it below.
echo "→ pin @pylonsync/* to the published release"
for pkg in "$APP/apps/expo/package.json" "$APP/apps/api/package.json"; do
	node -e '
const fs = require("node:fs");
const file = process.argv[1];
const pkg = JSON.parse(fs.readFileSync(file, "utf8"));
for (const section of ["dependencies", "devDependencies"]) {
	for (const name of Object.keys(pkg[section] ?? {})) {
		if (name.startsWith("@pylonsync/")) pkg[section][name] = "latest";
	}
}
fs.writeFileSync(file, JSON.stringify(pkg, null, 2) + "\n");
' "$pkg"
done

echo "→ bun install"
(cd "$APP" && bun install >"$TMP/install.log" 2>&1) || {
	tail -50 "$TMP/install.log" >&2
	exit 1
}

# Dependencies the local packages added since the last release. Installed
# before the overlay, since an install relinks @pylonsync/* from the store.
for app in apps/expo apps/api; do
	extra_deps="$(node "$ROOT/tools/missing-local-deps.mjs" "$APP/$app" \
		"$ROOT/packages/react" "$ROOT/packages/react-native" "$ROOT/packages/sdk" \
		"$ROOT/packages/sync" "$ROOT/packages/functions" "$ROOT/packages/realtime")"
	if [[ -n "$extra_deps" ]]; then
		echo "→ $app: add dependencies of the local packages: $(echo $extra_deps)"
		# shellcheck disable=SC2086 # one argument per line of output
		(cd "$APP/$app" && bun add $extra_deps >>"$TMP/install.log" 2>&1) || {
			tail -50 "$TMP/install.log" >&2
			exit 1
		}
	fi
done

# Link the local package's dependencies next to its real (store) directory,
# where module resolution from the package finds them. Only the ones missing
# there and installed in the app: the dependencies added above.
link_extra_deps() {
	local app_nm="$1" real="$2" src="$3" name target
	for name in $(node -e 'for (const d of Object.keys(require(process.argv[1]).dependencies ?? {})) console.log(d)' "$src/package.json"); do
		target="$(dirname "$(dirname "$real")")/$name"
		[[ -e "$target" || ! -e "$app_nm/$name" ]] && continue
		mkdir -p "$(dirname "$target")"
		ln -s "$(realpath "$app_nm/$name")" "$target"
	done
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
	link_extra_deps "$APP/$app/node_modules" "$real" "$src"
}
# A local @pylonsync package the overlaid packages depend on but npm does
# not have yet (added since the last release): copy it into the app.
vendor_local() {
	local app_nm="$1" name="$2" src="$3"
	[[ -e "$app_nm/@pylonsync/$name" ]] && return 0
	mkdir -p "$app_nm/@pylonsync/$name"
	cp -R "$src/src" "$src/package.json" "$app_nm/@pylonsync/$name/"
	if [[ -d "$src/dist" ]]; then cp -R "$src/dist" "$app_nm/@pylonsync/$name/"; fi
}

echo "→ overlay local @pylonsync packages"
for app in apps/expo apps/api; do
	vendor_local "$APP/$app/node_modules" realtime "$ROOT/packages/realtime"
	overlay "$app" realtime "$ROOT/packages/realtime"
	overlay "$app" react "$ROOT/packages/react"
	overlay "$app" react-native "$ROOT/packages/react-native"
	overlay "$app" sdk "$ROOT/packages/sdk"
	overlay "$app" sync "$ROOT/packages/sync"
	overlay "$app" functions "$ROOT/packages/functions"
	overlay "$app" revenuecat "$ROOT/packages/plugins/revenuecat"
	# apps/api imports @pylonsync/stripe/entitlement, a subpath the published
	# plugin gains with the next release; the local package has it now.
	overlay "$app" stripe "$ROOT/packages/plugins/stripe"
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
