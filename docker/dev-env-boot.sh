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
#   PYLON_DEV_TEMPLATE       create-pylon template to scaffold the workspace from
#                            (default, todo, shop, chat, crm, …) — the SAME
#                            templates `npm create @pylonsync/pylon` offers,
#                            scaffolded by the same code, baked into the image
#                            at /pylon/packages/create-pylon
#                            unset = start from an empty workspace
#   PYLON_DEV_FILE_API_TOKEN bearer token gating the file-write API (set by the
#                            control plane; unset = open, local dev only)
#   PYLON_BIN                pylon binary to exec          (default `pylon` on PATH)
#
# Colocated coding agent (OpenCode) — started only when ALL three are set:
#   PYLON_DEV_MODEL_PROXY_URL    base URL of pylon-model-proxy (holds the real key)
#   PYLON_DEV_MODEL_PROXY_TOKEN  per-env token the box uses instead of a real key
#   PYLON_DEV_MODELS             comma-separated set of provider model ids the
#                                env may use (e.g. gpt-5.6-luna,gpt-5.6-sol) — a
#                                build picks one per request. Matches the proxy's upstream.
#   PYLON_DEV_MODEL              default model when a build doesn't specify one
#                                (defaults to the first of PYLON_DEV_MODELS)
#   PYLON_DEV_RESPONSES_MODELS   subset of PYLON_DEV_MODELS that speak the
#                                /v1/responses API rather than
#                                /v1/chat/completions (OpenAI GPT-5.x needs
#                                this to use function tools with reasoning)
#   PYLON_DEV_OPENCODE_PORT      opencode serve port           (default 4096)
#   PYLON_DEV_OPENCODE_PASSWORD  basic-auth for the opencode server (control plane
#                                sets + uses it; server is 6PN-private regardless)
set -eu

WORKSPACE="${PYLON_DEV_WORKSPACE:-/data/workspace}"
PORT="${PYLON_PORT:-8080}"
TEMPLATE="${PYLON_DEV_TEMPLATE:-}"
PYLON_BIN="${PYLON_BIN:-pylon}"
MARKER="$WORKSPACE/.pylon-seeded"
CREATE_PYLON="/pylon/packages/create-pylon/bin/create-pylon.js"

if [ ! -f "$MARKER" ]; then
	echo "[dev-boot] seeding workspace at $WORKSPACE"
	mkdir -p "$WORKSPACE"
	if [ -n "$TEMPLATE" ] && [ -f "$CREATE_PYLON" ]; then
		# Scaffold the SAME templates `npm create @pylonsync/pylon` offers, using
		# the scaffolder itself — the image already ships packages/, so the
		# templates and their substitution logic are right here.
		#
		# create-pylon refuses a non-empty target, and writes into ./<name>, so
		# run it from the workspace's parent with the workspace as the name.
		echo "[dev-boot] scaffolding template '$TEMPLATE'"
		(
			cd "$(dirname "$WORKSPACE")" &&
				bun "$CREATE_PYLON" "$(basename "$WORKSPACE")" \
					--template "$TEMPLATE" --skip-install --no-skill --bun </dev/null
		) || echo "[dev-boot] scaffold failed — continuing with an empty workspace"
	else
		echo "[dev-boot] no PYLON_DEV_TEMPLATE — starting from an empty workspace"
	fi
	# Deps come from npm, pinned to the image version by the template's
	# package.json — same model as `npm create @pylonsync/pylon`. A hosted
	# dev env is just that scaffold, kept running and editable.
	if [ -f "$WORKSPACE/package.json" ]; then
		echo "[dev-boot] bun install"
		(cd "$WORKSPACE" && bun install)
	fi
	# Make the workspace a git repository with the starter as its first commit.
	# OpenCode derives a session's file diff from git, so without a repo the
	# builder can never report which files a build touched — the diff endpoint
	# answers 200 with an empty array. A baseline commit also gives every
	# subsequent build something to diff against, which is what per-build
	# history and revert are built on.
	#
	# Best-effort: a workspace that fails to become a repo should still boot and
	# serve. Identity is set locally so commits don't depend on global config.
	if command -v git >/dev/null 2>&1 && [ ! -d "$WORKSPACE/.git" ]; then
		(
			cd "$WORKSPACE" || exit 0
			git init -q 2>/dev/null || exit 0
			git config user.email "agent@pylonsync.com"
			git config user.name "Pylon Build"
			git add -A 2>/dev/null
			git commit -qm "Starter workspace" 2>/dev/null
		) || echo "[dev-boot] git init skipped (non-fatal)"
	fi
	touch "$MARKER"
	echo "[dev-boot] seed complete"
