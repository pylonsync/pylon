// create-pylon scaffolder tests. Node's built-in runner only — this package
// ships zero runtime/dev deps on purpose (it's invoked via `npm create`, which
// only guarantees a node binary). Run with `node --test`.
//
// Focus: the missing-template guard (#359). copyTemplate() returns false when a
// template dir isn't on disk; the unified scaffold path used to IGNORE that and
// still print "✓ Created", handing the user an empty project + a dead
// `pylon dev`. mustCopy() now fails loud. These tests pin both halves: a real
// template scaffolds a bootable tree, and a missing template exits non-zero
// without claiming success.

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
	mkdtempSync,
	existsSync,
	readFileSync,
	readdirSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const CLI = resolve(HERE, "..", "bin", "create-pylon.js");

function runScaffold({ name, template, cwd, templatesDir }) {
	// Pass EVERY flag the scaffolder would otherwise prompt for (name positional,
	// --template, --bun for the package manager, --no-skill) so the run is fully
	// non-interactive; --skip-install keeps it offline. input:"" is a belt-and-
	// suspenders EOF in case a prompt branch is ever added.
	const env = { ...process.env };
	if (templatesDir) env.PYLON_CREATE_TEMPLATES_DIR = templatesDir;
	return spawnSync(
		process.execPath,
		[
			CLI,
			name,
			"--template",
			template,
			"--bun",
			"--skip-install",
			"--no-skill",
		],
		{ cwd, env, encoding: "utf8", input: "" },
	);
}

test("scaffolds a real unified template into a bootable tree", () => {
	const dir = mkdtempSync(join(tmpdir(), "cp-ok-"));
	const res = runScaffold({ name: "myapp", template: "barebones", cwd: dir });
	assert.equal(res.status, 0, `expected exit 0, got ${res.status}\n${res.stderr}`);
	const root = join(dir, "myapp");
	// A unified template MUST land app.ts + package.json + an app/ route dir —
	// the three things `pylon dev` needs to boot. An empty scaffold has none.
	assert.ok(existsSync(join(root, "package.json")), "package.json missing");
	assert.ok(existsSync(join(root, "app.ts")), "app.ts missing");
	assert.ok(existsSync(join(root, "app")), "app/ dir missing");
	// gitignore must be restored to its dotted name (npm strips literal
	// .gitignore from tarballs; the scaffolder renames `gitignore` -> `.gitignore`).
	assert.ok(existsSync(join(root, ".gitignore")), ".gitignore not restored");
	// Placeholder substitution ran: no raw __APP_NAME__ tokens survive.
	const pkg = readFileSync(join(root, "package.json"), "utf8");
	assert.ok(!pkg.includes("__APP_NAME__"), "placeholder left unsubstituted");
});

test("missing template dir fails LOUD instead of scaffolding an empty project", () => {
	// Point the scaffolder at an empty templates dir so `barebones` (a valid
	// registry name) has no files on disk — the exact shape of a tarball
	// publish-drop / corrupt install.
	const emptyTemplates = mkdtempSync(join(tmpdir(), "cp-empty-templates-"));
	const dir = mkdtempSync(join(tmpdir(), "cp-miss-"));
	const res = runScaffold({
		name: "broken",
		template: "barebones",
		cwd: dir,
		templatesDir: emptyTemplates,
	});
	assert.notEqual(res.status, 0, "missing template must exit non-zero");
	const out = (res.stdout || "") + (res.stderr || "");
	assert.match(out, /missing from this install/, "no actionable error printed");
	// The cardinal sin this guards: NEVER claim success on an empty scaffold.
	assert.doesNotMatch(out, /✓ Created/, 'printed "✓ Created" for an empty scaffold');
	// And it must not leave a half-written project tree the user might run.
	const root = join(dir, "broken");
	if (existsSync(root)) {
		assert.ok(
			!existsSync(join(root, "package.json")),
			"left a package.json behind for a failed scaffold",
		);
		// At most an empty dir; certainly no bootable files.
		assert.equal(
			readdirSync(root).length,
			0,
			"failed scaffold left files behind",
		);
	}
});
