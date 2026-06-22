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
// PYLON_CREATE_TEMPLATES_DIR overrides the template source dir. Only used by
// the scaffolder's own tests (to point at a fixture / empty dir and exercise
// the missing-template guard); undocumented + irrelevant in normal use.
const TEMPLATES = process.env.PYLON_CREATE_TEMPLATES_DIR
	? resolve(process.env.PYLON_CREATE_TEMPLATES_DIR)
	: resolve(HERE, "..", "templates");

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
// Each template declares which platforms it supports. Unified templates
// (the default and every archetype) are a single full-stack SSR app and
// take no platforms at all. The few non-unified monorepo templates list
// the platforms whose demo flow they actually ship.
// ---------------------------------------------------------------------------

// `web` and `vite` both render into apps/web — they're mutually
// exclusive web-frontend toolchains. `web` is Next.js 16 (SSR +
// server actions); `vite` is plain Vite + React for an SPA setup.
// Validation below rejects passing both.
const PLATFORMS_AVAILABLE = ["web", "vite", "ios", "mac", "expo"];

const TEMPLATE_REGISTRY = {
	default: {
		blurb:
			"SaaS starter — server-rendered marketing landing + multi-tenant dashboard (orgs, members, tenant-scoped data). One app, one port, no Next.js.",
		// `unified` templates are a single Pylon app (app.ts + app/ routes +
		// functions/), NOT a monorepo of apps/api + apps/web. `pylon dev`
		// serves the SSR frontend and the API from one port. They take no
		// `--platforms` — the app IS the whole stack.
		platforms: [],
		unified: true,
	},
	barebones: {
		blurb:
			"Single entity, live list + optimistic create. The smallest SSR app, one port.",
		platforms: [],
		unified: true,
	},
	todo: {
		blurb:
			"Live, optimistic todo list — guest auth, owner-scoped, one server, no Next.js.",
		platforms: [],
		unified: true,
	},
	consumer: {
		blurb:
			"Social feed — live posts + likes, public-read, owner-write. One SSR app.",
		platforms: [],
		unified: true,
	},
	chat: {
		blurb:
			"Realtime chat — a live shared room, optimistic send. One SSR app, one port.",
		platforms: [],
		unified: true,
	},
	waitlist: {
		blurb:
			"Coming-soon landing — email capture + a LIVE signup counter (two tabs, no refresh) + owner dashboard. One SSR app.",
		platforms: [],
		unified: true,
	},
	"local-service": {
		blurb:
			"Appointment business — services + booking with LIVE slot availability (a slot greys out for everyone the instant it's booked) + owner dashboard. One SSR app.",
		platforms: [],
		unified: true,
	},
	saas: {
		blurb:
			"SaaS app — marketing landing + multi-tenant dashboard (orgs, members, tenant-scoped data). The 'saas' archetype; identical to the default template.",
		platforms: [],
		unified: true,
	},
	restaurant: {
		blurb:
			"Restaurant — menu + reservations with LIVE table availability (each seating shows tables-left, greys to 'Full' instantly) + owner dashboard. One SSR app.",
		platforms: [],
		unified: true,
	},
	creator: {
		blurb:
			"Personal brand / creator — bio + offerings + a newsletter signup with a LIVE subscriber counter + owner dashboard. One SSR app.",
		platforms: [],
		unified: true,
	},
	shop: {
		blurb:
			"DTC store — product grid with LIVE inventory (stock ticks down, flips to 'Sold out' instantly), cart + real Stripe checkout (graceful no-key fallback) + owner dashboard (orders + stock). One SSR app.",
		platforms: [],
		unified: true,
	},
	agency: {
		blurb:
			"Design/dev studio — services, case-study portfolio, team + a project inquiry form. LIVE availability (hero shows open project slots; booking a lead from the dashboard drops it instantly) + owner pipeline. Clearly-marked photo placeholders. One SSR app.",
		platforms: [],
		unified: true,
	},
	marketplace: {
		blurb:
			"Two-sided marketplace — SSR browse grid + listing detail pages (SEO) with REALTIME offers, a live 'just listed' ticker, buy-now/make-offer/watch, and per-user inbox. Email/password auth, owner-stamped listings/offers. Multi-user (no single owner). One SSR app.",
		platforms: [],
		unified: true,
	},
	directory: {
		blurb:
			"Curated directory — LIVE full-text search + category facets (db.useSearch), community upvotes that tick up across tabs, and a moderated submit flow (deny-all PII → curator approves → public). Showcases Pylon FTS. One SSR app.",
		platforms: [],
		unified: true,
	},
	"ai-chat": {
		blurb:
			"Streaming AI chat — token streaming via the built-in /api/ai/stream (your key stays server-side), multi-conversation history that's owner-scoped + synced across tabs in realtime, guest or signed-in. Set PYLON_AI_API_KEY to enable. One SSR app.",
		platforms: [],
		unified: true,
	},
	"ai-studio": {
		blurb:
			"Generative media studio (image/audio/video) — a LIVE gallery that fills in as each background job finishes (pending card → result, synced across tabs). Owner-scoped generations; provider call + key stay server-side. Boots with a no-key placeholder; set REPLICATE_API_TOKEN for real image/audio/video. One SSR app.",
		platforms: [],
		unified: true,
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
	// --skill / --no-skill: install the Pylon skill into the new project via
	// `npx skills add pylonsync/pylon` — the skills.sh CLI detects the coding
	// agent (Claude Code / Codex / Cursor) and drops the always-current skill
	// from this repo's skills/pylon/SKILL.md. undefined => prompt (default yes).
	skill: args.includes("--skill")
		? true
		: args.includes("--no-skill")
			? false
			: undefined,
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
		([k, v]) =>
			`    ${k.padEnd(10)} ${v.blurb} (${v.unified ? "single app" : v.platforms.join(", ")})`,
	);
	process.stdout.write(`
Usage: npm create @pylonsync/pylon [name] [options]

  --template <t>         ${TEMPLATES_AVAILABLE.join(" | ")}
${tmplLines.join("\n")}

  --platforms <list>     comma list: ${PLATFORMS_AVAILABLE.join(",")}  (default: web)
                         ignored for unified templates — they're a single full-stack app
  --bun|--pnpm|--yarn|--npm
  --skip-install         scaffold only, don't run install

Examples:
  npm create @pylonsync/pylon my-app                        # default — SaaS landing + multi-tenant dashboard
  npm create @pylonsync/pylon my-app --template todo        # live, optimistic todo (SSR, one port)
  npm create @pylonsync/pylon my-app --template chat         # realtime live chat room
  npm create @pylonsync/pylon my-app --template waitlist     # coming-soon landing + live signup counter
  npm create @pylonsync/pylon my-app --template local-service # appointment business + live booking availability
  npm create @pylonsync/pylon my-app --template restaurant   # restaurant menu + live reservation availability
  npm create @pylonsync/pylon my-app --template creator      # personal brand + live newsletter subscriber count
  npm create @pylonsync/pylon my-app --template shop         # DTC store + live inventory
  npm create @pylonsync/pylon my-app --template agency       # studio/agency + live project availability
  npm create @pylonsync/pylon my-app --template marketplace  # two-sided marketplace + realtime offers
  npm create @pylonsync/pylon my-app --template directory    # searchable directory + facets + live upvotes
  npm create @pylonsync/pylon my-app --template ai-chat      # streaming LLM chat + synced history
  npm create @pylonsync/pylon my-app --template ai-studio    # generative media studio + live gallery
`);
	exit(0);
}

