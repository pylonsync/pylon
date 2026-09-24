#!/usr/bin/env bash
# smoke-shard-codecs.sh — the TS and Swift shard clients against a real
# MessagePack shard (wire protocol v2, see pylon_realtime::wire).
#
#   tools/smoke-shard-codecs.sh
#
# Builds and starts crates/runtime/examples/shard_codec_server.rs, then runs
# packages/react/src/shard.e2e.test.ts and, when `swift` is on PATH, the
# ShardE2ETests in packages/swift. Each client decodes MessagePack
# snapshots, sends binary inputs, sees the ack advance, and gets a
# rejection frame for an input the shard refuses.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
OUT="$(mktemp -t pylon-shard-codec.XXXXXX)"
SERVER_PID=""
cleanup() {
	[[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
	rm -f "$OUT"
}
trap cleanup EXIT

echo "→ build the example shard server"
cargo build -p pylon-runtime --example shard_codec_server --quiet

echo "→ start it"
"$ROOT/target/debug/examples/shard_codec_server" >"$OUT" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 100); do
	[[ -s "$OUT" ]] && break
	sleep 0.1
done
LINE="$(head -1 "$OUT")"
[[ "$LINE" == \{* ]] || {
	echo "::error::the example server did not start: $LINE" >&2
	exit 1
}
export PYLON_SHARD_E2E="$LINE"

echo "→ TypeScript client"
(cd "$ROOT/packages/react" && bun test src/shard.e2e.test.ts)

if command -v swift >/dev/null 2>&1; then
	echo "→ Swift client"
	(cd "$ROOT/packages/swift" && swift test --filter ShardE2ETests)
else
	echo "→ Swift client: skipped (no swift on PATH)"
fi

echo
echo "✓ shard clients decode MessagePack, send binary inputs, and get acks and rejections"
