#!/usr/bin/env bash
# Verifies that copies of a file that must stay identical actually are.
#
# apps/site/public holds the installers the site serves. /scripts holds the
# canonical copy the README points people at. A stale served copy gives
# people install steps that no longer work.
#
# If this script fails, copy the canonical file over its stale copy:
#
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

check scripts/install.sh            apps/site/public/install.sh
check scripts/install.ps1           apps/site/public/install.ps1

if [ "$fail" -ne 0 ]; then
  echo
  echo "Run the cp commands at the top of this script, then re-commit."
  exit 1
fi
echo "OK: every vendored copy matches its canonical original."
