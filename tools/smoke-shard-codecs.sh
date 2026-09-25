#!/usr/bin/env bash
# smoke-shard-codecs.sh — the TS and Swift shard clients against real
# shards (wire protocol v2, see pylon_realtime::wire).
#
#   tools/smoke-shard-codecs.sh
#
# Starts two example servers:
# - crates/runtime/examples/shard_codec_server.rs: a MessagePack snapshot
#   shard. Each client decodes snapshots, sends binary inputs, sees the ack
#   advance, and gets a rejection frame for an input the shard refuses.
# - crates/runtime/examples/shard_replication_server.rs: an entity
#   replication shard. Each client's entity table follows spawns, moves,
#   component changes, and despawns, and never holds the stealthed unit.
#
# Then runs packages/react's shard.e2e.test.ts and
# shard-replication.e2e.test.ts, and, when `swift` is on PATH, the
# ShardE2ETests in packages/swift.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
TMP="$(mktemp -d -t pylon-shard-e2e.XXXXXX)"
PIDS=()
cleanup() {
	for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
	rm -rf "$TMP"
}
trap cleanup EXIT

echo "→ build the example shard servers"
cargo build -p pylon-runtime --example shard_codec_server --example shard_replication_server --quiet

# Start an example server and print its ready line.
start() {
	local name="$1" out="$TMP/$1.out"
	"$ROOT/target/debug/examples/$name" >"$out" 2>&1 &
	PIDS+=($!)
	for _ in $(seq 1 100); do
		[[ -s "$out" ]] && break
		sleep 0.1
	done
	local line
	line="$(head -1 "$out")"
	[[ "$line" == \{* ]] || {
		echo "::error::$name did not start: $line" >&2
		exit 1
	}
	echo "$line"
}

echo "→ start them"
PYLON_SHARD_E2E="$(start shard_codec_server)"
PYLON_SHARD_REPLICATION_E2E="$(start shard_replication_server)"
export PYLON_SHARD_E2E PYLON_SHARD_REPLICATION_E2E

echo "→ TypeScript client"
(cd "$ROOT/packages/react" && bun test src/shard.e2e.test.ts src/shard-replication.e2e.test.ts)

if command -v swift >/dev/null 2>&1; then
	echo "→ Swift client"
	(cd "$ROOT/packages/swift" && swift test --filter ShardE2ETests)
else
	echo "→ Swift client: skipped (no swift on PATH)"
fi

echo
echo "✓ shard clients decode MessagePack, send binary inputs, get acks and rejections, and keep a replicated entity table"
