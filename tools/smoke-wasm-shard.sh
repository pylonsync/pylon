#!/usr/bin/env bash
# smoke-wasm-shard.sh — examples/shard-arena end to end: shard logic in Rust,
# compiled to WebAssembly, run by the stock pylon binary.
#
#   tools/smoke-wasm-shard.sh [path/to/pylon]
#
# 1. `pylon shards build` compiles the example's crate to shards/arena.wasm.
# 2. `pylon start app.ts` boots from source; packages/react's
#    shard-wasm.e2e.test.ts joins as two guests over /shard on the main port,
#    moves, gets a rejection, and has a stolen ticket refused.
# 3. `pylon bench shard` runs 50 bots through the app's join function and
#    must connect all of them and see their inputs acked.
# 4. `pylon build` writes an artifact with the module in it; `pylon start
#    <dir>` boots it and the same test runs again.
#
# Needs Rust with the wasm32-unknown-unknown target, and `bun install` at the
# repo root. PYLON_SMOKE_KEEP=1 keeps the temp directory.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
PYLON="${1:-$ROOT/target/debug/pylon}"
[[ -x "$PYLON" || -x "$PYLON.exe" ]] || {
	echo "::error::no pylon binary at $PYLON (cargo build -p pylon-cli first)" >&2
	exit 1
}
PYLON="$(cd "$(dirname "$PYLON")" && pwd)/$(basename "$PYLON")"
APP="$ROOT/examples/shard-arena"
PORT="${PYLON_SMOKE_PORT:-4793}"
TMP="$(mktemp -d -t pylon-wasm-shard.XXXXXX)"
SERVER_PID=""
cleanup() {
	[[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
	# Keep the committed module: a local toolchain's build differs byte for byte.
	[[ -f "$TMP/arena.wasm.committed" ]] && cp "$TMP/arena.wasm.committed" "$APP/shards/arena.wasm"
	if [[ -n "${PYLON_SMOKE_KEEP:-}" ]]; then
		echo "kept $TMP"
	else
		rm -rf "$TMP"
	fi
}
trap cleanup EXIT

# Start `pylon start <target>` in <dir> and wait for /health.
serve() {
	local dir="$1" target="$2" log="$3"
	(cd "$dir" && PYLON_DB_PATH="$TMP/$log.db" PYLON_CORS_ORIGIN="http://localhost:$PORT" \
		PYLON_SHARD_WS_MAX_PER_IP=0 \
		exec "$PYLON" start "$target" --port "$PORT") >"$TMP/$log.log" 2>&1 &
	SERVER_PID=$!
	for _ in $(seq 1 120); do
		if curl -sf "http://localhost:$PORT/health" >/dev/null; then
			return 0
		fi
		if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
		sleep 0.5
	done
	cat "$TMP/$log.log" >&2
	echo "::error::pylon start $target did not come up" >&2
	exit 1
}

stop() {
	kill "$SERVER_PID" 2>/dev/null || true
	wait "$SERVER_PID" 2>/dev/null || true
	SERVER_PID=""
}

e2e() {
	(cd "$ROOT/packages/react" && PYLON_WASM_SHARD_E2E="localhost:$PORT" bun test src/shard-wasm.e2e.test.ts)
}

echo "→ pylon shards build"
cp "$APP/shards/arena.wasm" "$TMP/arena.wasm.committed"
(cd "$APP" && "$PYLON" shards build)
[[ -s "$APP/shards/arena.wasm" ]] || {
	echo "::error::pylon shards build wrote no shards/arena.wasm" >&2
	exit 1
}

echo "→ pylon start app.ts"
serve "$APP" app.ts source
grep -q "compiled 1 kind(s)" "$TMP/source.log" || {
	cat "$TMP/source.log" >&2
	echo "::error::the server did not compile the shard module" >&2
	exit 1
}
e2e

echo "→ pylon bench shard (50 bots)"
(cd "$APP" && "$PYLON" bench shard --url "http://localhost:$PORT" --join joinArena \
	--bots 50 --duration 5 --ramp 1 --rate 10 --input '"join"' \
	--input '{"move_to":{"x":"$rand:0:800","y":"$rand:0:500"}}' --json) >"$TMP/bench.json" || {
	cat "$TMP/bench.json" >&2
	exit 1
}
node -e '
const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
const fail = (m) => { console.error("::error::" + m + "\n" + JSON.stringify(r, null, 2)); process.exit(1); };
if (r.connected !== 50) fail(`${r.connected} of 50 bots connected`);
if (r.dropped !== 0) fail(`${r.dropped} bots dropped before the end`);
if (r.bots_acked !== 50) fail(`only ${r.bots_acked} of 50 bots got an ack`);
if (r.decode_errors !== 0) fail(`${r.decode_errors} frames did not decode`);
if (!(r.tick_rate_hz.p50 > 15)) fail(`bots saw ${r.tick_rate_hz.p50} Hz`);
console.log(`  50 bots: ${r.tick_rate_hz.p50} Hz, ack p99 ${r.ack_latency_ms.p99} ms`);
' "$TMP/bench.json"
stop

echo "→ pylon build"
(cd "$APP" && "$PYLON" build --out "$TMP/dist" >"$TMP/build.log" 2>&1) || {
	cat "$TMP/build.log" >&2
	exit 1
}
[[ -s "$TMP/dist/shards/arena.wasm" ]] || {
	echo "::error::the artifact has no shards/arena.wasm" >&2
	exit 1
}

echo "→ pylon start <artifact>"
serve "$TMP" "$TMP/dist" artifact
e2e
stop

echo
echo "✓ a WebAssembly shard runs from source and from a pylon build artifact"
