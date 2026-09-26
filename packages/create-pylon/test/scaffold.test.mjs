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

function runScaffold({ name, template, cwd, templatesDir, platforms }) {
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
			...(platforms ? ["--platforms", platforms] : []),
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

test("non-interactive run with NO optional flags completes on defaults (never hangs)", () => {
	// #360: the name/template/platforms/pm/skill prompts must not block on a
	// non-TTY stdin. spawnSync gives the child a piped (non-TTY) stdin, so this
	// reproduces CI. Pre-fix it hung on the template prompt forever; the 30s
	// timeout would fire and spawnSync returns signal=SIGTERM, status=null.
	const dir = mkdtempSync(join(tmpdir(), "cp-noninteractive-"));
	const res = spawnSync(
		process.execPath,
		// ONLY the project name — every other choice must fall back to a default.
		[CLI, "myapp", "--skip-install"],
		{ cwd: dir, env: { ...process.env }, encoding: "utf8", input: "", timeout: 30000 },
	);
	assert.equal(
		res.signal,
		null,
		`scaffolder hung on a prompt in non-TTY (killed by timeout): signal=${res.signal}`,
	);
	assert.equal(res.status, 0, `expected clean exit, got ${res.status}\n${res.stderr}`);
	const root = join(dir, "myapp");
	// Defaulted to --template default → a bootable unified app.
	assert.ok(existsSync(join(root, "package.json")), "package.json missing");
	assert.ok(existsSync(join(root, "app.ts")), "app.ts missing (default template)");
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

test("the mobile template scaffolds a backend + an Expo app with the store flow", () => {
	const dir = mkdtempSync(join(tmpdir(), "cp-mobile-"));
	const res = runScaffold({ name: "myapp", template: "mobile", cwd: dir });
	assert.equal(res.status, 0, `expected exit 0, got ${res.status}\n${res.stderr}`);
	const root = join(dir, "myapp");
	// The platform defaults to expo for this template (no --platforms given).
	for (const f of [
		"package.json",
		"apps/api/app.ts",
		"apps/api/functions/createNote.ts",
		"apps/api/functions/deleteMyData.ts",
		"apps/api/lib/site.ts",
		"apps/api/app/(site)/page.tsx",
		"apps/api/app/(site)/privacy/page.tsx",
		"apps/api/app/(site)/terms/page.tsx",
		"apps/api/app/(site)/support/page.tsx",
		"apps/api/functions/revenuecatWebhook.ts",
		"apps/expo/app.config.ts",
		"apps/expo/eas.json",
		"apps/expo/STORE.md",
		"apps/expo/app/_layout.tsx",
		"apps/expo/app/(onboarding)/welcome.tsx",
		"apps/expo/app/(auth)/sign-in.tsx",
		"apps/expo/app/paywall.tsx",
		"apps/expo/app/(tabs)/settings.tsx",
		"apps/expo/assets/icon.png",
	]) {
		assert.ok(existsSync(join(root, f)), `${f} missing`);
	}
	const cfg = readFileSync(join(root, "apps/expo/app.config.ts"), "utf8");
	assert.ok(!cfg.includes("__APP_NAME"), "placeholder left in app.config.ts");
	assert.match(cfg, /com\.example\.myapp/, "bundle id not derived from the app name");
});

test("consumer stays a single SSR app when no platforms are passed", () => {
	const dir = mkdtempSync(join(tmpdir(), "cp-consumer-web-"));
	const res = runScaffold({ name: "myapp", template: "consumer", cwd: dir });
	assert.equal(res.status, 0, `expected exit 0, got ${res.status}\n${res.stderr}`);
	const root = join(dir, "myapp");
	assert.ok(existsSync(join(root, "app.ts")), "app.ts missing");
	assert.ok(!existsSync(join(root, "apps")), "unified scaffold must not create apps/");
});

test("consumer --platforms ios,expo scaffolds the native apps on the shared backend", () => {
	const dir = mkdtempSync(join(tmpdir(), "cp-consumer-native-"));
	const res = runScaffold({ name: "myapp", template: "consumer", cwd: dir, platforms: "ios,expo" });
	assert.equal(res.status, 0, `expected exit 0, got ${res.status}\n${res.stderr}`);
	const root = join(dir, "myapp");
	for (const f of [
		"package.json",
		"apps/api/app.ts",
		"apps/api/functions/createPost.ts",
		"apps/api/functions/seedDemo.ts",
		"apps/api/lib/seed.ts",
		"apps/ios/project.yml",
		"apps/ios/Package.swift",
		"apps/ios/Sources/Myapp",
		"apps/expo/App.tsx",
		"apps/expo/package.json",
	]) {
		assert.ok(existsSync(join(root, f)), `${f} missing`);
	}
	assert.ok(!existsSync(join(root, "app.ts")), "native scaffold must not also land the unified app");
	for (const f of ["apps/ios/project.yml", "apps/expo/app.json", "apps/api/package.json"]) {
		const body = readFileSync(join(root, f), "utf8");
		assert.ok(!body.includes("__APP_NAME"), `placeholder left in ${f}`);
		assert.ok(!body.includes("__PYLON_VERSION"), `version placeholder left in ${f}`);
	}
});

test("consumer rejects a platform it has no native app for", () => {
	const dir = mkdtempSync(join(tmpdir(), "cp-consumer-bad-"));
	const res = runScaffold({ name: "myapp", template: "consumer", cwd: dir, platforms: "web" });
	assert.notEqual(res.status, 0, "expected a non-zero exit");
	assert.match(res.stderr, /doesn't support platform\(s\): web/);
});

test("the imessage template scaffolds the app, both transports, and the relay", () => {
	const dir = mkdtempSync(join(tmpdir(), "cp-imessage-"));
	const res = runScaffold({ name: "textbot", template: "imessage", cwd: dir });
	assert.equal(res.status, 0, `expected exit 0, got ${res.status}\n${res.stderr}`);
	const root = join(dir, "textbot");
	for (const f of [
		"package.json",
		"app.ts",
		".gitignore",
		"bunfig.toml",
		".env.example",
		"app/page.tsx",
		"app/login/page.tsx",
		"app/setup/page.tsx",
		"app/contacts/page.tsx",
		"functions/assistant.ts",
		"functions/sendblueWebhook.ts",
		"functions/relaySync.ts",
		"functions/processTurn.ts",
		"lib/transport.ts",
		"lib/sendblue.ts",
		"lib/relay-protocol.ts",
		"lib/gate.ts",
		"relay/main.ts",
		"relay/chatdb.ts",
		"relay/applescript.ts",
		"relay/.env.example",
		"tests/applescript.test.ts",
		"tests/chatdb.test.ts",
	]) {
		assert.ok(existsSync(join(root, f)), `${f} missing`);
	}
	const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
	assert.equal(pkg.name, "textbot");
	assert.equal(pkg.scripts.relay, "bun relay/main.ts");
	for (const f of ["package.json", "app.ts"]) {
		const body = readFileSync(join(root, f), "utf8");
		assert.ok(!body.includes("__APP_NAME"), `placeholder left in ${f}`);
		assert.ok(!body.includes("__PYLON_VERSION"), `version placeholder left in ${f}`);
	}
	// No secrets ship in the template: every secret in the env examples is blank.
	for (const f of [".env.example", "relay/.env.example"]) {
		const env = readFileSync(join(root, f), "utf8");
		for (const key of ["SENDBLUE_API_SECRET", "SENDBLUE_WEBHOOK_SECRET", "RELAY_TOKEN", "PYLON_ADMIN_TOKEN"]) {
			const line = env.split("\n").find((l) => l.replace(/^#\s*/, "").startsWith(`${key}=`));
			if (line) assert.match(line, new RegExp(`${key}=$`), `${f} ships a value for ${key}`);
		}
	}
});

test("the imessage template is listed in --help", () => {
	const res = spawnSync(process.execPath, [CLI, "--help"], { encoding: "utf8" });
	assert.equal(res.status, 0);
	assert.match(res.stdout, /imessage/);
});
