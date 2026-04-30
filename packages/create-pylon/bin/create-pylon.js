#!/usr/bin/env node
/**
 * @pylonsync/create-pylon — scaffold a new Pylon app.
 *
 * Run via `npm create @pylonsync/pylon@latest [name]` (or yarn/pnpm/bun
 * create @pylonsync/pylon).
 *
 * Generates a workspace with two packages:
 *   - api/ — Pylon backend (schema + functions; runs `pylon dev` from
 *     the @pylonsync/cli npm package, no global binary required)
 *   - web/ — Next.js 16 + React 19 frontend wired to @pylonsync/react
 *
 * Node-runnable (no Bun required) so `npm create` works for every
 * package manager. Uses only Node-builtin APIs — no runtime deps.
 */

import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline/promises";
import { stdin, stdout, exit, argv, cwd } from "node:process";

// ---------------------------------------------------------------------------
// Version pin — every generated dep references this version of @pylonsync/*.
// Bumped via the workspace's release-please flow (same version as the rest
// of the pylon stack).
// ---------------------------------------------------------------------------

const PYLON_VERSION = "0.3.16";

// ---------------------------------------------------------------------------
// CLI args + interactive prompt
// ---------------------------------------------------------------------------

const args = argv.slice(2);
let projectName = args.find((a) => !a.startsWith("--"));

const flags = {
	pm:
		args.find((a) => a === "--bun" || a === "--pnpm" || a === "--yarn" || a === "--npm")?.slice(2) ??
		detectPackageManager(),
	skipInstall: args.includes("--skip-install"),
	help: args.includes("--help") || args.includes("-h"),
};

if (flags.help) {
	process.stdout.write(`\nUsage: npm create @pylonsync/pylon [name] [--bun|--pnpm|--yarn|--npm] [--skip-install]\n\n`);
	exit(0);
}

if (!projectName) {
	const rl = createInterface({ input: stdin, output: stdout });
	projectName = (await rl.question("Project name: ")).trim() || "my-pylon-app";
	rl.close();
}

const root = resolve(cwd(), projectName);

if (existsSync(root) && readdirSync(root).length > 0) {
	console.error(`\nError: ${root} already exists and is not empty.\n`);
	exit(1);
}

console.log(`\nCreating ${projectName} in ${root}\n`);

// ---------------------------------------------------------------------------
// File-tree generator — every `write(path, content)` call creates parent
// dirs as needed and writes UTF-8 text. Keeping the scaffold inline (no
// template files) means create-pylon stays a single zero-dep file.
// ---------------------------------------------------------------------------

function write(path, content) {
	const full = join(root, path);
	mkdirSync(dirname(full), { recursive: true });
	writeFileSync(full, content);
}

function writeJson(path, value) {
	write(path, JSON.stringify(value, null, 2) + "\n");
}

// ---------------------------------------------------------------------------
// Root workspace
// ---------------------------------------------------------------------------

writeJson("package.json", {
	name: projectName,
	private: true,
	type: "module",
	workspaces: ["api", "web"],
	scripts: {
		dev: "npm-run-all --parallel dev:api dev:web",
		"dev:api": "npm --workspace api run dev",
		"dev:web": "npm --workspace web run dev",
		build: "npm --workspaces run build --if-present",
	},
	devDependencies: {
		"npm-run-all": "^4.1.5",
	},
});

write(".gitignore", `node_modules/
.next/
.turbo/
dist/
out/
.env
.env.local
*.db
*.db-journal
api/pylon.manifest.json
api/pylon.client.ts
`);

write(".env.example", `# Backend port the Pylon control plane listens on.
PYLON_PORT=4321

# Where the Next.js dev server can reach the control plane.
PYLON_TARGET=http://localhost:4321

# Cookie name the auth helpers look for.
# Pattern: \`\${app_name}_session\` from the Pylon manifest.
PYLON_COOKIE_NAME=${projectName}_session
`);

write(
	"README.md",
	`# ${projectName}

Realtime backend + Next.js dashboard, scaffolded by [create-pylon](https://npmjs.com/create-pylon).

## Getting started

\`\`\`sh
${flags.pm === "npm" ? "npm install" : `${flags.pm} install`}
${flags.pm === "npm" ? "npm run dev" : `${flags.pm} run dev`}
\`\`\`

That spins up two processes:

- **api** on http://localhost:4321 — Pylon control plane (schema, queries,
  mutations, live sync, auth)
- **web** on http://localhost:3000 — Next.js 16 frontend wired to the API
  via [\`@pylonsync/react\`](https://npmjs.com/package/@pylonsync/react)

## Project layout

\`\`\`
api/
  schema.ts         entities + policies + manifest
  functions/        TS query / mutation / action handlers
  pylon.manifest.json   (codegen — gitignored)
  pylon.client.ts       (typed client codegen — gitignored)

web/
  src/app/          Next.js app-router pages
  src/lib/pylon.ts  Pylon server helper (cookie-attached fetches)
\`\`\`

## What to do next

- Edit \`api/schema.ts\` to add your entities + policies.
- Add TS handlers to \`api/functions/\` — they're auto-discovered.
- Edit \`web/src/app/page.tsx\` — it uses the typed client codegen
  produced from your manifest.

## Docs

[pylonsync.com/docs](https://pylonsync.com/docs)
`,
);

