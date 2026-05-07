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
// Read from this package's own package.json so the value follows the rest
// of the workspace automatically (release.sh bumps every package.json in
// lockstep). Hard-coding it here was a drift bug we hit historically.
// ---------------------------------------------------------------------------

const PYLON_VERSION = JSON.parse(
	readFileSync(resolve(HERE, "..", "package.json"), "utf8"),
).version;

// ---------------------------------------------------------------------------
// Templates + platforms registry
//
// Each template declares which platforms it supports — `b2b` is web/mac
// only because the demo flow (org switcher, member invite, RBAC admin
// panel) is desktop-shaped and porting it to mobile would be busy work
// without value. Pick a different template if you want mobile.
// ---------------------------------------------------------------------------

const PLATFORMS_AVAILABLE = ["web", "ios", "mac", "expo"];

const TEMPLATE_REGISTRY = {
	barebones: {
		blurb: "Single entity, list + create. The smallest working app.",
		platforms: ["web", "ios", "mac", "expo"],
	},
	todo: {
		blurb: "CRUD + drag-reorder + optimistic mutations.",
		platforms: ["web", "ios", "mac", "expo"],
	},
	b2b: {
		blurb: "Multi-tenant SaaS: orgs, members, roles, RBAC policies.",
		platforms: ["web", "mac"],
	},
	consumer: {
		blurb: "Social feed: profiles, posts, likes, follows.",
		platforms: ["web", "ios", "mac", "expo"],
	},
	chat: {
		blurb: "Realtime messaging: rooms, presence, live message feed.",
		platforms: ["web", "ios", "mac", "expo"],
	},
};
const TEMPLATES_AVAILABLE = Object.keys(TEMPLATE_REGISTRY);

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
	const tmplLines = Object.entries(TEMPLATE_REGISTRY).map(
		([k, v]) => `    ${k.padEnd(10)} ${v.blurb} (${v.platforms.join(", ")})`,
	);
	process.stdout.write(`
Usage: npm create @pylonsync/pylon [name] [options]

  --template <t>         ${TEMPLATES_AVAILABLE.join(" | ")}
${tmplLines.join("\n")}

  --platforms <list>     comma list: ${PLATFORMS_AVAILABLE.join(",")}  (default: web)
  --bun|--pnpm|--yarn|--npm
  --skip-install         scaffold only, don't run install

Examples:
  npm create @pylonsync/pylon my-app
  npm create @pylonsync/pylon my-app --template todo --platforms web,ios
  npm create @pylonsync/pylon my-app --template b2b --platforms web,mac
  npm create @pylonsync/pylon my-app --template chat --platforms ios,mac,expo
`);
	exit(0);
}

const rl = createInterface({ input: stdin, output: stdout });
if (!projectName) {
	projectName = (await rl.question("Project name: ")).trim() || "my-pylon-app";
}
if (!flags.template) {
	const lines = Object.entries(TEMPLATE_REGISTRY)
		.map(([k, v]) => `  ${k.padEnd(10)} ${v.blurb}`)
		.join("\n");
	process.stdout.write(`\n${lines}\n`);
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
	const supported = TEMPLATE_REGISTRY[flags.template].platforms.join(", ");
	const ans = (
		await rl.question(
			`Platforms for ${flags.template} (${supported}, comma-separated) [web]: `,
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

// Reject combos a template doesn't yet support — better to fail loud
// than to scaffold an incomplete tree (e.g. b2b + expo would skip
// frontend entirely and leave the user with a half-empty workspace).
const supportedPlatforms = TEMPLATE_REGISTRY[flags.template].platforms;
const invalidForTemplate = platforms.filter(
	(p) => !supportedPlatforms.includes(p),
);
if (invalidForTemplate.length > 0) {
	console.error(
		`\nError: template "${flags.template}" doesn't support platform(s): ${invalidForTemplate.join(", ")}.\n` +
			`       supported: ${supportedPlatforms.join(", ")}\n`,
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
//   1. _root            — gitignore, env.example, README
//   2. backend/<t>      — apps/api/* always present (one per template)
//   3. ui               — packages/ui (only if web is in platforms)
//   4. <platform>/<t>   — one per requested platform under apps/<platform>/
//   5. Root package.json — generated, not templated
// ---------------------------------------------------------------------------

copyTemplate("_root");
copyTemplate(`backend/${flags.template}`);

if (platforms.includes("web")) {
	copyTemplate("ui");
	copyTemplate(`web/${flags.template}`);
}
for (const p of ["ios", "mac", "expo"]) {
	if (platforms.includes(p)) copyTemplate(`${p}/${flags.template}`);
}

walkAndSubstitute(root);

// ---------------------------------------------------------------------------
// Root package.json — generated based on selected platforms. Workspace
// scripts depend on which apps exist + which package manager the user
// picked (each PM exposes "run X in workspace Y" differently).
// ---------------------------------------------------------------------------

const wsScripts = pmScripts(flags.pm);
const devScripts = {};
// API runs always — every frontend connects to it.
devScripts["dev:api"] = wsScripts.devApi;
if (platforms.includes("web")) devScripts["dev:web"] = wsScripts.devWeb;
if (platforms.includes("expo")) devScripts["dev:expo"] = wsScripts.devExpo;
if (platforms.includes("ios")) {
	// `xcodegen generate` materializes the .xcodeproj from project.yml,
	// then it's an Xcode-driven flow — no `bun run dev` semantics.
	devScripts["dev:ios"] =
		"echo 'cd apps/ios && xcodegen generate && open *.xcodeproj  (or: swift run for a quick macOS preview)'";
}
if (platforms.includes("mac")) {
	devScripts["dev:mac"] =
		"echo 'cd apps/mac && swift run  (or: xcodegen generate && open *.xcodeproj)'";
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
platformLines.push("  → api      http://localhost:4321  (Pylon control plane)");
if (platforms.includes("web"))
	platformLines.push("  → web      http://localhost:3000  (Next.js)");
if (platforms.includes("expo"))
	platformLines.push(`  → expo     ${flags.pm} run dev:expo  (Metro + simulator)`);
if (platforms.includes("ios"))
	platformLines.push(`  → ios      cd apps/ios && xcodegen generate && open *.xcodeproj`);
if (platforms.includes("mac"))
	platformLines.push(`  → mac      cd apps/mac && swift run  (or xcodegen for .app)`);

const layoutLines = ["  apps/api          schema + functions/ handlers"];
if (platforms.includes("web")) {
	layoutLines.push("  apps/web          Next.js 16 + React 19 + Tailwind v4");
	layoutLines.push("  packages/ui       shared UI primitives");
}
if (platforms.includes("ios"))
	layoutLines.push("  apps/ios          Swift / SwiftUI (iOS)");
if (platforms.includes("mac"))
	layoutLines.push("  apps/mac          Swift / SwiftUI (macOS)");
if (platforms.includes("expo"))
	layoutLines.push("  apps/expo         Expo + React Native");

console.log(`
✓ Created ${projectName}

  cd ${projectName}
  ${runDev}

${platformLines.join("\n")}

Layout:
${layoutLines.join("\n")}

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
