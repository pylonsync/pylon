#!/usr/bin/env bash
# Post-deploy smoke test for the PylonSync relay worker.
#
# Verifies routing + auth-gating on a DEPLOYED worker without needing
# the HMAC secret: every check here is a negative (missing app, missing
# subprotocol, unsigned body) with a deterministic status code. A green
# run proves the /sync/* routes reach the DO and fail closed.
#
# Usage:
#   crates/workers/scripts/relay-smoke.sh https://pylon-sync-relay.<subdomain>.workers.dev
#
# It does NOT prove positive delivery — that needs a signed push and a
# real subscriber, which the local dual-write diff covers.

set -euo pipefail

BASE="${1:-}"
if [[ -z "$BASE" ]]; then
  echo "usage: $0 <worker-base-url>" >&2
  exit 2
fi
BASE="${BASE%/}"

fail=0

# $1 method, $2 path, $3 expected status, $4 description
#
# Retries once on a mismatch: a freshly-set `wrangler secret put` takes
# a few seconds to propagate to every edge isolate, and a cold DO's
# first request can race that. One retry after a short pause turns that
# transient into a pass without masking a real failure.
check() {
  local method="$1" path="$2" want="$3" desc="$4"
  local got
  for attempt in 1 2; do
    got=$(curl -sS -o /dev/null -w '%{http_code}' -X "$method" \
          --max-time 15 "${BASE}${path}" || echo "000")
    [[ "$got" == "$want" ]] && break
    [[ "$attempt" == 1 ]] && sleep 4
  done
  if [[ "$got" == "$want" ]]; then
    printf '  ok   %-3s %-34s -> %s  (%s)\n' "$method" "$path" "$got" "$desc"
  else
    printf '  FAIL %-3s %-34s -> %s, want %s  (%s)\n' "$method" "$path" "$got" "$want" "$desc"
    fail=1
  fi
}

echo "relay smoke: $BASE"

# handler.rs rejects an empty ?app= before the DO (400).
check GET  "/sync/ws"                    400 "empty app rejected at the edge"
# The DO runs but the request carries no bearer.<blob> subprotocol.
check GET  "/sync/ws?app=smoketest"      401 "ws without relay subprotocol"
# verified_body denies an unsigned push/manifest/status (401).
check POST "/sync/push?app=smoketest"    401 "unsigned push denied"
check POST "/sync/manifest?app=smoketest" 401 "unsigned manifest denied"
check GET  "/sync/status?app=smoketest"  401 "unsigned status denied"
# Unknown /sync/* verb+path -> DO 404 (proves routing reached the DO).
check POST "/sync/nope?app=smoketest"    404 "unknown sync path -> DO 404"

if [[ "$fail" == 0 ]]; then
  echo "PASS"
else
  echo "FAIL — one or more checks did not match" >&2
  exit 1
fi
