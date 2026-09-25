#!/usr/bin/env bash
# smoke-shard-cluster.sh — shards across machines, with real pylon processes
# on one Postgres running examples/shard-arena. No shared secret is set: the
# machines take their keys from the shard directory.
#
#   tools/smoke-shard-cluster.sh [path/to/pylon]
#
#   1. fly-replay: machine c answers a request for a shard on a with
#      `fly-replay: instance=a` (both have FLY_MACHINE_ID set).
#   2. proxied HTTP: an input sent to b for a shard on a reaches a's shard.
#   3. packages/realtime's shard-cluster.e2e.test.ts creates an arena through
#      a, pinned to b; connects through a (proxied to b: b is not on Fly);
#      moves a player; kills b; and finds the arena started on a from b's
#      saved state, with the player where it was, after the client
#      reconnects.
#   4. restart: a is killed and started again under the same id; once the
#      old process is silent for 10 s, it starts the shards placed on it
#      from saved state.
#   5. graceful leave: machine d stops (SIGTERM); its shard starts on a well
#      before a dead machine's would.
#   6. fencing: machine e reaches Postgres through a proxy; the proxy starts
#      dropping every byte without closing a socket, so e's database calls
#      hang. e's lease lapses and it stops its shard before a starts it.
#
# Needs Postgres (createdb/psql on PATH, or PYLON_SMOKE_PG_URL pointing at
# an empty database), Rust with the wasm32-unknown-unknown target, and
# `bun install` at the repo root. PYLON_SMOKE_KEEP=1 keeps the logs.

set -euo pipefail
export LC_ALL=C

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
PORT_A=4851
PORT_B=4861
PORT_C=4871
PORT_D=4881
PORT_E=4891
PG_PROXY_PORT=55432
TMP="$(mktemp -d -t pylon-shard-cluster.XXXXXX)"
PIDS=()
DB_NAME="pylon_shard_cluster_smoke"
ADMIN_TOKEN="shard-cluster-smoke-admin-0123456789"

cleanup() {
	for pid in "${PIDS[@]}"; do
		kill "$pid" 2>/dev/null || true
	done
	wait 2>/dev/null || true
	[[ -f "$TMP/arena.wasm.committed" ]] && cp "$TMP/arena.wasm.committed" "$APP/shards/arena.wasm"
	if [[ -n "${PYLON_SMOKE_KEEP:-}" ]]; then
		echo "kept $TMP"
	else
		rm -rf "$TMP"
	fi
}
trap cleanup EXIT

