#!/usr/bin/env bash
# The npm packages the runtime image ships come from docker/*/package.json and
# their committed bun.lock (installed frozen; see the Dockerfile's image-deps
# stage). This keeps those manifests equal to their sources:
#
#   docker/framework-deps   each version equals what the monorepo's bun.lock
#                           resolves for that package
#   docker/example-ui-deps  dependencies equal examples/_shared/package.json's
#
# After changing either source, update the manifest here and regenerate its
# lockfile outside the workspace:
#   d=$(mktemp -d) && cp docker/<set>/package.json "$d" && (cd "$d" && bun install --lockfile-only) \
#     && cp "$d/bun.lock" docker/<set>/bun.lock
set -euo pipefail
cd "$(dirname "$0")/.."

status=0

while IFS=$'\t' read -r name version; do
	if ! grep -Fq "\"${name}@${version}\"" bun.lock; then
		resolved="$(grep -oE "\"${name//\//\\/}@[0-9][^\"]*\"" bun.lock | sort -u | tr '\n' ' ')"
		echo "::error file=docker/framework-deps/package.json::${name} is ${version} here but the monorepo resolves ${resolved:-nothing}"
		status=1
	fi
done < <(jq -r '.dependencies | to_entries[] | "\(.key)\t\(.value)"' docker/framework-deps/package.json)

if ! diff <(jq -S .dependencies examples/_shared/package.json) <(jq -S .dependencies docker/example-ui-deps/package.json) >/dev/null; then
	echo "::error file=docker/example-ui-deps/package.json::dependencies differ from examples/_shared/package.json"
	diff <(jq -S .dependencies examples/_shared/package.json) <(jq -S .dependencies docker/example-ui-deps/package.json) || true
	status=1
fi

for set in framework-deps example-ui-deps opencode; do
	[[ -f "docker/$set/bun.lock" ]] || { echo "::error::docker/$set/bun.lock is missing"; status=1; }
done

exit "$status"