fi

# Git refuses to touch a repository owned by another user ("detected dubious
# ownership"), and that is the situation here: the volume is owned by `pylon`
# while the boot script and the agent run as root. Without this the repo exists
# and every git call against it still fails, so the session diff stays empty.
#
# Set on EVERY boot, not just at seed time: this writes to /etc/gitconfig, which
# lives on the ephemeral rootfs and is wiped by any machine restart — a
# seed-only write would silently stop applying the first time the box bounced.
# The guard protects against operating on someone else's repo on a shared
# machine; this is a single-tenant sandbox holding exactly one workspace.
if command -v git >/dev/null 2>&1; then
	git config --system --add safe.directory "$WORKSPACE" 2>/dev/null || true
fi

cd "$WORKSPACE"
# The runtime writes file-API edits into the watched tree; point it at the
# workspace so a PUT lands where the fs-watcher sees it.
export PYLON_DEV_WATCH_DIR="$WORKSPACE"

# --- Colocated coding agent (OpenCode) -------------------------------------
# When the control plane grants model access, run `opencode serve` next to
# `pylon dev` so an agent authors THIS workspace with hot-reload feedback.
# OpenCode never sees a real provider key — it calls pylon-model-proxy with a
# per-env token (PYLON_DEV_MODEL_PROXY_TOKEN) scoped to a SET of models
# (PYLON_DEV_MODELS); each BUILD picks one per request. The server binds the
# private net for the control-plane driver to reach over 6PN; it's NOT on the
# machine's public services, and is basic-auth gated on top.
if [ -n "${PYLON_DEV_MODEL_PROXY_URL:-}" ] &&
	[ -n "${PYLON_DEV_MODEL_PROXY_TOKEN:-}" ] &&
	[ -n "${PYLON_DEV_MODELS:-${PYLON_DEV_MODEL:-}}" ] &&
	command -v opencode >/dev/null 2>&1; then
	OC_PORT="${PYLON_DEV_OPENCODE_PORT:-4096}"
	OC_CFG="${HOME:-/app}/.config/opencode"
	# OpenCode keeps every session — prompts, messages, tool calls, per-message
	# diffs — in a SQLite db under $XDG_DATA_HOME/opencode. That defaults to
	# $HOME/.local/share, which on Fly is the EPHEMERAL rootfs: only the volume
	# survives a stop/start. A dev env auto-sleeps when idle, so each wake came
	# back with an empty db and the whole build history read as "agent activity
	# is temporarily unavailable" — on a workspace whose builds had in fact
	# landed. Keep the db beside the workspace, on the volume.
	OC_DATA="${PYLON_DEV_AGENT_STATE:-$(dirname "$WORKSPACE")/opencode-state}"
	mkdir -p "$OC_DATA"
	export XDG_DATA_HOME="$OC_DATA"
	mkdir -p "$OC_CFG"
	# Split the allowed models across TWO providers, because OpenCode picks the
	# wire protocol from the provider's npm package and that package can only be
	# set per PROVIDER (the config schema has no per-model override):
	#
	#   pylon            @ai-sdk/openai-compatible  ->  /v1/chat/completions
	#   pylon-responses  @ai-sdk/openai             ->  /v1/responses
	#
	# OpenAI's GPT-5.x family REFUSES function tools over chat/completions while
	# reasoning is enabled, and a coding agent without tools does nothing — so
	# models named in PYLON_DEV_RESPONSES_MODELS have to go through the second
	# provider. Both point at the same proxy with the same token; only the
	# protocol differs. An org mixing vendors gets some models in each.
	OC_CHAT_JSON=""
	OC_RESP_JSON=""
	OLD_IFS="$IFS"
	IFS=","
	for m in ${PYLON_DEV_MODELS:-$PYLON_DEV_MODEL}; do
		m="$(printf '%s' "$m" | sed 's/^ *//;s/ *$//')"
		[ -z "$m" ] && continue
		entry="\"$m\": { \"name\": \"$m\" }"
		# Exact membership test that does NOT depend on IFS — this loop runs
		# with IFS="," to split the model list, which would otherwise swallow a
		# nested word-split and silently classify every model as chat.
		is_resp=0
		case ",$(printf '%s' "${PYLON_DEV_RESPONSES_MODELS:-}" | tr -d ' ')," in
		*",$m,"*) is_resp=1 ;;
		esac
		if [ "$is_resp" = "1" ]; then
			if [ -z "$OC_RESP_JSON" ]; then OC_RESP_JSON="$entry"; else OC_RESP_JSON="$OC_RESP_JSON, $entry"; fi
		else
			if [ -z "$OC_CHAT_JSON" ]; then OC_CHAT_JSON="$entry"; else OC_CHAT_JSON="$OC_CHAT_JSON, $entry"; fi
		fi
	done
	IFS="$OLD_IFS"
	OC_DEFAULT="${PYLON_DEV_MODEL:-$(printf '%s' "$PYLON_DEV_MODELS" | cut -d, -f1 | sed 's/^ *//;s/ *$//')}"
	# Which provider serves the default model decides the `model` line.
	OC_DEFAULT_PROVIDER="pylon"
	case ",$(printf '%s' "${PYLON_DEV_RESPONSES_MODELS:-}" | tr -d ' ')," in
	*",$OC_DEFAULT,"*) OC_DEFAULT_PROVIDER="pylon-responses" ;;
	esac

	OC_PROVIDERS=""
	if [ -n "$OC_CHAT_JSON" ]; then
		OC_PROVIDERS="\"pylon\": { \"npm\": \"@ai-sdk/openai-compatible\", \"name\": \"Pylon Build\", \"options\": { \"baseURL\": \"${PYLON_DEV_MODEL_PROXY_URL}/v1\", \"apiKey\": \"${PYLON_DEV_MODEL_PROXY_TOKEN}\" }, \"models\": { ${OC_CHAT_JSON} } }"
	fi
	if [ -n "$OC_RESP_JSON" ]; then
		RESP_BLOCK="\"pylon-responses\": { \"npm\": \"@ai-sdk/openai\", \"name\": \"Pylon Build (Responses)\", \"options\": { \"baseURL\": \"${PYLON_DEV_MODEL_PROXY_URL}/v1\", \"apiKey\": \"${PYLON_DEV_MODEL_PROXY_TOKEN}\" }, \"models\": { ${OC_RESP_JSON} } }"
		if [ -z "$OC_PROVIDERS" ]; then OC_PROVIDERS="$RESP_BLOCK"; else OC_PROVIDERS="$OC_PROVIDERS, $RESP_BLOCK"; fi
	fi

	# Regenerated every boot from env (rootfs is ephemeral) — no secrets persist.
	cat >"$OC_CFG/opencode.json" <<JSON
{
  "provider": { ${OC_PROVIDERS} },
  "model": "${OC_DEFAULT_PROVIDER}/${OC_DEFAULT}"
}
JSON
	export OPENCODE_SERVER_PASSWORD="${PYLON_DEV_OPENCODE_PASSWORD:-}"
	echo "[dev-boot] starting opencode serve on :$OC_PORT (models: ${PYLON_DEV_MODELS:-$PYLON_DEV_MODEL}, default pylon/${OC_DEFAULT})"
	(opencode serve --port "$OC_PORT" --hostname :: >/tmp/opencode.log 2>&1 &)
fi

echo "[dev-boot] exec pylon dev --port $PORT (workspace=$WORKSPACE)"
exec "$PYLON_BIN" dev --port "$PORT"
