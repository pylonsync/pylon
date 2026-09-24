#!/usr/bin/env bash
# smoke-production-build.sh — `pylon build` writes an artifact that
# `pylon start <dir>` runs with no source tree and no node_modules.
#
#   tools/smoke-production-build.sh [path/to/pylon]   (default: target/debug/pylon)
#   PYLON_SMOKE_KEEP=1 keeps the temp directory for debugging.
#
# What it verifies, on the barebones template with the packages in THIS
# checkout:
#   1. `pylon build` succeeds with a browser target, usage polyfills, an
#      external server package, and an `include` directory.
#   2. The artifact, copied to a directory with no node_modules above it,
#      serves the SSR page with its polyfill and entry scripts, every
#      /_pylon/build/ asset it references, the metadata routes, a route
#      handler, an OG image, and a server function that uses the external
#      package and reads the included file.
#   3. The client JS has no optional chaining or `??` left (the target
#      lowers both).
#   4. The server writes nothing inside the artifact: data goes next to it.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
PYLON="${1:-$ROOT/target/debug/pylon}"
[[ -x "$PYLON" || -x "$PYLON.exe" ]] || {
	echo "::error::no pylon binary at $PYLON (cargo build -p pylon-cli first)" >&2
	exit 1
}
# Absolute, because the steps below run from other directories.
PYLON="$(cd "$(dirname "$PYLON")" && pwd)/$(basename "$PYLON")"
PORT="${PYLON_SMOKE_PORT:-4791}"
TMP="$(mktemp -d -t pylon-prod-smoke.XXXXXX)"
SERVER_PID=""
cleanup() {
	[[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
	if [[ -n "${PYLON_SMOKE_KEEP:-}" ]]; then
		echo "kept $TMP" >&2
		return
	fi
	find "$TMP" -mindepth 1 -delete 2>/dev/null || true
	rmdir "$TMP" 2>/dev/null || true
}
trap cleanup EXIT

fail() {
	echo "::error::$1" >&2
	[[ -f "$TMP/server.log" ]] && tail -40 "$TMP/server.log" >&2
	exit 1
}

echo "→ scaffold barebones"
(cd "$TMP" && node "$ROOT/packages/create-pylon/bin/create-pylon.js" app --template barebones --bun --skip-install --no-skill </dev/null >"$TMP/scaffold.log" 2>&1) || {
	cat "$TMP/scaffold.log" >&2
	exit 1
}
APP="$TMP/app"

# The scaffold pins @pylonsync/* to this checkout's version, which may not be
# on npm yet. Install what is published; the local packages replace it below.
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
' "$APP/package.json"

echo "→ add the fixture: build settings, a function, an OG image, a route handler"
mkdir -p "$APP/content" "$APP/app/feed"
echo "hello from content" >"$APP/content/hello.txt"
cat >"$APP/functions/greet.ts" <<'EOF'
import { query } from "@pylonsync/functions";
import ms from "ms";
import { readFileSync } from "node:fs";

export default query({
  auth: "public",
  args: {},
  handler: async () => ({
    duration: ms(90_000),
    content: readFileSync("content/hello.txt", "utf8").trim(),
  }),
});
EOF
cat >"$APP/app/opengraph-image.tsx" <<'EOF'
import React from "react";
import { ImageResponse } from "@pylonsync/react";

export default function OG() {
  return new ImageResponse(
    <div style={{ display: "flex", width: "100%", height: "100%", fontSize: 64 }}>Smoke</div>,
    { width: 600, height: 315 },
  );
}
EOF
cat >"$APP/app/feed/route.ts" <<'EOF'
export const GET = async () => ({ body: "feed ok", contentType: "text/plain; charset=utf-8" });
EOF
node -e '
const fs = require("node:fs");
const file = process.argv[1];
const src = fs.readFileSync(file, "utf8");
const block = `  routes: await discoverAppRoutes(),
  build: {
    target: ["safari >= 12", "chrome >= 70", "firefox >= 68"],
    polyfill: "usage",
    server: { external: ["ms"] },
    include: ["content"],
  },`;
if (!src.includes("  routes: await discoverAppRoutes(),")) throw new Error("app.ts shape changed");
fs.writeFileSync(file, src.replace("  routes: await discoverAppRoutes(),", block));
' "$APP/app.ts"

echo "→ bun install"
# Right after a release, `latest` names a version the registry CDN may not
# serve yet, and bun caches the stale package index it fetched. Retry with a
# fresh cache each time.
installed=""
for attempt in 1 2 3 4 5; do
	export BUN_INSTALL_CACHE_DIR="$TMP/bun-cache-$attempt"
	if (cd "$APP" && bun add ms core-js >"$TMP/install.log" 2>&1 &&
		bun add -d @swc/core browserslist lightningcss >>"$TMP/install.log" 2>&1); then
		installed=1
		break
	fi
	echo "  install attempt $attempt failed; retrying in 30s" >&2
	tail -3 "$TMP/install.log" >&2
	sleep 30
done
[[ -n "$installed" ]] || {
	tail -50 "$TMP/install.log" >&2
	exit 1
}

# `pylon build` installs dependencies first when its install marker is stale,
# which would replace the overlay below. A codegen-only build now brings the
# marker up to date.
(cd "$APP" && "$PYLON" build --no-bundle >"$TMP/codegen.log" 2>&1) || {
	cat "$TMP/codegen.log" >&2
	exit 1
}

# Copy a local package over the installed one (following Bun's store link).
overlay() {
	local name="$1" src="$2"
	local link="$APP/node_modules/@pylonsync/$name"
	[[ -e "$link" ]] || return 0
	local real
	real="$(cd "$link" && pwd -P)"
	cp -R "$src/src/." "$real/src/"
	[[ -d "$src/assets" ]] && cp -R "$src/assets/." "$real/assets/"
	cp "$src/package.json" "$real/package.json"
}
echo "→ overlay local @pylonsync packages"
for name in functions sdk react client sync; do
	overlay "$name" "$ROOT/packages/$name"
done

echo "→ pylon build"
(cd "$APP" && "$PYLON" build >"$TMP/build.log" 2>&1) || {
	cat "$TMP/build.log" >&2
	exit 1
}
[[ -f "$APP/dist/pylon-build.json" ]] || fail "no dist/pylon-build.json"

echo "→ lowered client JS"
while IFS= read -r f; do
	case "$f" in *polyfills-*) continue ;; esac
	if grep -q '?\.' "$f" || grep -q '??' "$f"; then
		fail "$f still has ?. or ?? after lowering"
	fi
done < <(find "$APP/dist/client" -name '*.js')

echo "→ pylon start <dir> with no node_modules above it"
mkdir -p "$TMP/run"
cp -R "$APP/dist" "$TMP/run/dist"
find "$TMP/run/dist" -type f | sort >"$TMP/files.before"
(cd "$TMP/run" && PYLON_DEV_MODE=true exec "$PYLON" start dist --port "$PORT" >"$TMP/server.log" 2>&1) &
SERVER_PID=$!
for _ in $(seq 1 60); do
	curl -fsS "http://localhost:$PORT/health" >/dev/null 2>&1 && break
	sleep 1
done
curl -fsS "http://localhost:$PORT/health" >/dev/null || fail "server did not become healthy"

BASE="http://localhost:$PORT"
HTML="$(curl -fsS "$BASE/")" || fail "GET / failed"
echo "$HTML" | grep -q '__PYLON_DATA__' || fail "GET / has no hydration payload"
echo "$HTML" | grep -q 'src="/_pylon/build/polyfills-' || fail "GET / does not load the polyfill bundle"
for asset in $(echo "$HTML" | grep -o '/_pylon/build/[^"]*' | sort -u); do
	code="$(curl -s -o /dev/null -w '%{http_code}' "$BASE$asset")"
	[[ "$code" == "200" ]] || fail "$asset returned $code"
done

check() {
	local path="$1" want="$2"
	local body
	body="$(curl -fsS "$BASE$path")" || fail "GET $path failed"
	echo "$body" | grep -q "$want" || fail "GET $path did not contain '$want'"
}
check /robots.txt "User-agent"
check /sitemap.xml "<urlset"
check /feed "feed ok"
ctype="$(curl -s -o /dev/null -w '%{content_type}' "$BASE/opengraph-image")"
[[ "$ctype" == image/png* ]] || fail "/opengraph-image returned $ctype"
FN="$(curl -fsS -X POST "$BASE/api/fn/greet" -H 'content-type: application/json' -d '{}')" || fail "POST /api/fn/greet failed"
echo "$FN" | grep -q '"duration":"2m"' || fail "greet did not use the external package: $FN"
echo "$FN" | grep -q 'hello from content' || fail "greet did not read the included file: $FN"

kill "$SERVER_PID"
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""
find "$TMP/run/dist" -type f | sort >"$TMP/files.after"
diff "$TMP/files.before" "$TMP/files.after" >/dev/null || {
	diff "$TMP/files.before" "$TMP/files.after" >&2 || true
	fail "the server wrote files inside the artifact"
}
[[ -f "$TMP/run/pylon.db" ]] || fail "no pylon.db next to the artifact"

echo
echo "✓ pylon build artifact runs with no source tree or node_modules"
