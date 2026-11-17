#!/usr/bin/env bash
# Set each version in docker/framework-deps/package.json to the one the
# monorepo's bun.lock resolves, and regenerate docker/framework-deps/bun.lock
# when a version changed. tools/check-image-locks.sh checks the result.
#
# release.sh runs this after it regenerates bun.lock. A release resolves
# third-party packages again, so a new upstream version (loro-crdt 1.17.1 to
# 1.18.0 on v0.22.71) left this manifest behind, CI failed on the release
# commit, and nothing was published.
set -euo pipefail
cd "$(dirname "$0")/.."

manifest=docker/framework-deps/package.json
changed=0

while IFS=$'\t' read -r name version; do
	resolved="$(grep -oE "\"${name//\//\\/}@[0-9][^\"]*\"" bun.lock | sort -u | tr -d '"' || true)"
	count="$(printf '%s\n' "$resolved" | grep -c . || true)"
	if [[ "$count" -ne 1 ]]; then
		echo "sync-image-locks: ${name} resolves to ${count} versions in bun.lock; set ${manifest} by hand" >&2
		exit 1
	fi
	want="${resolved##*@}"
	if [[ "$want" != "$version" ]]; then
		echo "sync-image-locks: ${name} ${version} -> ${want}"
		tmp="$(mktemp)"
		jq --arg n "$name" --arg v "$want" '.dependencies[$n] = $v' "$manifest" >"$tmp"
		mv "$tmp" "$manifest"
		changed=1
	fi
done < <(jq -r '.dependencies | to_entries[] | "\(.key)\t\(.value)"' "$manifest")

if [[ "$changed" -eq 1 ]]; then
	# The image installs this lockfile frozen with the bun its Dockerfile
	# names. Write it with that bun, so the lockfile format matches.
	image_bun="$(grep -oE 'oven/bun:[0-9]+\.[0-9]+\.[0-9]+' Dockerfile | head -1 | cut -d: -f2)"
	if [[ -z "$image_bun" ]]; then
		echo "sync-image-locks: no oven/bun:<version> in Dockerfile" >&2
		exit 1
	fi
	# Outside the workspace, so the install resolves this manifest alone.
	d="$(mktemp -d)"
	cp "$manifest" "$d/package.json"
	(cd "$d" && npx --yes "bun@${image_bun}" install --lockfile-only --silent)
	cp "$d/bun.lock" docker/framework-deps/bun.lock
fi
