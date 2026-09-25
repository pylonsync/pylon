#!/usr/bin/env bash
# smoke-shard-cluster.sh — shards across machines, with two real pylon
# processes on one Postgres running examples/shard-arena.
#
#   tools/smoke-shard-cluster.sh [path/to/pylon]
#
# packages/realtime's shard-cluster.e2e.test.ts:
#   1. creates an arena through machine A, pinned to machine B;
#   2. connects to A, which proxies the WebSocket to B; moves a player;
#   3. kills B; A starts the arena from B's saved state, the client
#      reconnects, and its player is where it was.
# Before that, a machine C with FLY_MACHINE_ID set answers a request for a
# shard on A with `fly-replay: instance=a` (Fly's proxy replays it there).
#
# Needs Postgres (createdb/psql on PATH, or PYLON_SMOKE_PG_URL pointing at
# an empty database), Rust with the wasm32-unknown-unknown target, and
# `bun install` at the repo root. PYLON_SMOKE_KEEP=1 keeps the logs.

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
# Each machine also takes port+1 (sync WS), +2 (SSE), and +3 (shard port).
PORT_A="${PYLON_SMOKE_PORT_A:-4851}"
PORT_B="${PYLON_SMOKE_PORT_B:-4861}"
PORT_C="${PYLON_SMOKE_PORT_C:-4871}"
TMP="$(mktemp -d -t pylon-shard-cluster.XXXXXX)"
PID_A=""
PID_B=""
PID_C=""
DB_NAME="pylon_shard_cluster_smoke"

cleanup() {
	# Never kill an unset pid: `kill 0` signals the whole process group.
	[[ -n "$PID_A" ]] && kill "$PID_A" 2>/dev/null || true
	[[ -n "$PID_B" ]] && kill "$PID_B" 2>/dev/null || true
	[[ -n "$PID_C" ]] && kill "$PID_C" 2>/dev/null || true
	wait 2>/dev/null || true
	if [[ -n "${PYLON_SMOKE_KEEP:-}" ]]; then
		echo "kept $TMP"
	else
		rm -rf "$TMP"
	fi
}
trap cleanup EXIT

if [[ -n "${PYLON_SMOKE_PG_URL:-}" ]]; then
	DB_URL="$PYLON_SMOKE_PG_URL"
else
	psql -d postgres -qc "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname='$DB_NAME' AND pid <> pg_backend_pid();" >/dev/null 2>&1 || true
	dropdb --if-exists "$DB_NAME"
	createdb "$DB_NAME"
	DB_URL="postgres://$(whoami)@localhost:5432/$DB_NAME"
fi

echo "→ pylon shards build"
cp "$APP/shards/arena.wasm" "$TMP/arena.wasm.committed"
(cd "$APP" && "$PYLON" shards build) >"$TMP/build.log" 2>&1 || {
	cat "$TMP/build.log" >&2
	exit 1
}
restore_module() { cp "$TMP/arena.wasm.committed" "$APP/shards/arena.wasm"; }
trap 'restore_module; cleanup' EXIT

# start <name> <port> [VAR=value...]: machine <name> on <port>.
start() {
	local name="$1" port="$2"
	shift 2
	# exec twice: the background pid is pylon's own, so the test can kill it.
	(cd "$APP" && exec env \
		DATABASE_URL="$DB_URL" \
		PYLON_SECRET="0000000000000000000000000000000000000000000000000000000000000000" \
		PYLON_SHARD_TICKET_SECRET="shard-cluster-smoke-ticket-secret" \
		PYLON_SHARD_ADVERTISE_URL="http://127.0.0.1:$port" \
		PYLON_SHARD_SAVE_SECS=1 \
		PYLON_SHARD_WS_MAX_PER_IP=0 \
		PYLON_CORS_ORIGIN="http://localhost:$port" \
		"$@" \
		"$PYLON" start app.ts --port "$port") >"$TMP/$name.log" 2>&1 &
}

echo "→ machine a on :$PORT_A, machine b on :$PORT_B, machine c (Fly) on :$PORT_C"
start a "$PORT_A" PYLON_REPLICA_ID=a
PID_A=$!
start b "$PORT_B" PYLON_REPLICA_ID=b
PID_B=$!
start c "$PORT_C" FLY_MACHINE_ID=c
PID_C=$!
for port in "$PORT_A" "$PORT_B" "$PORT_C"; do
	for _ in $(seq 1 120); do
		curl -sf -o /dev/null "http://127.0.0.1:$port/health" && break
		sleep 0.5
	done
	curl -sf -o /dev/null "http://127.0.0.1:$port/health" || {
		cat "$TMP"/*.log >&2
		echo "::error::the machine on :$port did not come up" >&2
		exit 1
	}
done
# Both machines in the directory before the test places anything.
sleep 3

echo "→ fly-replay: machine c sends a request for a shard on a to a"
TOKEN=$(curl -sf -X POST "http://127.0.0.1:$PORT_A/api/auth/guest" | sed -E 's/.*"token":"([^"]+)".*/\1/')
curl -sf -X POST "http://127.0.0.1:$PORT_A/api/fn/joinArena" \
	-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
	-d '{"arena":"replay-check","machine":"a"}' >/dev/null
for path in "/shard?shard=replay-check&sid=u&v=2" "/api/shards/replay-check/input"; do
	HEAD=$(curl -si "http://127.0.0.1:$PORT_C$path" \
		-H "Upgrade: websocket" -H "Connection: Upgrade" \
		-H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" -H "Sec-WebSocket-Version: 13" | tr -d '\r')
	grep -qi "^fly-replay: instance=a$" <<<"$HEAD" || {
		echo "$HEAD" >&2
		echo "::error::machine c did not answer $path with fly-replay: instance=a" >&2
		exit 1
	}
done
kill "$PID_C"
PID_C=""

echo "→ proxied HTTP: an input sent to b for the shard on a"
# The shard on a answers (an input needs a connected subscriber, so it
# refuses this one): the refusal proves the request reached a's shard.
REPLY=$(curl -s -X POST "http://127.0.0.1:$PORT_B/api/shards/replay-check/input" \
	-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
	-d '{"input":"join","client_seq":1}')
grep -q 'is not attached to this shard' <<<"$REPLY" || {
	echo "$REPLY" >&2
	echo "::error::b did not forward the input to a" >&2
	exit 1
}
grep -q "POST /api/shards/replay-check/input" "$TMP/a.log" || {
	echo "::error::a never saw the forwarded input" >&2
	exit 1
}

echo "→ e2e: create on b through a, connect through a, kill b"
(cd "$ROOT/packages/realtime" &&
	PYLON_SHARD_CLUSTER_E2E="127.0.0.1:$PORT_A,127.0.0.1:$PORT_B" \
		PYLON_SHARD_CLUSTER_KILL_B="$PID_B" \
		bun test src/shard-cluster.e2e.test.ts) || {
	echo "--- machine a ---" >&2
	tail -40 "$TMP/a.log" >&2
	echo "--- machine b ---" >&2
	tail -40 "$TMP/b.log" >&2
	exit 1
}
PID_B=""
grep -q "starting it here" "$TMP/a.log" || {
	tail -40 "$TMP/a.log" >&2
	echo "::error::machine a did not take over the arena" >&2
	exit 1
}

echo
echo "✓ shards across machines: placement, routing, and failover from saved state"
