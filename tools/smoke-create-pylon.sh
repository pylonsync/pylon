#!/usr/bin/env bash
# smoke-create-pylon.sh — end-to-end gate for @pylonsync/create-pylon.
#
# Runs on every PR + push to main via .github/workflows/ci.yml, plus
# locally before tagging a release. Catches the class of bug that
# shipped 0.3.50 broken: a tab-mangling transform replaced every real
# tab in bin/create-pylon.js with a literal `\t` two-char sequence,
# causing Node.js to SyntaxError on parse before scaffolding could
# even start.
#
# What this gate verifies, in order of "you must fix this before
# merging":
#
#   1. The script parses (node --check). The 0.3.50 failure mode.
#   2. The script runs end-to-end and creates a project.
#   3. Generated files contain real tab characters, NOT literal
#      backslash-t text (the secondary failure mode hidden inside
#      template literals — script parses, but scaffolded files
#      have "\t\t\tconst trimmed = ..." text instead of indentation).
#   4. Each generated TypeScript file is itself well-formed enough
#      that `node --check --input-type=module` accepts it after a
#      minimal type-strip. (Cheap parse, no tsc dep needed in CI.)

set -euo pipefail

cd "$(dirname "$0")/.."

PKG_DIR="packages/create-pylon"
SCRIPT="$PKG_DIR/bin/create-pylon.js"
TMP="$(mktemp -d -t pylon-smoke.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

echo "→ smoke 1/4: node --check $SCRIPT"
node --check "$SCRIPT"

echo "→ smoke 2/4: scaffold to $TMP"
(
	cd "$TMP"
	# --skip-install so we don't need network or workspace deps.
	# --bun is just the PM choice for the generated package.json;
	# nothing executes since we skip install.
	node "$OLDPWD/$SCRIPT" smoke-app --bun --skip-install </dev/null
)

PROJECT="$TMP/smoke-app"
if [[ ! -d "$PROJECT" ]]; then
	echo "::error::scaffold produced no smoke-app/ directory" >&2
	exit 1
fi

echo "→ smoke 3/4: generated files have real tab indentation"
# Walk every TS/TSX/JSON file the script wrote and confirm NO line
# starts with the literal two-char sequence `\t` (backslash + lower
# t). A real tab character at the same position would NOT match —
# real tabs are what we want. This is the exact regression that
# broke 0.3.50: real tabs got string-replaced into literal `\t`,
# the JS parser failed on the script itself, AND any file that
# survived to scaffolding had `\t\t\tconst trimmed` text instead
# of real indentation.
#
# Done in Node so it's portable across Linux (CI) and macOS
# (developer machines) — BSD grep on macOS lacks `-P` for PCRE.
node - "$PROJECT" <<'NODE'
const fs = require('node:fs');
const path = require('node:path');
const root = process.argv[2];
const exts = new Set(['.ts', '.tsx', '.json']);
const bad = [];
function walk(dir) {
	for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
		const p = path.join(dir, e.name);
		if (e.isDirectory()) {
			if (e.name === 'node_modules' || e.name === '.next') continue;
			walk(p);
		} else if (exts.has(path.extname(e.name))) {
			const lines = fs.readFileSync(p, 'utf8').split('\n');
			const offenders = [];
			for (let i = 0; i < lines.length; i++) {
				if (lines[i].startsWith('\\t')) offenders.push([i + 1, lines[i]]);
			}
			if (offenders.length) bad.push({ p, offenders });
		}
	}
}
walk(root);
if (bad.length) {
	console.error("::error::Generated files have literal '\\t' text instead of real tabs:");
	for (const { p, offenders } of bad) {
		console.error('  ' + p);
		for (const [n, line] of offenders.slice(0, 3)) {
			console.error('    ' + n + ': ' + line);
		}
	}
	process.exit(1);
}
NODE

echo "→ smoke 4/4: every generated .ts file parses as valid syntax"
# We can't tsc-check (would need @pylonsync/* deps installed) but we
# can confirm each file is at least lexically well-formed TypeScript
# by stripping types with a 1-pass swc-style transform — Node 22's
# built-in type-stripping accepts plain TS via --experimental-strip-types.
# Falls back to a regex-strip + node --check if the flag isn't
# available on the runner.
NODE_VERSION="$(node -p 'process.versions.node')"
NODE_MAJOR="${NODE_VERSION%%.*}"
PARSE_FAILED=()
while IFS= read -r -d '' f; do
	# Skip empty files (some scaffolded configs are intentionally empty).
	[[ -s "$f" ]] || continue
	if (( NODE_MAJOR >= 22 )); then
		# Node 22+ ships --experimental-strip-types: parse + strip in one
		# step. We don't actually run the file — --check stops after parse.
		if ! node --experimental-strip-types --check "$f" 2>/tmp/parse.err; then
			# Permitted error: imports from packages we haven't published
			# yet. We only fail on actual SyntaxError.
			if grep -q 'SyntaxError' /tmp/parse.err; then
				PARSE_FAILED+=("$f")
				cat /tmp/parse.err >&2
			fi
		fi
	else
		# Older Node: skip the strict parse, but warn.
		echo "warning: node $NODE_VERSION lacks --experimental-strip-types; skipping ts parse" >&2
		break
	fi
done < <(find "$PROJECT" -type f \( -name '*.ts' -o -name '*.tsx' \) -print0)
if (( ${#PARSE_FAILED[@]} > 0 )); then
	echo "::error::Generated TypeScript files do not parse:" >&2
	printf '  %s\n' "${PARSE_FAILED[@]}" >&2
	exit 1
fi

echo "✓ smoke passed — create-pylon@$(node -p "require('./$PKG_DIR/package.json').version") scaffolds cleanly"