const rl = createInterface({ input: stdin, output: stdout });
// Non-interactive stdin (CI, piped, `| npm create …`): never block on a prompt.
// Fall back to the same defaults the interactive path uses. The skill-install
// below already gates on `stdin.isTTY`; the INPUT prompts must too — without
// this, `npm create @pylonsync/pylon my-app` (or any partial-flag invocation)
// HANGS forever in CI waiting on input that never comes.
const isInteractive = !!stdin.isTTY;
if (!projectName) {
	projectName =
		(isInteractive ? (await rl.question("Project name: ")).trim() : "") ||
		"my-pylon-app";
}
if (!flags.template) {
	if (isInteractive) {
		const lines = Object.entries(TEMPLATE_REGISTRY)
			.map(([k, v]) => `  ${k.padEnd(10)} ${v.blurb}`)
			.join("\n");
		process.stdout.write(`\n${lines}\n`);
		const ans = (
			await rl.question(
				`Template (${TEMPLATES_AVAILABLE.join(", ")}) [default]: `,
			)
		)
			.trim()
			.toLowerCase();
		flags.template = TEMPLATES_AVAILABLE.includes(ans) ? ans : "default";
	} else {
		console.log("Non-interactive stdin — using --template default.");
		flags.template = "default";
	}
}
// `ssr` was the original name of the default template; keep it working as a
// quiet alias so older `--template ssr` invocations don't break.
if (flags.template === "ssr") flags.template = "default";
// `saas` is the archetype NAME for the default template (a SaaS landing +
// multi-tenant dashboard). It scaffolds the exact same app as `default`; we
// alias rather than duplicate the whole tree so there's one source of truth.
if (flags.template === "saas") flags.template = "default";
// `unified` templates (default) are a single app, not a monorepo — they take
// no platforms. Skip the platform prompt + validation for them entirely.
const isUnified = TEMPLATE_REGISTRY[flags.template]?.unified === true;
if (!isUnified && !flags.platforms) {
	const supported = TEMPLATE_REGISTRY[flags.template].platforms.join(", ");
	const ans = isInteractive
		? (
				await rl.question(
					`Platforms for ${flags.template} (${supported}, comma-separated) [web]: `,
				)
			).trim()
		: "";
	flags.platforms = ans || "web";
}
if (!flags.pm) {
	const detected = detectPackageManager();
	const def = detected ?? "bun";
	if (isInteractive) {
		const choice = (
			await rl.question(`Package manager (bun, pnpm, yarn, npm) [${def}]: `)
		)
			.trim()
			.toLowerCase();
		flags.pm = ["bun", "pnpm", "yarn", "npm"].includes(choice) ? choice : def;
	} else {
		flags.pm = def;
	}
}
if (flags.skill === undefined) {
	if (isInteractive) {
		const ans = (
			await rl.question(
				"Add the Pylon skill to your coding agent (Claude Code / Codex / Cursor)? [Y/n]: ",
			)
		)
			.trim()
			.toLowerCase();
		flags.skill = ans !== "n" && ans !== "no";
	} else {
		// Non-interactive: can't prompt. The actual install is TTY-gated below,
		// so this only controls whether the footer prints the one-liner hint.
		flags.skill = true;
	}
}
rl.close();

