#!/usr/bin/env node
/**
 * @pylonsync/create-pylon — scaffold a new Pylon app.
 *
 * Run via `npm create @pylonsync/pylon@latest [name]` (or
 * yarn/pnpm/bun create @pylonsync/pylon).
 *
 * Picks one or more platforms (web, mobile, expo) and a template
 * (barebones, todo, …). Each platform shares the same Pylon backend
 * under apps/api so `bun run dev` brings the whole project up.
 *
 * Templates live as real files under ../templates/<scope>/<template>.
 * The scaffolder walks each requested template dir, substitutes
 * placeholders, and writes the result. Keeping them on disk (instead
 * of as inline strings in this file) is what stopped 0.3.50's tab-
 * mangling regression class — there is no JS template-literal layer
 * to corrupt.
 *
 * Node-runnable, no Bun required. Uses only Node-builtin APIs (no
 * runtime deps): npm/yarn/pnpm/bun's `create` runners just need a
 * working node binary.
 */

import {
	cpSync,
	existsSync,
	mkdirSync,
	readFileSync,
	readdirSync,
	statSync,
	writeFileSync,
	renameSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline/promises";
import { stdin, stdout, exit, argv, cwd } from "node:process";

// ---------------------------------------------------------------------------
// Locate templates relative to this script (works whether installed via
// npm, run from a clone, or invoked through `npx -p @pylonsync/create-pylon`).
// ---------------------------------------------------------------------------

const HERE = dirname(fileURLToPath(import.meta.url));
const TEMPLATES = resolve(HERE, "..", "templates");

// ---------------------------------------------------------------------------
// Version pin — every generated dep references this version of @pylonsync/*.
// Bumped via the workspace's release-please flow (same version as the rest
// of the pylon stack).
// ---------------------------------------------------------------------------

const PYLON_VERSION = "0.3.51";

// ---------------------------------------------------------------------------
// Templates + platforms registry
// ---------------------------------------------------------------------------

const TEMPLATES_AVAILABLE = ["barebones", "todo"];
const PLATFORMS_AVAILABLE = ["web", "mobile", "expo"];

// ---------------------------------------------------------------------------
// CLI args + interactive prompts
// ---------------------------------------------------------------------------

const args = argv.slice(2);
let projectName = args.find((a) => !a.startsWith("--"));

const flags = {
	pm: pickValue(args, "--bun", "--pnpm", "--yarn", "--npm")?.replace(/^--/, ""),
	template: takeValue(args, "--template"),
	platforms: takeValue(args, "--platforms"),
	skipInstall: args.includes("--skip-install"),
	help: args.includes("--help") || args.includes("-h"),
};

function takeValue(arr, name) {
	const flagWithEq = arr.find((a) => a.startsWith(name + "="));
	if (flagWithEq) return flagWithEq.slice(name.length + 1);
	const idx = arr.indexOf(name);
	if (idx >= 0 && idx + 1 < arr.length && !arr[idx + 1].startsWith("--")) {
		return arr[idx + 1];
	}
	return undefined;
}

function pickValue(arr, ...candidates) {
	for (const c of candidates) {
		if (arr.includes(c)) return c;
	}
	return undefined;
}

if (flags.help) {
	process.stdout.write(`
Usage: npm create @pylonsync/pylon [name] [options]

  --template <t>         barebones | todo
  --platforms <list>     comma list: web,mobile,expo  (default: web)
  --bun|--pnpm|--yarn|--npm
  --skip-install         scaffold only, don't run install

Examples:
  npm create @pylonsync/pylon my-app
  npm create @pylonsync/pylon my-app --template todo --platforms web,mobile
  npm create @pylonsync/pylon my-app --template barebones --platforms expo --bun
`);
	exit(0);
}

const rl = createInterface({ input: stdin, output: stdout });
if (!projectName) {
	projectName = (await rl.question("Project name: ")).trim() || "my-pylon-app";
}
if (!flags.template) {
	const ans = (
		await rl.question(
			`Template (${TEMPLATES_AVAILABLE.join(", ")}) [todo]: `,
		)
	)
		.trim()
		.toLowerCase();
	flags.template = TEMPLATES_AVAILABLE.includes(ans) ? ans : "todo";
}
if (!flags.platforms) {
	const ans = (
		await rl.question(
			`Platforms (${PLATFORMS_AVAILABLE.join(", ")}, comma-separated) [web]: `,
		)
	).trim();
	flags.platforms = ans || "web";
}
if (!flags.pm) {
	const detected = detectPackageManager();
	const def = detected ?? "bun";
	const choice = (
		await rl.question(`Package manager (bun, pnpm, yarn, npm) [${def}]: `)
	)
		.trim()
		.toLowerCase();
	flags.pm = ["bun", "pnpm", "yarn", "npm"].includes(choice) ? choice : def;
}
rl.close();

const platforms = flags.platforms
	.split(",")
	.map((p) => p.trim().toLowerCase())
	.filter(Boolean);
const unknownPlatforms = platforms.filter(
	(p) => !PLATFORMS_AVAILABLE.includes(p),
);
if (unknownPlatforms.length > 0) {
	console.error(
		`\nError: unknown platform(s): ${unknownPlatforms.join(", ")}. Valid: ${PLATFORMS_AVAILABLE.join(", ")}\n`,
	);
	exit(1);
}
if (platforms.length === 0) {
	console.error(`\nError: at least one platform required.\n`);
	exit(1);
}
if (!TEMPLATES_AVAILABLE.includes(flags.template)) {
	console.error(
		`\nError: unknown template "${flags.template}". Valid: ${TEMPLATES_AVAILABLE.join(", ")}\n`,
	);
	exit(1);
}

// Some PMs reject the `workspace:` protocol. Bun/pnpm/yarn berry
// understand it and rewrite to the local sibling version at install
// time. npm errors EUNSUPPORTEDPROTOCOL ("Unsupported URL Type").
// For npm, emit "*" — npm's own workspaces feature still resolves
// it to the local sibling because the workspace package is in the
// root's `workspaces` list.
const usesWorkspaceProtocol = flags.pm !== "npm";
const workspaceDepSpec = usesWorkspaceProtocol ? "workspace:*" : "*";

const root = resolve(cwd(), projectName);
if (existsSync(root) && readdirSync(root).length > 0) {
	console.error(`\nError: ${root} already exists and is not empty.\n`);
	exit(1);
}
mkdirSync(root, { recursive: true });

console.log(
	`\nCreating ${projectName} (${flags.template}, ${platforms.join(" + ")}) in ${root}\n`,
);

// ---------------------------------------------------------------------------
// Substitution table — used by every template file copy. Names that
// only make sense for some platforms (e.g. PASCAL for Swift) are still
// computed unconditionally; the unused replacements are no-ops.
// ---------------------------------------------------------------------------

const APP_NAME = projectName;
const APP_NAME_KEBAB = APP_NAME.replace(/[^a-z0-9]+/gi, "-").toLowerCase();
const APP_NAME_SNAKE = APP_NAME.replace(/[^a-z0-9]+/gi, "_").toLowerCase();
const APP_NAME_PASCAL = APP_NAME.replace(/(^|[^a-z0-9])([a-z0-9])/gi, (_, _s, c) =>
	c.toUpperCase(),
).replace(/[^A-Za-z0-9]/g, "");

const SUBS = {
	__APP_NAME__: APP_NAME,
	__APP_NAME_KEBAB__: APP_NAME_KEBAB,
	__APP_NAME_SNAKE__: APP_NAME_SNAKE,
	__APP_NAME_PASCAL__: APP_NAME_PASCAL,
	__PYLON_VERSION__: PYLON_VERSION,
	__WORKSPACE_DEP__: workspaceDepSpec,
};

// Filenames that contain placeholders get renamed AFTER copy. Keeps
// the loader simple — copy the directory tree raw, then rename any
// file/dir whose name has a placeholder.
function substituteString(s) {
	let out = s;
	for (const [k, v] of Object.entries(SUBS)) {
		out = out.split(k).join(v);
	}
	return out;
}

function substituteFile(absPath) {
	// Skip binary-ish files — we only ever ship text in templates,
	// but be safe in case someone drops a PNG icon in later.
	const buf = readFileSync(absPath);
	for (let i = 0; i < Math.min(buf.length, 8000); i++) {
		if (buf[i] === 0) return;
	}
	const before = buf.toString("utf8");
	const after = substituteString(before);
	if (before !== after) writeFileSync(absPath, after);
}

function walkAndSubstitute(dir) {
	for (const entry of readdirSync(dir)) {
		const abs = join(dir, entry);
		const renamed = substituteString(entry);
		let target = abs;
		if (renamed !== entry) {
			target = join(dir, renamed);
			renameSync(abs, target);
		}
		const st = statSync(target);
		if (st.isDirectory()) walkAndSubstitute(target);
		else if (st.isFile()) substituteFile(target);
	}
}

function copyTemplate(srcSubpath, destSubpath = "") {
	const src = join(TEMPLATES, srcSubpath);
	if (!existsSync(src)) return false;
	const dest = destSubpath ? join(root, destSubpath) : root;
	mkdirSync(dest, { recursive: true });
	cpSync(src, dest, { recursive: true });
	return true;
}

// ---------------------------------------------------------------------------
// Apply templates in order:
//   1. _root/_shared       — gitignore, env.example, basic README
//   2. backend/<template>  — apps/api/* always present
//   3. ui                  — packages/ui (only if web is in platforms)
//   4. web/<template>      — apps/web/* (only if web in platforms)
//   5. mobile/<template>   — apps/mobile/* (only if mobile in platforms)
//   6. expo/<template>     — apps/expo/* (only if expo in platforms)
//   7. Root package.json   — generated, not templated; depends on platforms
// ---------------------------------------------------------------------------

copyTemplate("_root");
copyTemplate(`backend/${flags.template}`);

if (platforms.includes("web")) {
	copyTemplate("ui");
	copyTemplate(`web/${flags.template}`);
}
if (platforms.includes("mobile")) {
	copyTemplate(`mobile/${flags.template}`);
}
if (platforms.includes("expo")) {
	copyTemplate(`expo/${flags.template}`);
}

walkAndSubstitute(root);

// ---------------------------------------------------------------------------
// Root package.json — generated based on selected platforms. Workspace
// scripts depend on which apps exist + which package manager the user
// picked (each PM exposes "run X in workspace Y" differently).
// ---------------------------------------------------------------------------

const wsScripts = pmScripts(flags.pm);
const devScripts = {};
const buildScripts = {};
if (platforms.includes("web")) {
	devScripts["dev:api"] = wsScripts.devApi;
	devScripts["dev:web"] = wsScripts.devWeb;
}
if (!platforms.includes("web")) {
	// API still runs even without a web platform — mobile / expo connect
	// to it directly.
	devScripts["dev:api"] = wsScripts.devApi;
}
if (platforms.includes("expo")) {
	devScripts["dev:expo"] = wsScripts.devExpo;
}
if (platforms.includes("mobile")) {
	// Swift/iOS isn't a `bun run dev` thing — surfaced as a separate
	// script invocation since `swift run` blocks and Xcode is out-of-band.
	devScripts["dev:mobile"] = "echo 'Open apps/mobile in Xcode (or run: cd apps/mobile && swift run)'";
}

const parallelDevs = Object.keys(devScripts);
const rootPkg = {
	name: APP_NAME_KEBAB,
	private: true,
	type: "module",
	workspaces: ["apps/*", "packages/*"].filter((p) => {
		// Only declare packages/* as a workspace if we actually scaffolded
		// packages/ui — otherwise the empty match warns on bun install.
		if (p === "packages/*") return platforms.includes("web");
		return true;
	}),
	scripts: {
		dev:
			parallelDevs.length > 1
				? `npm-run-all --parallel ${parallelDevs.join(" ")}`
				: wsScripts.devApi,
		...devScripts,
		build: wsScripts.build,
	},
	devDependencies: parallelDevs.length > 1 ? { "npm-run-all": "^4.1.5" } : {},
};
writeFileSync(
	join(root, "package.json"),
	JSON.stringify(rootPkg, null, 2) + "\n",
);

// ---------------------------------------------------------------------------
// Optional: install dependencies
// ---------------------------------------------------------------------------

if (!flags.skipInstall) {
	console.log(`Installing dependencies with ${flags.pm}...`);
	const { spawnSync } = await import("node:child_process");
	const result = spawnSync(flags.pm, ["install"], {
		cwd: root,
		stdio: "inherit",
	});
	if (result.status !== 0) {
		console.warn(
			`\n${flags.pm} install exited with code ${result.status}. Re-run from ${projectName}/.\n`,
		);
	}
}

// ---------------------------------------------------------------------------
// Final instructions
// ---------------------------------------------------------------------------

const runDev = flags.pm === "npm" ? "npm run dev" : `${flags.pm} run dev`;

const platformLines = [];
if (platforms.includes("web"))
	platformLines.push("  → web      http://localhost:3000  (Next.js)");
platformLines.push("  → api      http://localhost:4321  (Pylon control plane)");
if (platforms.includes("expo"))
	platformLines.push(`  → expo     ${flags.pm} run dev:expo  (Metro + simulator)`);
if (platforms.includes("mobile"))
	platformLines.push(`  → mobile   open apps/mobile in Xcode  (or: swift run)`);

console.log(`
✓ Created ${projectName}

  cd ${projectName}
  ${runDev}

${platformLines.join("\n")}

Layout:
  apps/api          schema + functions/ handlers
${platforms.includes("web") ? "  apps/web          Next.js 16 + React 19 + Tailwind v4\n  packages/ui       shared UI primitives" : ""}
${platforms.includes("mobile") ? "  apps/mobile       Swift / SwiftUI" : ""}
${platforms.includes("expo") ? "  apps/expo         Expo + React Native" : ""}

Docs: https://pylonsync.com/docs
`);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function detectPackageManager() {
	const ua = process.env.npm_config_user_agent ?? "";
	if (ua.startsWith("bun")) return "bun";
	if (ua.startsWith("pnpm")) return "pnpm";
	if (ua.startsWith("yarn")) return "yarn";
	if (ua.startsWith("npm")) return "npm";
	return null;
}

function pmScripts(pm) {
	switch (pm) {
		case "bun":
			return {
				devApi: "bun run --filter './apps/api' dev",
				devWeb: "bun run --filter './apps/web' dev",
				devExpo: "bun run --filter './apps/expo' start",
				build: "bun run --filter '*' build",
			};
		case "pnpm":
			return {
				devApi: "pnpm --filter './apps/api' run dev",
				devWeb: "pnpm --filter './apps/web' run dev",
				devExpo: "pnpm --filter './apps/expo' run start",
				build: "pnpm --filter '*' run build",
			};
		case "yarn":
			return {
				devApi: `yarn workspace @${APP_NAME_KEBAB}/api run dev`,
				devWeb: `yarn workspace @${APP_NAME_KEBAB}/web run dev`,
				devExpo: `yarn workspace @${APP_NAME_KEBAB}/expo run start`,
				build: "yarn workspaces foreach -A run build",
			};
		case "npm":
		default:
			return {
				devApi: "npm --workspace apps/api run dev",
				devWeb: "npm --workspace apps/web run dev",
				devExpo: "npm --workspace apps/expo run start",
				build: "npm --workspaces run build --if-present",
			};
	}
}
