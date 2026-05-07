#!/usr/bin/env bash
# smoke-create-pylon.sh — end-to-end gate for @pylonsync/create-pylon.
#
# Runs on every PR + push to main via .github/workflows/ci.yml, plus
# locally before tagging a release.
#
# What this gate verifies, for EVERY (template × platform-set) combo:
#
#   1. The script parses (node --check). Catches the 0.3.50 failure mode
#      where real tabs in bin/create-pylon.js had been string-replaced
#      into literal `\t` two-char sequences and Node SyntaxError'd.
#   2. The script runs end-to-end and creates a project tree.
#   3. No generated text file has unsubstituted `__APP_NAME__` etc
#      placeholders left behind. A scaffolded project that ships with
#      `import "@__APP_NAME_KEBAB__/ui"` text is just as broken as a
#      script that won't parse.
#   4. No generated file's line begins with the literal two-char
#      sequence `\t` (backslash + lower t) — that's the secondary
#      "tabs got escaped to text" failure mode.
#   5. Generated TS/TSX files parse as valid TypeScript (Node 22+
#      `--experimental-strip-types --check`). Caught a missing comma
#      / mismatched brace / unterminated template literal in the
#      template tree before users hit it.
#   6. Generated Swift files don't have stray `__APP_NAME` placeholders
#      and have real-tab indentation. (We don't `swift build` in CI
#      because it's slow and Linux runners lack the toolchain we want
#      to support; the placeholder + tab gates catch the same class
#      of bug.)
#
# Each combo is scaffolded into its own tempdir so failures isolate.

set -euo pipefail

cd "$(dirname "$0")/.."

PKG_DIR="packages/create-pylon"
SCRIPT="$PKG_DIR/bin/create-pylon.js"
TMP="$(mktemp -d -t pylon-smoke.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

echo "→ smoke 1: node --check $SCRIPT"
node --check "$SCRIPT"

# Combos to scaffold + verify. Add a row when you add a template or
# platform — the loop body covers everything else.
COMBOS=(
	"barebones|web"
	"barebones|mobile"
	"barebones|expo"
	"barebones|web,mobile,expo"
	"todo|web"
	"todo|mobile"
	"todo|expo"
	"todo|web,mobile,expo"
)

NODE_VERSION="$(node -p 'process.versions.node')"
NODE_MAJOR="${NODE_VERSION%%.*}"

for combo in "${COMBOS[@]}"; do
	template="${combo%%|*}"
	platforms="${combo#*|}"
	slug="$(echo "$combo" | tr '|,' '__')"
	project_dir="$TMP/$slug"
	mkdir -p "$project_dir"

	echo
	echo "→ smoke combo: template=$template platforms=$platforms"
	(
		cd "$project_dir"
		node "$OLDPWD/$SCRIPT" smoke-app \
			--bun --skip-install \
			--template "$template" \
			--platforms "$platforms" </dev/null > "$project_dir/scaffold.log" 2>&1
	) || {
		echo "::error::scaffold failed for $combo"
		cat "$project_dir/scaffold.log" >&2
		exit 1
	}

	project="$project_dir/smoke-app"
	if [[ ! -d "$project" ]]; then
		echo "::error::scaffold produced no smoke-app/ directory for $combo" >&2
		exit 1
	fi

	# Gates 3 + 4: no leftover placeholders, no literal `\t` line-starts.
	# Done in Node for portability across Linux/macOS — BSD grep lacks -P.
	node - "$project" <<'NODE'
const fs = require('node:fs');
const path = require('node:path');
const root = process.argv[2];
const TEXT_EXTS = new Set([
	'.ts', '.tsx', '.js', '.jsx', '.json', '.swift', '.yml', '.yaml',
	'.md', '.css', '.mjs', '.cjs',
]);
const PLACEHOLDERS = [
	'__APP_NAME__',
	'__APP_NAME_KEBAB__',
	'__APP_NAME_SNAKE__',
	'__APP_NAME_PASCAL__',
	'__PYLON_VERSION__',
	'__WORKSPACE_DEP__',
];
const placeholderHits = [];
const tabTextHits = [];

function walk(dir) {
	for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
		if (e.name === 'node_modules' || e.name === '.next' || e.name === '.build') continue;
		const p = path.join(dir, e.name);
		if (e.isDirectory()) walk(p);
		else if (e.isFile() && TEXT_EXTS.has(path.extname(e.name))) {
			let text;
			try { text = fs.readFileSync(p, 'utf8'); } catch { continue; }
			for (const ph of PLACEHOLDERS) {
				if (text.includes(ph)) {
					placeholderHits.push([p, ph]);
					break;
				}
			}
			const lines = text.split('\n');
			for (let i = 0; i < lines.length; i++) {
				if (lines[i].startsWith('\\t')) {
					tabTextHits.push([p, i + 1, lines[i]]);
					break;
				}
			}
		}
	}
}
walk(root);

let failed = false;
if (placeholderHits.length) {
	failed = true;
	console.error("::error::Unsubstituted placeholders in generated files:");
	for (const [p, ph] of placeholderHits.slice(0, 5)) {
		console.error('  ' + ph + ' in ' + p);
	}
}
if (tabTextHits.length) {
	failed = true;
	console.error("::error::Generated files have literal '\\t' text instead of real tabs:");
	for (const [p, n, line] of tabTextHits.slice(0, 5)) {
		console.error('  ' + p + ':' + n + ': ' + line);
	}
}
if (failed) process.exit(1);
NODE

	# Gate 5: TS/TSX files parse. Skip on Node <22 (no strip-types).
	if (( NODE_MAJOR >= 22 )); then
		while IFS= read -r -d '' f; do
			[[ -s "$f" ]] || continue
			if ! node --experimental-strip-types --check "$f" 2>/tmp/parse.err; then
				if grep -q 'SyntaxError' /tmp/parse.err; then
					echo "::error::TS parse failed in $combo: $f" >&2
					cat /tmp/parse.err >&2
					exit 1
				fi
			fi
		done < <(find "$project" -type f \( -name '*.ts' -o -name '*.tsx' \) ! -path '*/node_modules/*' -print0)
	fi
done

echo
echo "✓ smoke passed for ${#COMBOS[@]} combos · create-pylon@$(node -p "require('./$PKG_DIR/package.json').version") scaffolds cleanly"