// Unified templates own no platforms — they ARE the whole app. For
// everything else, parse + validate the platform list.
const platforms = isUnified
	? []
	: (flags.platforms ?? "web")
			.split(",")
			.map((p) => p.trim().toLowerCase())
			.filter(Boolean);
if (!isUnified) {
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
	if (platforms.includes("web") && platforms.includes("vite")) {
		console.error(
			`\nError: --platforms web and vite are mutually exclusive (both render into apps/web).\n` +
				`       Pick one: web for Next.js 16, vite for plain Vite + React.\n`,
		);
		exit(1);
	}
}
if (!TEMPLATES_AVAILABLE.includes(flags.template)) {
	console.error(
		`\nError: unknown template "${flags.template}". Valid: ${TEMPLATES_AVAILABLE.join(", ")}\n`,
	);
	exit(1);
}

// Reject combos a template doesn't yet support — better to fail loud
// than to scaffold an incomplete tree (e.g. a web-only template + expo
// would skip frontend entirely and leave a half-empty workspace).
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
	`\nCreating ${projectName} (${flags.template}${isUnified ? "" : `, ${platforms.join(" + ")}`}) in ${root}\n`,
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
	__RUN_DEV__: flags.pm === "npm" ? "npm run dev" : `${flags.pm} run dev`,
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
		let renamed = substituteString(entry);
		// npm strips a literal `.gitignore` from published tarballs, so
		// templates ship it as `gitignore` and we restore the dot at
		// scaffold time — otherwise the new project has no ignore file
		// and node_modules / .pylon / *.db get committed.
		if (renamed === "gitignore") renamed = ".gitignore";
		// `bun publish` (which the release uses) strips a literal
		// `bunfig.toml` — it treats it as Bun's own runtime config and
		// refuses to ship it in a package. Templates ship it as `bunfig`
		// and we restore the `.toml` here, so the happy-dom test preload
		// actually lands and component tests can render.
		if (renamed === "bunfig") renamed = "bunfig.toml";
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

