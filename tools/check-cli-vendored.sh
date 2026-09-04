#!/usr/bin/env bash
# Verifies that copies of a file that must stay identical actually are.
#
# Two sets:
#
# 1. crates/cli/{templates,embedded} against /templates and /packages/sdk.
#    The vendored copies exist because `cargo publish` packages each crate
#    in isolation and can't reach include_str! paths outside the crate
#    root. Drift means `pylon init` ships an outdated app.ts / tsconfig.json
#    / SDK source to new users.
#
# 2. apps/site/public against /scripts. The installers are served from the
#    site, and /scripts holds the canonical copy the README points people
#    at. These had already drifted once: the served install.sh still told
#    Windows users to run `cargo install pylon-cli`, which cannot work,
#    long after the repo copy stopped saying so.
#
# If this script fails, copy the canonical file over its stale copy:
#
#   cp templates/basic/app.ts        crates/cli/templates/basic/app.ts
#   cp templates/basic/tsconfig.json crates/cli/templates/basic/tsconfig.json
#   cp packages/sdk/src/index.ts     crates/cli/embedded/sdk-index.ts
#   cp scripts/install.sh            apps/site/public/install.sh
#   cp scripts/install.ps1           apps/site/public/install.ps1
#
# Then commit.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fail=0

check() {
  local src="$1" vendored="$2"
  if ! diff -q "$ROOT/$src" "$ROOT/$vendored" >/dev/null 2>&1; then
    echo "DRIFT: $vendored is out of sync with $src"
    diff -u "$ROOT/$src" "$ROOT/$vendored" || true
    fail=1
  fi
}

check templates/basic/app.ts        crates/cli/templates/basic/app.ts
check templates/basic/tsconfig.json crates/cli/templates/basic/tsconfig.json
check packages/sdk/src/index.ts     crates/cli/embedded/sdk-index.ts

check scripts/install.sh            apps/site/public/install.sh
check scripts/install.ps1           apps/site/public/install.ps1

if [ "$fail" -ne 0 ]; then
  echo
  echo "Run the cp commands at the top of this script, then re-commit."
  exit 1
fi
echo "OK: every vendored copy matches its canonical original."
