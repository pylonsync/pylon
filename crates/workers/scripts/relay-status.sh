#!/usr/bin/env bash
# Read a PylonSync DO's operational status with a valid machine
# signature. /sync/status is HMAC-gated exactly like push/manifest, so
# this reproduces the machine's signing: HMAC-SHA256(secret, "<ts>."+body),
# lowercase hex (pylon_auth::trusted_mint::sign). Body is empty for a
# status read.
#
# Usage:
#   PYLON_RELAY_SECRET=<secret> \
#     crates/workers/scripts/relay-status.sh <worker-base-url> <app>
#
# Prints the DO's status JSON (has_manifest, ring_len, oldest/latest_seq,
# sockets) — the live view of what the machine has shipped to this app's
# DO.

set -euo pipefail

BASE="${1:?usage: relay-status.sh <worker-base-url> <app>}"
APP="${2:?usage: relay-status.sh <worker-base-url> <app>}"
BASE="${BASE%/}"
SECRET="${PYLON_RELAY_SECRET:?set PYLON_RELAY_SECRET}"

ts=$(date +%s)
body=""
# sign "<ts>.<body>" — note the literal dot the primitive prepends.
sig=$(printf '%s.%s' "$ts" "$body" \
      | openssl dgst -sha256 -hmac "$SECRET" -r | awk '{print $1}')

curl -sS --max-time 15 \
  -H "X-Pylon-Relay-Timestamp: $ts" \
  -H "X-Pylon-Relay-Signature: $sig" \
  "${BASE}/sync/status?app=${APP}"
echo