// Like copyTemplate but FATAL when the source is missing. A required
// template dir that isn't on disk means a corrupt/partial create-pylon
// install (or a published tarball that dropped files) — historically this
// scaffolded an EMPTY project and STILL printed "✓ Created", sending the
// user to a dead `pylon dev` with no clue why. Fail loud + actionable
// instead. (The @pylonsync/client publish-drop that broke the default
// scaffold is exactly this failure mode.)
function mustCopy(srcSubpath, destSubpath = "", label = srcSubpath) {
	if (!copyTemplate(srcSubpath, destSubpath)) {
		console.error(
			`\nError: template files for "${label}" are missing from this install\n` +
				`       (expected ${join(TEMPLATES, srcSubpath)}).\n` +
				`       This usually means a corrupt or partial create-pylon install.\n` +
				`       Re-run with @latest:  npm create @pylonsync/pylon@latest\n` +
				`       or report it:         https://github.com/pylonsync/pylon/issues\n`,
		);
		exit(1);
	}
}

// ---------------------------------------------------------------------------
// Apply templates in order:
//   1. _root            — gitignore, env.example, README
//   2. backend/<t>      — apps/api/* always present (one per template)
//   3. ui               — packages/ui (only if web is in platforms)
//   4. <platform>/<t>   — one per requested platform under apps/<platform>/
//   5. Root package.json — generated, not templated
// ---------------------------------------------------------------------------

if (isUnified) {
	// Single unified app: app.ts + app/ routes + functions/, served by
	// `pylon dev` (frontend + API, one port). The template ships its own
	// package.json — no monorepo root, no turbo, no workspaces.
	mustCopy(flags.template);
} else {
	mustCopy("_root");
	mustCopy(`backend/${flags.template}`);

	// `web` (Next.js) and `vite` are alternative web-frontend toolchains;
	// the mutex check above guarantees at most one of them is set. Either
	// way we also pull in packages/ui so the shared primitives are present.
	if (platforms.includes("web")) {
		mustCopy("ui");
		mustCopy(`web/${flags.template}`);
	}
	if (platforms.includes("vite")) {
		mustCopy("ui");
		mustCopy(`vite/${flags.template}`);
	}
	for (const p of ["ios", "mac", "expo"]) {
		if (platforms.includes(p)) mustCopy(`${p}/${flags.template}`);
	}
}

walkAndSubstitute(root);

// ---------------------------------------------------------------------------
// Root package.json — generated based on selected platforms (monorepo
// templates only). Unified templates ship their own single-app
// package.json, so skip this entirely.
// ---------------------------------------------------------------------------

if (!isUnified) {
	// Turborepo orchestrates the workspace. `turbo dev` runs the `dev`
	// task in every package that defines one (apps/api always; apps/web,
	// apps/expo when scaffolded). Native targets (ios, mac) aren't
	// `turbo dev`-shaped — Xcode / `swift run` block — so they get
	// dedicated escape-hatch scripts instead. turbo.json ships in
	// _root/, so it's already in the project.
	const helperScripts = {};
	if (platforms.includes("ios")) {
		helperScripts["dev:ios"] =
			"echo 'cd apps/ios && xcodegen generate && open *.xcodeproj  (or: swift run for a quick macOS preview)'";
	}
	if (platforms.includes("mac")) {
		helperScripts["dev:mac"] =
			"echo 'cd apps/mac && swift run  (or: xcodegen generate && open *.xcodeproj)'";
	}

	// Turbo 2.x refuses to run without packageManager set. Pick a recent-
	// stable for whichever PM the user picked. npm doesn't enforce this
	// field but turbo still expects it to be present.
	const PACKAGE_MANAGERS = {
		bun: "bun@1.2.19",
		pnpm: "pnpm@9.12.0",
		yarn: "yarn@4.5.0",
		npm: "npm@10.9.0",
	};

	const rootPkg = {
		name: APP_NAME_KEBAB,
		private: true,
		type: "module",
		packageManager: PACKAGE_MANAGERS[flags.pm],
		workspaces: ["apps/*", "packages/*"].filter((p) => {
			// Only declare packages/* as a workspace if we actually scaffolded
			// packages/ui — otherwise the empty match warns on bun install.
			if (p === "packages/*")
				return platforms.includes("web") || platforms.includes("vite");
			return true;
		}),
		scripts: {
			dev: "turbo dev",
			build: "turbo build",
			check: "turbo check",
			lint: "turbo lint",
			...helperScripts,
		},
		devDependencies: {
			turbo: "^2.3.0",
		},
	};
	writeFileSync(
		join(root, "package.json"),
		JSON.stringify(rootPkg, null, 2) + "\n",
	);
}

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
// Optional: install the Pylon skill into the project's coding agent.
//
// `npx skills add pylonsync/pylon` (skills.sh) detects the installed agent
// (Claude Code / Codex / Cursor) and drops the canonical skill from this
// repo's skills/pylon/SKILL.md — always current, no stale bundled copy. We
// only auto-run it when stdin is a TTY: the skills CLI prompts for scope and
// agent, which would hang a non-interactive scaffold (CI, piped input). When
// we can't run it, the footer prints the one-liner so the user can opt in.
// ---------------------------------------------------------------------------

