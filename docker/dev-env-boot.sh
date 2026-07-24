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
#
# Colocated coding agent (OpenCode) — started only when ALL three are set:
#   PYLON_DEV_MODEL_PROXY_URL    base URL of pylon-model-proxy (holds the real key)
#   PYLON_DEV_MODEL_PROXY_TOKEN  per-env token the box uses instead of a real key
#   PYLON_DEV_MODEL              provider model id (e.g. moonshotai/kimi-k3,
#                                gpt-4o, deepseek-chat) — matches the proxy's upstream
#   PYLON_DEV_OPENCODE_PORT      opencode serve port           (default 4096)
#   PYLON_DEV_OPENCODE_PASSWORD  basic-auth for the opencode server (control plane
#                                sets + uses it; server is 6PN-private regardless)
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

# --- Colocated coding agent (OpenCode) -------------------------------------
# When the control plane grants model access, run `opencode serve` next to
# `pylon dev` so an agent authors THIS workspace with hot-reload feedback.
# OpenCode never sees a real provider key — it calls pylon-model-proxy with a
# per-env, model-pinned token (PYLON_DEV_MODEL_PROXY_TOKEN). The server binds
# the private net for the control-plane driver to reach over 6PN; it's NOT on
# the machine's public services, and is basic-auth gated on top.
if [ -n "${PYLON_DEV_MODEL_PROXY_URL:-}" ] &&
	[ -n "${PYLON_DEV_MODEL_PROXY_TOKEN:-}" ] &&
	[ -n "${PYLON_DEV_MODEL:-}" ] &&
	command -v opencode >/dev/null 2>&1; then
	OC_PORT="${PYLON_DEV_OPENCODE_PORT:-4096}"
	OC_CFG="${HOME:-/app}/.config/opencode"
	mkdir -p "$OC_CFG"
	# Regenerated every boot from env (rootfs is ephemeral) — no secrets persist.
	cat >"$OC_CFG/opencode.json" <<JSON
{
  "provider": {
    "pylon": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "Pylon Build",
      "options": { "baseURL": "${PYLON_DEV_MODEL_PROXY_URL}/v1", "apiKey": "${PYLON_DEV_MODEL_PROXY_TOKEN}" },
      "models": { "${PYLON_DEV_MODEL}": { "name": "${PYLON_DEV_MODEL}" } }
    }
  },
  "model": "pylon/${PYLON_DEV_MODEL}"
}
JSON
	export OPENCODE_SERVER_PASSWORD="${PYLON_DEV_OPENCODE_PASSWORD:-}"
	echo "[dev-boot] starting opencode serve on :$OC_PORT (model pylon/${PYLON_DEV_MODEL})"
	(opencode serve --port "$OC_PORT" --hostname :: >/tmp/opencode.log 2>&1 &)
fi

echo "[dev-boot] exec pylon dev --port $PORT (workspace=$WORKSPACE)"
exec "$PYLON_BIN" dev --port "$PORT"