// ---------------------------------------------------------------------------
// api/ — the Pylon control plane
// ---------------------------------------------------------------------------

writeJson("api/package.json", {
	name: `${projectName}-api`,
	version: "0.0.1",
	private: true,
	type: "module",
	scripts: {
		dev: "pylon dev schema.ts --port 4321",
		build: "pylon codegen schema.ts --out pylon.manifest.json && pylon codegen client pylon.manifest.json --out pylon.client.ts",
		"schema:push": "pylon schema push pylon.manifest.json --sqlite dev.db",
		"schema:inspect": "pylon schema inspect --sqlite dev.db",
	},
	dependencies: {
		"@pylonsync/sdk": `^${PYLON_VERSION}`,
		"@pylonsync/functions": `^${PYLON_VERSION}`,
	},
	devDependencies: {
		"@pylonsync/cli": `^${PYLON_VERSION}`,
		typescript: "^5.5.0",
	},
});

writeJson("api/tsconfig.json", {
	compilerOptions: {
		target: "ES2022",
		module: "ESNext",
		moduleResolution: "Bundler",
		strict: true,
		skipLibCheck: true,
		noEmit: true,
		esModuleInterop: true,
		allowSyntheticDefaultImports: true,
	},
	include: ["schema.ts", "functions/**/*.ts"],
});

write(
	"api/schema.ts",
	`import { entity, field, defineRoute, query, action, policy, buildManifest } from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const Todo = entity("Todo", {
\ttitle: field.string(),
\tdone: field.bool().default(false),
\tcreatedAt: field.datetime().default("now"),
});

// ---------------------------------------------------------------------------
// Queries / mutations
// ---------------------------------------------------------------------------

const listTodos = query("listTodos", {
\thandler: \`
\t\tasync (ctx) => {
\t\t\treturn await ctx.db.query("Todo", { $order: { createdAt: "desc" } });
\t\t}
\t\`,
});

const addTodo = action("addTodo", {
\targs: { title: { type: "string" } },
\thandler: \`
\t\tasync (ctx, args) => {
\t\t\treturn await ctx.db.insert("Todo", {
\t\t\t\ttitle: args.title,
\t\t\t\tdone: false,
\t\t\t\tcreatedAt: new Date().toISOString(),
\t\t\t});
\t\t}
\t\`,
});

// ---------------------------------------------------------------------------
// Policies — wide-open by default. Tighten before production.
// ---------------------------------------------------------------------------

const todoPolicy = policy({
\tname: "todo_open",
\tentity: "Todo",
\tallowRead: "true",
\tallowInsert: "true",
\tallowUpdate: "true",
\tallowDelete: "true",
});

// ---------------------------------------------------------------------------
// Manifest — codegen reads this and emits pylon.manifest.json
// ---------------------------------------------------------------------------

export default buildManifest({
\tname: "${projectName}",
\tversion: "0.0.1",
\tentities: [Todo],
\tqueries: [listTodos],
\tactions: [addTodo],
\tpolicies: [todoPolicy],
\troutes: [],
});
`,
);

// ---------------------------------------------------------------------------
// web/ — Next.js 16 + React 19 + Tailwind v4 + @pylonsync/react
// ---------------------------------------------------------------------------

writeJson("web/package.json", {
	name: `${projectName}-web`,
	version: "0.0.1",
	private: true,
	type: "module",
	scripts: {
		dev: "next dev --port 3000",
		build: "next build",
		start: "next start",
		lint: "next lint",
	},
	dependencies: {
		"@pylonsync/sdk": `^${PYLON_VERSION}`,
		"@pylonsync/react": `^${PYLON_VERSION}`,
		"@pylonsync/next": `^${PYLON_VERSION}`,
		next: "^16.0.0",
		react: "^19.0.0",
		"react-dom": "^19.0.0",
	},
	devDependencies: {
		"@types/react": "^19.0.0",
		"@types/react-dom": "^19.0.0",
		"@types/node": "^20.0.0",
		"@tailwindcss/postcss": "^4.0.0",
		tailwindcss: "^4.0.0",
		typescript: "^5.5.0",
	},
});