fail() {
	echo "::error::$1" >&2
	for log in "$TMP"/*.log; do
		echo "--- $(basename "$log") ---" >&2
		tail -25 "$log" >&2
	done
	exit 1
}

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
(cd "$APP" && "$PYLON" shards build) >"$TMP/build.log" 2>&1 || fail "pylon shards build failed"

# start <log name> <port> [VAR=value...]: a machine on <port>. Sets PID.
start() {
	local log="$1" port="$2"
	shift 2
	# exec twice: the background pid is pylon's own, so it can be killed.
	(cd "$APP" && exec env \
		DATABASE_URL="$DB_URL" \
		PYLON_SECRET="0000000000000000000000000000000000000000000000000000000000000000" \
		PYLON_SHARD_ADVERTISE_URL="http://127.0.0.1:$port" \
		PYLON_SHARD_SAVE_SECS=1 \
		PYLON_SHARD_WS_MAX_PER_IP=0 \
		PYLON_CORS_ORIGIN="http://localhost:$port" \
		PYLON_ADMIN_TOKEN="$ADMIN_TOKEN" \
		"$@" \
		"$PYLON" start app.ts --port "$port") >"$TMP/$log.log" 2>&1 &
	PID=$!
	PIDS+=("$PID")
}

up() {
	local port="$1"
	for _ in $(seq 1 120); do
		curl -sf -o /dev/null "http://127.0.0.1:$port/health" && return 0
		sleep 0.5
	done
	fail "the machine on :$port did not come up"
}

# wait_log <file> <pattern> <seconds>
wait_log() {
	for _ in $(seq 1 $(($3 * 4))); do
		grep -q -- "$2" "$TMP/$1" && return 0
		sleep 0.25
	done
	fail "no \"$2\" in $1 within $3 s"
}

# join <port> <token> <json args>
join() {
	curl -sf -X POST "http://127.0.0.1:$1/api/fn/joinArena" \
		-H "Authorization: Bearer $2" -H "Content-Type: application/json" -d "$3"
}

echo "→ machines a (:$PORT_A, Fly), b (:$PORT_B), c (:$PORT_C, Fly)"
start a "$PORT_A" PYLON_REPLICA_ID=a FLY_MACHINE_ID=a
PID_A=$PID
start b "$PORT_B" PYLON_REPLICA_ID=b
PID_B=$PID
start c "$PORT_C" FLY_MACHINE_ID=c
PID_C=$PID
up "$PORT_A"
up "$PORT_B"
up "$PORT_C"
# Every machine in the directory before anything is placed.
sleep 3
TOKEN=$(curl -sf -X POST "http://127.0.0.1:$PORT_A/api/auth/guest" | sed -E 's/.*"token":"([^"]+)".*/\1/')

echo "→ 1. fly-replay: c sends a request for a shard on a to a"
join "$PORT_A" "$TOKEN" '{"arena":"replay-check","machine":"a"}' >/dev/null ||
	fail "joinArena on a failed"
for path in "/shard?shard=replay-check&sid=u&v=2" "/api/shards/replay-check/input"; do
	HEAD=$(curl -si --max-time 5 "http://127.0.0.1:$PORT_C$path" \
		-H "Upgrade: websocket" -H "Connection: Upgrade" \
		-H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" -H "Sec-WebSocket-Version: 13" | head -c 2000 | tr -d '\r')
	grep -qi "^fly-replay: instance=a$" <<<"$HEAD" || {
		echo "$HEAD" >&2
		fail "c did not answer $path with fly-replay: instance=a"
	}
done
kill "$PID_C"

echo "→ 2. proxied HTTP: an input sent to b reaches the shard on a"
# The shard on a answers (an input needs a connected subscriber, so it
# refuses this one): the refusal proves the request reached a's shard.
REPLY=$(curl -s -X POST "http://127.0.0.1:$PORT_B/api/shards/replay-check/input" \
	-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
	-d '{"input":"join","client_seq":1}')
grep -q 'is not attached to this shard' <<<"$REPLY" || {
	echo "$REPLY" >&2
	fail "b did not forward the input to a"
}

echo "→ 2b. the shard port: a WebSocket to b's shard port (+3) reaches a"
curl -si --max-time 3 "http://127.0.0.1:$((PORT_B + 3))/?shard=replay-check&sid=port-check&v=2" \
	-H "Upgrade: websocket" -H "Connection: Upgrade" \
	-H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" -H "Sec-WebSocket-Version: 13" >/dev/null 2>&1 || true
wait_log a.log "GET /shard?shard=replay-check&sid=port-check" 5

echo "→ 3. e2e: create on b through a, connect through a, kill b"
(cd "$ROOT/packages/realtime" &&
	PYLON_SHARD_CLUSTER_E2E="127.0.0.1:$PORT_A,127.0.0.1:$PORT_B" \
		PYLON_SHARD_CLUSTER_KILL_B="$PID_B" \
		bun test src/shard-cluster.e2e.test.ts) || fail "the e2e test failed"
grep -q "starting it here" "$TMP/a.log" || fail "a did not take over b's arena"

echo "→ 4. restart: a comes back under the same id and starts its shards"
kill -9 "$PID_A"
wait "$PID_A" 2>/dev/null || true
start a-restarted "$PORT_A" PYLON_REPLICA_ID=a FLY_MACHINE_ID=a
PID_A=$PID
up "$PORT_A"
wait_log a-restarted.log "\[shard replay-check\] started (arena, from saved state)" 30

echo "→ 5. graceful leave: d stops and its shard moves at once"
start d "$PORT_D" PYLON_REPLICA_ID=d
PID_D=$PID
up "$PORT_D"
sleep 3
join "$PORT_A" "$TOKEN" '{"arena":"leave-check","machine":"d"}' >/dev/null ||
	fail "joinArena pinned to d failed"
sleep 2
LEFT_AT=$(date +%s)
kill -TERM "$PID_D"
wait_log a-restarted.log "\[shard leave-check\] machine d is dead; starting it here" 8
(($(date +%s) - LEFT_AT < 8)) || fail "d's shard took the dead-machine delay to move"

echo "→ 6. fencing: e's database calls hang; e stops its shard, then a starts it"
url_part() { bun -e 'const u = new URL(process.argv[1]); console.log(process.argv[2] === "host" ? u.hostname : (u.port || "5432"))' "$DB_URL" "$1"; }
PROXY_URL=$(bun -e 'const u = new URL(process.argv[1]); u.hostname = "127.0.0.1"; u.port = process.argv[2]; console.log(u.toString())' "$DB_URL" "$PG_PROXY_PORT")
bun "$ROOT/tools/tcp-proxy.ts" "$PG_PROXY_PORT" "$(url_part host)" "$(url_part port)" >"$TMP/pg-proxy.log" 2>&1 &
PROXY_PID=$!
PIDS+=("$PROXY_PID")
sleep 1
start e "$PORT_E" PYLON_REPLICA_ID=e DATABASE_URL="$PROXY_URL"
PID_E=$PID
up "$PORT_E"
sleep 3
join "$PORT_A" "$TOKEN" '{"arena":"fence-check","machine":"e"}' >/dev/null ||
	fail "joinArena pinned to e failed"
sleep 2
kill -USR1 "$PROXY_PID"
wait_log e.log "\[shard fence-check\] stopped: this machine's lease on the shard directory lapsed" 15
wait_log a-restarted.log "\[shard fence-check\] took over from machine e" 25
# e stopped its copy (the line follows the end of its last tick) before a
# started one.
stamp() { grep -- "$2" "$TMP/$1" | head -1 | sed -E 's/\x1b\[[0-9;]*m//g' | awk '{print $1}'; }
FENCED=$(stamp e.log "\[shard fence-check\] stopped")
ADOPTED=$(stamp a-restarted.log "\[shard fence-check\] started")
[[ "$FENCED" < "$ADOPTED" ]] || fail "e stopped its copy at $FENCED, after a started one at $ADOPTED"
# a's copy runs: its tick count goes up.
tick() {
	curl -sf "http://127.0.0.1:$PORT_A/api/shards/fence-check" -H "Authorization: Bearer $ADMIN_TOKEN" |
		sed -E 's/.*"tick":([0-9]+).*/\1/'
}
T1=$(tick) || fail "a does not report fence-check"
sleep 1
T2=$(tick) || fail "a does not report fence-check"
((T2 > T1)) || fail "fence-check on a is not ticking ($T1, then $T2)"

echo
echo "✓ shards across machines: placement, routing, restarts, leaving, fencing, failover"
