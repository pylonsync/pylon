#!/usr/bin/env bash
# smoke-cluster.sh — prove the multi-machine story with two REAL
# pylon processes sharing Postgres, fanning realtime over the Redis
# cluster bus. This script checks the shared data and realtime paths:
#
#   1. shared data       insert on A → readable on B
#   2. sync cursor       change on A → B's /api/sync/pull sees it
#   3. change fanout     write on A → B's connected WS gets the event
#   4. CRDT relay        Loro edit on A → B's subscriber gets the frame
#   5. presence relay    presence set on B → A's room subscriber sees it
#   6. cron leadership   exactly ONE machine acquires the scheduler lock
#
# Requirements: postgres running locally (psql on PATH), redis-server,
# bun, a pylon binary (PYLON_BIN, default target/debug/pylon), and
# examples/pad's node_modules (bun install there once).
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PYLON_BIN="${PYLON_BIN:-$ROOT/target/debug/pylon}"
APP_DIR="$ROOT/examples/pad"
DB_NAME="pylon_cluster_smoke"
REDIS_PORT=6399
# Spaced by 10: each machine claims port, port+1 (WS), port+3 (shards).
PORT_A=4811
PORT_B=4821
SCRATCH="$(mktemp -d /tmp/pylon-cluster-smoke.XXXXXX)"
PASS=0; FAIL=0

say()  { printf '\n=== %s\n' "$*"; }
ok()   { printf '  PASS  %s\n' "$*"; PASS=$((PASS+1)); }
bad()  { printf '  FAIL  %s\n' "$*"; FAIL=$((FAIL+1)); }