const SKILL_INSTALL_CMD = "npx skills add pylonsync/pylon";
let skillInstalled = false;
if (flags.skill && stdin.isTTY) {
	console.log("\nAdding the Pylon skill to your coding agent...");
	const { spawnSync } = await import("node:child_process");
	const result = spawnSync("npx", ["-y", "skills", "add", "pylonsync/pylon"], {
		cwd: root,
		stdio: "inherit",
	});
	skillInstalled = result.status === 0;
	if (!skillInstalled) {
		console.warn(
			`\nCouldn't add the skill automatically. Run it yourself anytime:\n  ${SKILL_INSTALL_CMD}\n`,
		);
	}
}

// ---------------------------------------------------------------------------
// Final instructions
// ---------------------------------------------------------------------------

const runDev = flags.pm === "npm" ? "npm run dev" : `${flags.pm} run dev`;

const platformLines = [];
if (isUnified)
	platformLines.push("  → app      http://localhost:4321  (SSR frontend + API, one port)");
else
	platformLines.push("  → api      http://localhost:4321  (Pylon control plane)");
if (platforms.includes("web"))
	platformLines.push("  → web      http://localhost:3000  (Next.js)");
if (platforms.includes("vite"))
	platformLines.push("  → web      http://localhost:3000  (Vite + React)");
if (platforms.includes("expo"))
	platformLines.push(`  → expo     ${runDev}  (Metro + simulator, alongside web/api)`);
if (platforms.includes("ios"))
	platformLines.push(`  → ios      cd apps/ios && xcodegen generate && open *.xcodeproj`);
if (platforms.includes("mac"))
	platformLines.push(`  → mac      cd apps/mac && swift run  (or xcodegen for .app)`);

const layoutLines = isUnified
	? [
			"  app.ts            data model + manifest (entities, functions, routes)",
			"  app/              file-based SSR routes (app/page.tsx → /)",
			"  app/layout.tsx    root layout (receives url + auth)",
			"  functions/        server functions (query/action)",
		]
	: ["  apps/api          schema + functions/ handlers"];
if (!isUnified && platforms.includes("web")) {
	layoutLines.push("  apps/web          Next.js 16 + React 19 + Tailwind v4");
	layoutLines.push("  packages/ui       shared UI primitives");
}
if (platforms.includes("vite")) {
	layoutLines.push("  apps/web          Vite + React 19 + Tailwind v4");
	layoutLines.push("  packages/ui       shared UI primitives");
}
if (platforms.includes("ios"))
	layoutLines.push("  apps/ios          Swift / SwiftUI (iOS)");
if (platforms.includes("mac"))
	layoutLines.push("  apps/mac          Swift / SwiftUI (macOS)");
if (platforms.includes("expo"))
	layoutLines.push("  apps/expo         Expo + React Native");

const skillLine = skillInstalled
	? "\nPylon skill added to your coding agent (Claude Code / Codex / Cursor).\n"
	: flags.skill
		? `\nAdd the Pylon skill to your coding agent:\n  ${SKILL_INSTALL_CMD}\n`
		: "";

console.log(`
✓ Created ${projectName}

  cd ${projectName}
  ${runDev}

${platformLines.join("\n")}

Layout:
${layoutLines.join("\n")}
${skillLine}
Docs: https://docs.pylonsync.com
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