writeJson("web/tsconfig.json", {
	compilerOptions: {
		target: "ES2022",
		lib: ["dom", "dom.iterable", "esnext"],
		allowJs: true,
		skipLibCheck: true,
		strict: true,
		noEmit: true,
		esModuleInterop: true,
		module: "esnext",
		moduleResolution: "bundler",
		resolveJsonModule: true,
		isolatedModules: true,
		jsx: "preserve",
		incremental: true,
		plugins: [{ name: "next" }],
		paths: { "@/*": ["./src/*"] },
	},
	include: ["next-env.d.ts", "src/**/*.ts", "src/**/*.tsx", ".next/types/**/*.ts"],
	exclude: ["node_modules"],
});

write(
	"web/next.config.ts",
	`import type { NextConfig } from "next";

/**
 * Pylon's typed client + functions packages re-export across the
 * server/client boundary; \`transpilePackages\` makes Next bundle them
 * cleanly from the workspace.
 */
const config: NextConfig = {
\ttranspilePackages: [
\t\t"@pylonsync/sdk",
\t\t"@pylonsync/react",
\t\t"@pylonsync/next",
\t\t"@pylonsync/functions",
\t\t"@pylonsync/sync",
\t],
};

export default config;
`,
);

write(
	"web/postcss.config.mjs",
	`/** Tailwind v4 PostCSS pipeline. */
export default {
\tplugins: { "@tailwindcss/postcss": {} },
};
`,
);

write(
	"web/src/app/globals.css",
	`@import "tailwindcss";

:root {
\tcolor-scheme: light dark;
}

html, body { height: 100%; }
body { font-family: ui-sans-serif, system-ui, -apple-system, sans-serif; }
`,
);

write(
	"web/src/app/layout.tsx",
	`import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
\ttitle: "${projectName}",
\tdescription: "Realtime app powered by Pylon",
};

export default function RootLayout({
\tchildren,
}: {
\tchildren: React.ReactNode;
}) {
\treturn (
\t\t<html lang="en">
\t\t\t<body className="antialiased min-h-screen bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100">
\t\t\t\t{children}
\t\t\t</body>
\t\t</html>
\t);
}
`,
);

write(
	"web/src/lib/pylon.ts",
	`import { createPylonServer } from "@pylonsync/next/server";

/**
 * Single server-helper instance. Imported by every Server Component
 * and Server Action that needs to talk to the Pylon control plane.
 *
 * \`cookieName\` MUST match the backend's emitted cookie. Pylon uses
 * \`\${app_name}_session\` from the manifest — for this app that's
 * \`${projectName}_session\`. Pin it in code (NOT env) so a bad
 * deployment env can't silently break auth.
 */
export const pylon = createPylonServer({
\tcookieName: "${projectName}_session",
});
`,
);

write(
	"web/src/app/page.tsx",
	`import { pylon } from "@/lib/pylon";
import { TodoList } from "./TodoList";

// Force dynamic — every render reads the live todo list from Pylon.
// Without this Next would try to statically generate the page and
// the cookie-attached fetch in pylon.json would error at build time.
export const dynamic = "force-dynamic";

type Todo = {
\tid: string;
\ttitle: string;
\tdone: boolean;
\tcreatedAt: string;
};

export default async function HomePage() {
\tconst todos = await pylon
\t\t.json<Todo[]>("/api/fn/listTodos", { method: "POST", body: "{}", headers: { "Content-Type": "application/json" } })
\t\t.catch(() => [] as Todo[]);

\treturn (
\t\t<main className="mx-auto max-w-2xl px-6 py-12 space-y-8">
\t\t\t<header className="space-y-2">
\t\t\t\t<h1 className="text-3xl font-semibold tracking-tight">${projectName}</h1>
\t\t\t\t<p className="text-sm text-neutral-500 dark:text-neutral-400">
\t\t\t\t\tA Pylon-powered realtime app. Edit{" "}
\t\t\t\t\t<code className="font-mono text-xs">api/schema.ts</code> to change the
\t\t\t\t\tdata model or{" "}
\t\t\t\t\t<code className="font-mono text-xs">web/src/app/page.tsx</code> for
\t\t\t\t\tthe UI.
\t\t\t\t</p>
\t\t\t</header>

\t\t\t<TodoList initialTodos={todos} />
\t\t</main>
\t);
}
`,
);

