#!/usr/bin/env bash
# Verifies that the files vendored into crates/cli/{templates,embedded}
# match the canonical originals at /templates and /packages/sdk. The
# vendored copies exist because `cargo publish` packages each crate in
# isolation and can't reach include_str! paths outside the crate root.
#
# If this script fails, copy the canonical files into the cli crate:
#
#   cp templates/basic/app.ts        crates/cli/templates/basic/app.ts
#   cp templates/basic/tsconfig.json crates/cli/templates/basic/tsconfig.json
#   cp packages/sdk/src/index.ts     crates/cli/embedded/sdk-index.ts
#
# Then commit. The drift would otherwise mean `pylon init` ships an
# outdated app.ts / tsconfig.json / SDK source to new users.
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

if [ "$fail" -ne 0 ]; then
  echo
  echo "Run the cp commands at the top of this script, then re-commit."
  exit 1
fi
echo "OK: cli vendored copies match the canonical originals."
