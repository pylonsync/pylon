#!/usr/bin/env bash
# Smoke-test the docker self-host path end to end:
#   pylon deploy --target docker  →  docker build  →  docker run  →  SSR 200
#
# Catches the bug classes that only show up inside a container:
#   - generated-Dockerfile rot (wrong base image, dead CMD, bad build context)
#   - the runner pool dying under PID-1 semantics (runtime.ts orphan watch:
#     every bun child exited every 2s when pylon start was the container's
#     PID 1, so SSR flapped forever while /health stayed green)
#
# Usage:
#   tools/smoke-docker-selfhost.sh <app-dir> [base-image]
#
#   <app-dir>     a scaffolded pylon app (create-pylon output) with app.ts
#   [base-image]  override the Dockerfile's FROM (e.g. a locally built
#                 `pylon-local` image); defaults to whatever the generated
#                 Dockerfile pins.
#
# Requires: docker, curl, a pylon binary on PATH (or PYLON_BIN).
set -euo pipefail

APP_DIR="${1:?usage: smoke-docker-selfhost.sh <app-dir> [base-image]}"
BASE_IMAGE="${2:-}"
PYLON_BIN="${PYLON_BIN:-pylon}"
PORT="${SMOKE_PORT:-4777}"
TAG="pylon-selfhost-smoke"
NAME="pylon-selfhost-smoke"

cd "$APP_DIR"

echo "→ generating docker artifacts"
"$PYLON_BIN" deploy --target docker >/dev/null

DOCKERFILE=Dockerfile
if [ -n "$BASE_IMAGE" ]; then
    sed "s|^FROM .*|FROM $BASE_IMAGE|" Dockerfile > Dockerfile.smoke
    DOCKERFILE=Dockerfile.smoke
fi

echo "→ docker build"
docker build -f "$DOCKERFILE" -t "$TAG" . >/dev/null

docker rm -f "$NAME" >/dev/null 2>&1 || true
echo "→ docker run"
docker run -d --name "$NAME" -p "$PORT:4321" "$TAG" >/dev/null
trap 'docker rm -f "$NAME" >/dev/null 2>&1 || true' EXIT

echo "→ waiting for /health"
for _ in $(seq 1 30); do
    if curl -fsS -o /dev/null "http://localhost:$PORT/health" 2>/dev/null; then break; fi
    sleep 2
done
curl -fsS -o /dev/null "http://localhost:$PORT/health" || {
    echo "FAIL: /health never came up"
    docker logs "$NAME" | tail -20
    exit 1
}

# Let the pool settle past the first orphan-watch tick (2s) with margin,
# then require BOTH a rendering SSR route and a stable runner pool.
sleep 15

CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 60 "http://localhost:$PORT/")"
if [ "$CODE" != "200" ]; then
    echo "FAIL: / returned HTTP $CODE (SSR not serving)"
    docker logs "$NAME" | tail -20
    exit 1
fi

DEATHS="$(docker logs "$NAME" 2>&1 | grep -c 'runner child exited' || true)"
FLAPS="$(docker logs "$NAME" 2>&1 | grep -c 'not alive' || true)"
if [ "${DEATHS:-0}" -gt 0 ] || [ "${FLAPS:-0}" -gt 2 ]; then
    echo "FAIL: runner pool unstable (child exits: $DEATHS, respawns: $FLAPS)"
    docker logs "$NAME" 2>&1 | grep -E "runner child exited|not alive" | tail -10
    exit 1
fi

echo "OK: SSR / is 200, runner pool stable (child exits: ${DEATHS:-0}, respawns: ${FLAPS:-0})"