write(
	"web/src/app/TodoList.tsx",
	`"use client";

import { useState, useTransition } from "react";

type Todo = {
\tid: string;
\ttitle: string;
\tdone: boolean;
\tcreatedAt: string;
};

/**
 * Optimistic todo list — local state mirrors the server-fetched
 * initial list and refreshes on every successful add. For full
 * real-time updates wire \`@pylonsync/react\`'s \`useQuery\` hook
 * (see https://pylonsync.com/docs/clients/react).
 */
export function TodoList({ initialTodos }: { initialTodos: Todo[] }) {
\tconst [todos, setTodos] = useState(initialTodos);
\tconst [title, setTitle] = useState("");
\tconst [pending, startTransition] = useTransition();

\tasync function add() {
\t\tif (!title.trim()) return;
\t\tconst newTitle = title;
\t\tsetTitle("");
\t\tstartTransition(async () => {
\t\t\tconst res = await fetch("/api/fn/addTodo", {
\t\t\t\tmethod: "POST",
\t\t\t\theaders: { "Content-Type": "application/json" },
\t\t\t\tbody: JSON.stringify({ title: newTitle }),
\t\t\t});
\t\t\tif (res.ok) {
\t\t\t\tconst todo = (await res.json()) as Todo;
\t\t\t\tsetTodos([todo, ...todos]);
\t\t\t}
\t\t});
\t}

\treturn (
\t\t<div className="space-y-4">
\t\t\t<form
\t\t\t\tonSubmit={(e) => {
\t\t\t\t\te.preventDefault();
\t\t\t\t\tadd();
\t\t\t\t}}
\t\t\t\tclassName="flex gap-2"
\t\t\t>
\t\t\t\t<input
\t\t\t\t\tvalue={title}
\t\t\t\t\tonChange={(e) => setTitle(e.target.value)}
\t\t\t\t\tplaceholder="What needs doing?"
\t\t\t\t\tclassName="flex-1 rounded-md border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
\t\t\t\t\tdisabled={pending}
\t\t\t\t/>
\t\t\t\t<button
\t\t\t\t\ttype="submit"
\t\t\t\t\tclassName="rounded-md bg-neutral-900 dark:bg-white text-white dark:text-neutral-900 px-4 py-2 text-sm font-medium disabled:opacity-50"
\t\t\t\t\tdisabled={pending || !title.trim()}
\t\t\t\t>
\t\t\t\t\tAdd
\t\t\t\t</button>
\t\t\t</form>

\t\t\t{todos.length === 0 ? (
\t\t\t\t<p className="text-sm text-neutral-500 dark:text-neutral-400 text-center py-8">
\t\t\t\t\tNo todos yet. Add one above.
\t\t\t\t</p>
\t\t\t) : (
\t\t\t\t<ul className="divide-y divide-neutral-200 dark:divide-neutral-800 rounded-md border border-neutral-200 dark:border-neutral-800">
\t\t\t\t\t{todos.map((t) => (
\t\t\t\t\t\t<li
\t\t\t\t\t\t\tkey={t.id}
\t\t\t\t\t\t\tclassName="flex items-center gap-3 px-4 py-3 text-sm"
\t\t\t\t\t\t>
\t\t\t\t\t\t\t<span className={t.done ? "line-through text-neutral-400" : ""}>
\t\t\t\t\t\t\t\t{t.title}
\t\t\t\t\t\t\t</span>
\t\t\t\t\t\t</li>
\t\t\t\t\t))}
\t\t\t\t</ul>
\t\t\t)}
\t\t</div>
\t);
}
`,
);

write(
	"web/next-env.d.ts",
	`/// <reference types="next" />
/// <reference types="next/image-types/global" />
`,
);

// ---------------------------------------------------------------------------
// Detect package manager — read npm_config_user_agent set by the runner.
// ---------------------------------------------------------------------------

function detectPackageManager() {
	const ua = process.env.npm_config_user_agent ?? "";
	if (ua.startsWith("bun")) return "bun";
	if (ua.startsWith("pnpm")) return "pnpm";
	if (ua.startsWith("yarn")) return "yarn";
	return "npm";
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
// Final instructions
// ---------------------------------------------------------------------------

const runDev = flags.pm === "npm" ? "npm run dev" : `${flags.pm} run dev`;

console.log(`
✓ Created ${projectName}

  cd ${projectName}
  ${runDev}

  → api  http://localhost:4321  (Pylon control plane)
  → web  http://localhost:3000  (Next.js dashboard)

Next:
  - Edit api/schema.ts to add entities + policies.
  - Drop TypeScript handlers into api/functions/ — auto-discovered.
  - The Next page at web/src/app/page.tsx talks to the API via the
    cookie-attached helper in web/src/lib/pylon.ts.

Docs: https://pylonsync.com/docs
`);
