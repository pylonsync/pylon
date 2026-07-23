#!/bin/sh
# Pylon Cloud dev-mode env bootstrap.
#
# Baked into the runtime image; Pylon Cloud's `provisionDevEnvironment` sets
# this as the machine's init.cmd. It turns a bare machine into a live,
# mutable `pylon dev` workspace that an agent — or build.pylonsync.com —
# authors in place through the file-write API (/_pylon/dev/files/<path>),
# with edits hot-reloading via the fs-watcher + in-process reload path.
#
# Idempotent by design: the workspace lives on the /data volume and is seeded
# exactly ONCE (guarded by a marker file). Every subsequent boot re-execs
# `pylon dev` against the existing tree, so a machine restart never clobbers
# the user's live edits.
#
# Inputs (env):
#   PYLON_DEV_WORKSPACE      workspace dir on the volume   (default /data/workspace)
#   PYLON_PORT               HTTP port pylon dev binds     (default 8080)
#   PYLON_DEV_SEED_URL       tarball to seed the workspace (http(s):// | file:// | path)
#                            unset  = start from an empty workspace
#   PYLON_DEV_FILE_API_TOKEN bearer token gating the file-write API (set by the
#                            control plane; unset = open, local dev only)
#   PYLON_BIN                pylon binary to exec          (default `pylon` on PATH)
set -eu

WORKSPACE="${PYLON_DEV_WORKSPACE:-/data/workspace}"
PORT="${PYLON_PORT:-8080}"
SEED_URL="${PYLON_DEV_SEED_URL:-}"
PYLON_BIN="${PYLON_BIN:-pylon}"
MARKER="$WORKSPACE/.pylon-seeded"

if [ ! -f "$MARKER" ]; then
	echo "[dev-boot] seeding workspace at $WORKSPACE"
	mkdir -p "$WORKSPACE"
	if [ -n "$SEED_URL" ]; then
		case "$SEED_URL" in
		http://* | https://*) curl -fsSL "$SEED_URL" | tar -xz -C "$WORKSPACE" ;;
		file://*) tar -xzf "${SEED_URL#file://}" -C "$WORKSPACE" ;;
		*) tar -xzf "$SEED_URL" -C "$WORKSPACE" ;;
		esac
	else
		echo "[dev-boot] no PYLON_DEV_SEED_URL — starting from an empty workspace"
	fi
	# Deps come from npm, pinned to the image version by the template's
	# package.json — same model as `npm create @pylonsync/pylon`. A hosted
	# dev env is just that scaffold, kept running and editable.
	if [ -f "$WORKSPACE/package.json" ]; then
		echo "[dev-boot] bun install"
		(cd "$WORKSPACE" && bun install)
	fi
	touch "$MARKER"
	echo "[dev-boot] seed complete"
fi

cd "$WORKSPACE"
# The runtime writes file-API edits into the watched tree; point it at the
# workspace so a PUT lands where the fs-watcher sees it.
export PYLON_DEV_WATCH_DIR="$WORKSPACE"
echo "[dev-boot] exec pylon dev --port $PORT (workspace=$WORKSPACE)"
exec "$PYLON_BIN" dev --port "$PORT"