cleanup() {
  # NEVER kill an unset pid — `kill 0` signals the whole process group.
  [ -n "${PID_A:-}" ] && kill "$PID_A" 2>/dev/null
  [ -n "${PID_B:-}" ] && kill "$PID_B" 2>/dev/null
  redis-cli -p $REDIS_PORT shutdown nosave 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

say "scratch stores"
# Evict stragglers from a previous run: their sessions block dropdb.
pkill -f "pylon start app.ts" 2>/dev/null && sleep 1
psql -d postgres -qc "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname='$DB_NAME' AND pid <> pg_backend_pid();" >/dev/null 2>&1
dropdb --if-exists "$DB_NAME" || { echo "dropdb failed"; exit 1; }
createdb "$DB_NAME" || { echo "createdb failed — is postgres running?"; exit 1; }
redis-server --daemonize yes --port $REDIS_PORT --save '' >/dev/null || exit 1
redis-cli -p $REDIS_PORT flushall >/dev/null
DB_URL="postgres://$(whoami)@localhost:5432/$DB_NAME"

say "booting machine A ($PORT_A) and machine B ($PORT_B)"
common_env=(
  DATABASE_URL="$DB_URL"
  PYLON_CLUSTER_BUS="redis://127.0.0.1:$REDIS_PORT"
  PYLON_SECRET="0000000000000000000000000000000000000000000000000000000000000000"
)
( cd "$APP_DIR" && env "${common_env[@]}" \
    PYLON_PORT=$PORT_A PYLON_CORS_ORIGIN="http://localhost:$PORT_A" \
    "$PYLON_BIN" start app.ts >"$SCRATCH/a.log" 2>&1 ) & PID_A=$!
( cd "$APP_DIR" && env "${common_env[@]}" \
    PYLON_PORT=$PORT_B PYLON_CORS_ORIGIN="http://localhost:$PORT_B" \
    "$PYLON_BIN" start app.ts >"$SCRATCH/b.log" 2>&1 ) & PID_B=$!

for port in $PORT_A $PORT_B; do
  for _ in $(seq 1 60); do
    curl -sf -o /dev/null "http://localhost:$port/health" && break
    sleep 1
  done
  curl -sf -o /dev/null "http://localhost:$port/health" \
    || { echo "machine on :$port never became healthy — logs in $SCRATCH"; exit 1; }
done
A="http://localhost:$PORT_A"; B="http://localhost:$PORT_B"
echo "  both healthy (logs: $SCRATCH)"

TOKEN_A=$(curl -s -X POST "$A/api/auth/guest" | bun -e 'const j=await new Response(Bun.stdin.stream()).json();console.log(j.token)')
TOKEN_B=$(curl -s -X POST "$B/api/auth/guest" | bun -e 'const j=await new Response(Bun.stdin.stream()).json();console.log(j.token)')
[ -n "$TOKEN_A" ] && [ -n "$TOKEN_B" ] || { echo "guest auth failed"; exit 1; }

say "1. shared data: insert on A → read on B"
NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
DOC_ID=$(curl -s -X POST "$A/api/entities/Doc" \
  -H "Authorization: Bearer $TOKEN_A" -H 'content-type: application/json' \
  -d "{\"title\":\"cluster\",\"content\":\"hello from A\",\"createdBy\":\"smoke\",\"createdAt\":\"$NOW\",\"updatedAt\":\"$NOW\"}" \
  | bun -e 'const j=await new Response(Bun.stdin.stream()).json();console.log(j.id ?? "")')
if [ -n "$DOC_ID" ] && curl -s "$B/api/entities/Doc/$DOC_ID" -H "Authorization: Bearer $TOKEN_B" | grep -q "hello from A"; then
  ok "row inserted on A is readable on B ($DOC_ID)"
else
  bad "cross-machine read (doc id: '$DOC_ID')"
fi

say "2. sync cursor: change on A visible in B's pull"
PULL=$(curl -s "$B/api/sync/pull?since=0" -H "Authorization: Bearer $TOKEN_B")
if echo "$PULL" | grep -q "$DOC_ID"; then
  ok "B's /api/sync/pull?since=0 carries A's insert"
else
  bad "B's pull missing A's insert"
fi
# cursor is an object ({last_seq}); pull's `since` takes the seq number.
CURSOR=$(echo "$PULL" | bun -e 'const j=await new Response(Bun.stdin.stream()).json();console.log(j.cursor?.last_seq ?? j.cursor ?? 0)')
curl -s -X PATCH "$A/api/entities/Doc/$DOC_ID" \
  -H "Authorization: Bearer $TOKEN_A" -H 'content-type: application/json' \
  -d '{"title":"cluster-updated"}' >/dev/null
sleep 2
if curl -s "$B/api/sync/pull?since=$CURSOR" -H "Authorization: Bearer $TOKEN_B" | grep -q "cluster-updated"; then
  ok "post-boot change on A appears in B's incremental pull"
else
  bad "B's incremental pull (since=$CURSOR) missing A's update"
fi

say "3. change fanout: write on A → B's WS client sees it"
bun "$ROOT/tools/cluster-ws-probe.ts" change "ws://localhost:$PORT_B/api/sync/ws" "$TOKEN_B" Doc \
  --expect 1 --match "$DOC_ID" --timeout 12000 >"$SCRATCH/probe-change.out" 2>"$SCRATCH/probe-change.err" & PROBE=$!
sleep 2
curl -s -X PATCH "$A/api/entities/Doc/$DOC_ID" \
  -H "Authorization: Bearer $TOKEN_A" -H 'content-type: application/json' \
  -d '{"title":"fanout-check"}' >/dev/null
if wait $PROBE; then
  ok "A's write reached a WS subscriber on B ($(grep -c CHANGE "$SCRATCH/probe-change.out") event)"
else
  bad "no change event on B within 12s (see $SCRATCH/probe-change.*)"
fi

say "4. CRDT relay: Loro edit on A → frame on B"
bun "$ROOT/tools/cluster-ws-probe.ts" crdt "ws://localhost:$PORT_B/api/sync/ws" "$TOKEN_B" Doc "$DOC_ID" \
  --expect 2 --timeout 15000 >"$SCRATCH/probe-crdt.out" 2>"$SCRATCH/probe-crdt.err" & PROBE=$!
sleep 2
( cd "$APP_DIR" && bun "$ROOT/tools/cluster-crdt-edit.ts" "$A" "$TOKEN_A" Doc "$DOC_ID" " +edited-on-A" ) \
  >"$SCRATCH/crdt-edit.out" 2>&1
if wait $PROBE; then
  ok "catch-up + relayed CRDT frame both landed on B"
else
  bad "B's CRDT subscriber missed the relayed frame (see $SCRATCH/probe-crdt.* + crdt-edit.out)"
fi
sleep 1
if curl -s "$B/api/entities/Doc/$DOC_ID" -H "Authorization: Bearer $TOKEN_B" | grep -q "edited-on-A"; then
  ok "CRDT edit projected into the shared row (read from B)"
else
  bad "projection of A's CRDT edit not visible from B"
fi

say "5. presence relay: join+presence on B → room subscriber on A"
ROOM="pad:$DOC_ID"
curl -s -X POST "$A/api/rooms/join" -H "Authorization: Bearer $TOKEN_A" \
  -H 'content-type: application/json' -d "{\"room\":\"$ROOM\",\"presence\":{\"name\":\"probe-A\"}}" >/dev/null
bun "$ROOT/tools/cluster-ws-probe.ts" room "ws://localhost:$PORT_A/api/sync/ws" "$TOKEN_A" "$ROOM" \
  --expect 1 --match "cursor-from-B" --timeout 12000 >"$SCRATCH/probe-room.out" 2>"$SCRATCH/probe-room.err" & PROBE=$!
sleep 2
curl -s -X POST "$B/api/rooms/join" -H "Authorization: Bearer $TOKEN_B" \
  -H 'content-type: application/json' -d "{\"room\":\"$ROOM\",\"presence\":{\"name\":\"cursor-from-B\"}}" >/dev/null
curl -s -X POST "$B/api/rooms/presence" -H "Authorization: Bearer $TOKEN_B" \
  -H 'content-type: application/json' -d "{\"room\":\"$ROOM\",\"data\":{\"name\":\"cursor-from-B\",\"caret\":42}}" >/dev/null
if wait $PROBE; then
  ok "B's presence reached A's room subscriber"
else
  bad "A's room subscriber never saw B's presence (see $SCRATCH/probe-room.*)"
fi

say "6. cron leadership: exactly one leader"
sleep 1
LEADERS=$(grep -l "acquired cluster leadership" "$SCRATCH/a.log" "$SCRATCH/b.log" 2>/dev/null | wc -l | tr -d ' ')
if [ "$LEADERS" = "1" ]; then
  ok "exactly one machine acquired scheduler leadership"
else
  bad "expected 1 leader, found $LEADERS (a.log/b.log in $SCRATCH)"
fi

say "result: $PASS pass, $FAIL fail"
[ $FAIL -eq 0 ] || exit 1
