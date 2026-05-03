#!/usr/bin/env node
/**
 * @pylonsync/create-pylon — scaffold a new Pylon app.
 *
 * Run via `npm create @pylonsync/pylon@latest [name]` (or yarn/pnpm/bun
 * create @pylonsync/pylon).
 *
 * Generates a workspace with three packages under apps/* + packages/*:
 *   - apps/api   — Pylon backend (schema + functions/* handlers).
 *   - apps/web   — Next.js 16 + React 19 + Tailwind v4 frontend.
 *   - packages/ui — shared shadcn-style UI primitives consumed by web.
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

const PYLON_VERSION = "0.3.22";

// ---------------------------------------------------------------------------
// CLI args + interactive prompt
// ---------------------------------------------------------------------------

const args = argv.slice(2);
let projectName = args.find((a) => !a.startsWith("--"));

const flags = {
	pm: args.find(
		(a) => a === "--bun" || a === "--pnpm" || a === "--yarn" || a === "--npm",
	)?.slice(2),
	skipInstall: args.includes("--skip-install"),
	help: args.includes("--help") || args.includes("-h"),
};

if (flags.help) {
	process.stdout.write(`\nUsage: npm create @pylonsync/pylon [name] [--bun|--pnpm|--yarn|--npm] [--skip-install]\n\n`);
	exit(0);
}

// Interactive prompts for project name + package manager. Default
// PM to bun: it handles `workspace:*` correctly out of the box,
// installs faster than the alternatives, and is what the
// @pylonsync/* packages are tested against. The user can pick
// anything though.
const rl = createInterface({ input: stdin, output: stdout });
if (!projectName) {
	projectName = (await rl.question("Project name: ")).trim() || "my-pylon-app";
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

// Per-PM script syntax: bun has its own --filter, pnpm uses --filter,
// npm/yarn use --workspace. Picking the right shape at scaffold time
// means `npm run dev` (or whichever PM) works without the user
// learning each PM's flag dialect.
const wsScripts = pmScripts(flags.pm);

writeJson("package.json", {
	name: projectName,
	private: true,
	type: "module",
	workspaces: ["apps/*", "packages/*"],
	scripts: {
		dev: "npm-run-all --parallel dev:api dev:web",
		"dev:api": wsScripts.devApi,
		"dev:web": wsScripts.devWeb,
		build: wsScripts.build,
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
apps/api/pylon.manifest.json
apps/api/pylon.client.ts
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

Realtime backend + Next.js dashboard, scaffolded by [@pylonsync/create-pylon](https://npmjs.com/@pylonsync/create-pylon).

## Layout

\`\`\`
apps/
  api/       Pylon backend — schema, policies, function handlers
    schema.ts
    functions/
      listTodos.ts        live query handler
      addTodo.ts          mutation handler

  web/       Next.js 16 + React 19 + Tailwind v4 frontend
    src/
      app/
        layout.tsx
        page.tsx          server component → fetches initial todos
        components/
          TodoList.tsx    client component → optimistic add
      lib/
        pylon.ts          cookie-attached fetch helper

packages/
  ui/        Shared shadcn-style primitives (Button, Input, etc.)
    src/
      button.tsx, input.tsx, card.tsx, ...
\`\`\`

## Getting started

\`\`\`sh
${flags.pm === "npm" ? "npm install" : `${flags.pm} install`}
${flags.pm === "npm" ? "npm run dev" : `${flags.pm} run dev`}
\`\`\`

That spins up two processes:

- **api** on http://localhost:4321 — Pylon control plane
- **web** on http://localhost:3000 — Next.js frontend wired via
  [\`@pylonsync/next\`](https://npmjs.com/@pylonsync/next)

## What to do next

- Edit \`apps/api/schema.ts\` to add entities + policies.
- Add handlers under \`apps/api/functions/\` — auto-discovered by name.
- Drop new UI primitives into \`packages/ui/src/\`; import them from
  any app via \`import { Button } from "@${projectName}/ui";\`.

## Docs

[pylonsync.com/docs](https://pylonsync.com/docs)
`,
);

// ---------------------------------------------------------------------------
// apps/api — Pylon backend
// ---------------------------------------------------------------------------

writeJson("apps/api/package.json", {
	name: `@${projectName}/api`,
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

writeJson("apps/api/tsconfig.json", {
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

// Schema declares NAMES only — the SDK's query/action/mutation are
// pure manifest declarations. Handler code lives under functions/*.
write(
	"apps/api/schema.ts",
	`import {
\tentity,
\tfield,
\tquery,
\taction,
\tpolicy,
\tbuildManifest,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const Todo = entity("Todo", {
\ttitle: field.string(),
\tdone: field.bool(),
\tcreatedAt: field.datetime(),
\t// Float position so drag-reorder can insert between two existing
\t// rows without renumbering the whole list. Frontend computes
\t// (prev.position + next.position) / 2 on drop. Optional for
\t// backwards compat with legacy rows.
\tposition: field.float().optional(),
});

// ---------------------------------------------------------------------------
// Function declarations — names only. Implementations live under
// functions/<name>.ts and are auto-discovered by the runtime.
// ---------------------------------------------------------------------------

const listTodos = query("listTodos");

const addTodo = action("addTodo", {
\tinput: [{ name: "title", type: "string" }],
});

const toggleTodo = action("toggleTodo", {
\tinput: [{ name: "id", type: "id(Todo)" }, { name: "done", type: "bool" }],
});

const deleteTodo = action("deleteTodo", {
\tinput: [{ name: "id", type: "id(Todo)" }],
});

const editTodo = action("editTodo", {
\tinput: [
\t\t{ name: "id", type: "id(Todo)" },
\t\t{ name: "title", type: "string" },
\t],
});

const reorderTodo = action("reorderTodo", {
\tinput: [
\t\t{ name: "id", type: "id(Todo)" },
\t\t{ name: "position", type: "float" },
\t],
});

// ---------------------------------------------------------------------------
// Policies — wide-open by default. Tighten for production.
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
// Manifest — pylon codegen reads this and emits pylon.manifest.json
// ---------------------------------------------------------------------------

// pylon dev / pylon codegen run \`bun run schema.ts\` and read the
// manifest off stdout. The framework expects JSON, not the JS object —
// every Pylon entry file ends with this console.log line.
const manifest = buildManifest({
\tname: "${projectName}",
\tversion: "0.0.1",
\tentities: [Todo],
\tqueries: [listTodos],
\tactions: [addTodo, toggleTodo, deleteTodo, editTodo, reorderTodo],
\tpolicies: [todoPolicy],
\troutes: [],
});

console.log(JSON.stringify(manifest));
`,
);

write(
	"apps/api/functions/listTodos.ts",
	`import { query } from "@pylonsync/functions";

/**
 * Live query — every Todo, in user-controlled drag-reorder position.
 * Rows without a \`position\` (legacy data) get sorted by createdAt as
 * a fallback so the list stays deterministic.
 */
export default query({
\targs: {},
\tasync handler(ctx) {
\t\tconst rows = await ctx.db.query("Todo", {});
\t\treturn [...rows].sort((a: any, b: any) => {
\t\t\tconst ap =
\t\t\t\ttypeof a.position === "number"
\t\t\t\t\t? a.position
\t\t\t\t\t: Date.parse(a.createdAt) || 0;
\t\t\tconst bp =
\t\t\t\ttypeof b.position === "number"
\t\t\t\t\t? b.position
\t\t\t\t\t: Date.parse(b.createdAt) || 0;
\t\t\treturn ap - bp;
\t\t});
\t},
});
`,
);

write(
\t"apps/api/functions/editTodo.ts",
\t\`import { mutation, v } from "@pylonsync/functions";

/**
 * Rename a Todo. Trims whitespace; rejects empty titles.
 */
export default mutation({
\\targs: { id: v.id("Todo"), title: v.string() },
\\tasync handler(ctx, args: { id: string; title: string }) {
\\t\\tconst trimmed = args.title.trim();
\\t\\tif (!trimmed) {
\\t\\t\\tthrow ctx.error("EMPTY_TITLE", "title cannot be empty");
\\t\\t}
\\t\\tawait ctx.db.update("Todo", args.id, { title: trimmed });
\\t\\treturn await ctx.db.get("Todo", args.id);
\\t},
});
\`,
);

write(
\t"apps/api/functions/reorderTodo.ts",
\t\`import { mutation, v } from "@pylonsync/functions";

/**
 * Drag-reorder. Frontend computes \\\`position\\\` as the midpoint of the
 * drop target's neighbors; we just write it. Floats give us ~52 inserts
 * between any two rows before precision matters.
 */
export default mutation({
\\targs: { id: v.id("Todo"), position: v.number() },
\\tasync handler(ctx, args: { id: string; position: number }) {
\\t\\tawait ctx.db.update("Todo", args.id, { position: args.position });
\\t\\treturn await ctx.db.get("Todo", args.id);
\\t},
});
\`,
);

write(
	"apps/api/functions/toggleTodo.ts",
	`import { mutation, v } from "@pylonsync/functions";

/**
 * Flip the \`done\` flag on a Todo. Mutation, not action — needs
 * \`ctx.db.update\` which is only on writable ctx variants.
 */
export default mutation({
\targs: { id: v.id("Todo"), done: v.bool() },
\tasync handler(ctx, args: { id: string; done: boolean }) {
\t\tawait ctx.db.update("Todo", args.id, { done: args.done });
\t\treturn await ctx.db.get("Todo", args.id);
\t},
});
`,
);

write(
	"apps/api/functions/deleteTodo.ts",
	`import { mutation, v } from "@pylonsync/functions";

/**
 * Remove a Todo row. Returns the row as it existed pre-delete so
 * the client can show a "todo removed" toast or animate it out.
 */
export default mutation({
\targs: { id: v.id("Todo") },
\tasync handler(ctx, args: { id: string }) {
\t\tconst snapshot = await ctx.db.get("Todo", args.id);
\t\tawait ctx.db.delete("Todo", args.id);
\t\treturn snapshot;
\t},
});
`,
);

write(
	"apps/api/functions/addTodo.ts",
	`import { mutation, v } from "@pylonsync/functions";

/**
 * Insert a new Todo. Seeds \`position\` to (max + 1024) so new rows
 * land at the end of the drag-reorder list; the 1024 step leaves
 * room for inserts-between without needing global renumber.
 */
export default mutation({
\targs: { title: v.string() },
\tasync handler(ctx, args: { title: string }) {
\t\tconst existing = await ctx.db.query("Todo", {});
\t\tconst maxPos = existing.reduce((acc: number, row: any) => {
\t\t\tconst p =
\t\t\t\ttypeof row.position === "number"
\t\t\t\t\t? row.position
\t\t\t\t\t: Date.parse(row.createdAt) || 0;
\t\t\treturn p > acc ? p : acc;
\t\t}, 0);
\t\tconst id = await ctx.db.insert("Todo", {
\t\t\ttitle: args.title,
\t\t\tdone: false,
\t\t\tcreatedAt: new Date().toISOString(),
\t\t\tposition: maxPos + 1024,
\t\t});
\t\treturn await ctx.db.get("Todo", id);
\t},
});
`,
);

// ---------------------------------------------------------------------------
// packages/ui — shared shadcn-style primitives
// ---------------------------------------------------------------------------

writeJson("packages/ui/package.json", {
	name: `@${projectName}/ui`,
	version: "0.0.1",
	private: true,
	type: "module",
	main: "src/index.ts",
	types: "src/index.ts",
	exports: {
		".": "./src/index.ts",
		"./button": "./src/button.tsx",
		"./input": "./src/input.tsx",
		"./card": "./src/card.tsx",
		"./cn": "./src/cn.ts",
	},
	dependencies: {
		clsx: "^2.1.0",
		"tailwind-merge": "^2.5.0",
	},
	peerDependencies: {
		react: "^19.0.0",
	},
	devDependencies: {
		"@types/react": "^19.0.0",
		typescript: "^5.5.0",
	},
});

writeJson("packages/ui/tsconfig.json", {
	compilerOptions: {
		target: "ES2022",
		lib: ["dom", "esnext"],
		jsx: "preserve",
		module: "ESNext",
		moduleResolution: "Bundler",
		strict: true,
		skipLibCheck: true,
		noEmit: true,
		esModuleInterop: true,
		allowSyntheticDefaultImports: true,
	},
	include: ["src/**/*.ts", "src/**/*.tsx"],
});

write(
	"packages/ui/src/cn.ts",
	`import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Tailwind-aware class merger. Last-class-wins semantics so a
 * caller's \`className\` reliably overrides a default in a UI
 * primitive (e.g. <Button className="bg-red-500"> beats the
 * primitive's bg-neutral-900 base).
 */
export function cn(...inputs: ClassValue[]): string {
\treturn twMerge(clsx(inputs));
}
`,
);

write(
	"packages/ui/src/button.tsx",
	`import * as React from "react";
import { cn } from "./cn";

type Variant = "default" | "primary" | "ghost";
type Size = "sm" | "md";

const variants: Record<Variant, string> = {
\tdefault:
\t\t"bg-neutral-100 hover:bg-neutral-200 text-neutral-900 dark:bg-neutral-800 dark:hover:bg-neutral-700 dark:text-neutral-100",
\tprimary:
\t\t"bg-neutral-900 hover:bg-neutral-800 text-white dark:bg-white dark:hover:bg-neutral-200 dark:text-neutral-900",
\tghost:
\t\t"bg-transparent hover:bg-neutral-100 text-neutral-700 dark:hover:bg-neutral-800 dark:text-neutral-300",
};

const sizes: Record<Size, string> = {
\tsm: "h-8 px-3 text-[13px]",
\tmd: "h-9 px-4 text-sm",
};

export interface ButtonProps
\textends React.ButtonHTMLAttributes<HTMLButtonElement> {
\tvariant?: Variant;
\tsize?: Size;
}

export function Button({
\tclassName,
\tvariant = "default",
\tsize = "md",
\t...props
}: ButtonProps) {
\treturn (
\t\t<button
\t\t\tclassName={cn(
\t\t\t\t"inline-flex items-center justify-center gap-1.5 rounded-md font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500",
\t\t\t\tvariants[variant],
\t\t\t\tsizes[size],
\t\t\t\tclassName,
\t\t\t)}
\t\t\t{...props}
\t\t/>
\t);
}
`,
);

write(
	"packages/ui/src/input.tsx",
	`import * as React from "react";
import { cn } from "./cn";

export type InputProps = React.InputHTMLAttributes<HTMLInputElement>;

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
\tfunction Input({ className, ...props }, ref) {
\t\treturn (
\t\t\t<input
\t\t\t\tref={ref}
\t\t\t\tclassName={cn(
\t\t\t\t\t"flex h-9 w-full rounded-md border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm placeholder:text-neutral-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-50",
\t\t\t\t\tclassName,
\t\t\t\t)}
\t\t\t\t{...props}
\t\t\t/>
\t\t);
\t},
);
`,
);

write(
	"packages/ui/src/card.tsx",
	`import * as React from "react";
import { cn } from "./cn";

export function Card({
\tclassName,
\t...props
}: React.HTMLAttributes<HTMLDivElement>) {
\treturn (
\t\t<div
\t\t\tclassName={cn(
\t\t\t\t"rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900",
\t\t\t\tclassName,
\t\t\t)}
\t\t\t{...props}
\t\t/>
\t);
}

export function CardHeader({
\tclassName,
\t...props
}: React.HTMLAttributes<HTMLDivElement>) {
\treturn (
\t\t<div className={cn("p-5 border-b border-neutral-200 dark:border-neutral-800", className)} {...props} />
\t);
}

export function CardContent({
\tclassName,
\t...props
}: React.HTMLAttributes<HTMLDivElement>) {
\treturn <div className={cn("p-5", className)} {...props} />;
}
`,
);

write(
	"packages/ui/src/index.ts",
	`export { cn } from "./cn";
export { Button, type ButtonProps } from "./button";
export { Input, type InputProps } from "./input";
export { Card, CardHeader, CardContent } from "./card";
`,
);

// ---------------------------------------------------------------------------
// apps/web — Next.js 16 + React 19 + Tailwind v4
// ---------------------------------------------------------------------------

writeJson("apps/web/package.json", {
	name: `@${projectName}/web`,
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
		[`@${projectName}/ui`]: workspaceDepSpec,
		"@pylonsync/sdk": `^${PYLON_VERSION}`,
		"@pylonsync/react": `^${PYLON_VERSION}`,
		"@pylonsync/next": `^${PYLON_VERSION}`,
		// Drag-reorder for the scaffolded TodoList demo.
		"@dnd-kit/core": "^6.3.1",
		"@dnd-kit/sortable": "^10.0.0",
		"@dnd-kit/utilities": "^3.2.2",
		next: "^16.0.0",
		react: "^19.0.0",
		"react-dom": "^19.0.0",
	},
	devDependencies: {
		"@types/node": "^20.0.0",
		"@types/react": "^19.0.0",
		"@types/react-dom": "^19.0.0",
		"@tailwindcss/postcss": "^4.0.0",
		tailwindcss: "^4.0.0",
		typescript: "^5.5.0",
	},
});

writeJson("apps/web/tsconfig.json", {
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
	"apps/web/next.config.ts",
	`import type { NextConfig } from "next";

/**
 * Pylon's typed client + functions packages re-export across the
 * server/client boundary AND the workspace UI package ships TSX.
 * \`transpilePackages\` makes Next bundle them cleanly.
 *
 * \`rewrites\` proxies every Pylon-owned path (\`/api/fn/*\`,
 * \`/api/auth/*\`, \`/api/sync/*\`, …) to the Pylon binary running
 * on \`PYLON_API_URL\` (default http://localhost:4321). Without this,
 * Next.js sees \`/api/fn/addTodo\` as a missing route and 404s before
 * the request ever reaches Pylon.
 *
 * In production set \`PYLON_API_URL\` to wherever you've deployed the
 * Pylon binary (Fly, Render, Railway, your own box). The browser
 * still hits same-origin paths under your Next deployment, and Next
 * forwards them server-side — no CORS, no extra DNS.
 */
const PYLON_API_URL = process.env.PYLON_API_URL ?? "http://localhost:4321";

const config: NextConfig = {
\ttranspilePackages: [
\t\t"@${projectName}/ui",
\t\t"@pylonsync/sdk",
\t\t"@pylonsync/react",
\t\t"@pylonsync/next",
\t\t"@pylonsync/functions",
\t\t"@pylonsync/sync",
\t],
\tasync rewrites() {
\t\treturn [
\t\t\t{ source: "/api/fn/:path*", destination: \`\${PYLON_API_URL}/api/fn/:path*\` },
\t\t\t{ source: "/api/auth/:path*", destination: \`\${PYLON_API_URL}/api/auth/:path*\` },
\t\t\t{ source: "/api/sync/:path*", destination: \`\${PYLON_API_URL}/api/sync/:path*\` },
\t\t\t{ source: "/api/:path*", destination: \`\${PYLON_API_URL}/api/:path*\` },
\t\t];
\t},
};

export default config;
`,
);

write(
	"apps/web/postcss.config.mjs",
	`/** Tailwind v4 PostCSS pipeline. */
export default {
\tplugins: { "@tailwindcss/postcss": {} },
};
`,
);

write(
	"apps/web/src/app/globals.css",
	`@import "tailwindcss";
@source "../../../../packages/ui/src/**/*.{ts,tsx}";

:root {
\tcolor-scheme: light dark;
}

html, body { height: 100%; }
body { font-family: ui-sans-serif, system-ui, -apple-system, sans-serif; }
`,
);

write(
	"apps/web/src/app/layout.tsx",
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
	"apps/web/src/lib/pylon.ts",
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
	"apps/web/src/app/page.tsx",
	`import { pylon } from "@/lib/pylon";
import { TodoList } from "./components/TodoList";

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
\t\t.json<Todo[]>("/api/fn/listTodos", {
\t\t\tmethod: "POST",
\t\t\tbody: "{}",
\t\t\theaders: { "Content-Type": "application/json" },
\t\t})
\t\t.catch(() => [] as Todo[]);

\treturn (
\t\t<main className="mx-auto max-w-2xl px-6 py-12 space-y-8">
\t\t\t<header className="space-y-2">
\t\t\t\t<h1 className="text-3xl font-semibold tracking-tight">${projectName}</h1>
\t\t\t\t<p className="text-sm text-neutral-500 dark:text-neutral-400">
\t\t\t\t\tA Pylon-powered realtime app. Edit{" "}
\t\t\t\t\t<code className="font-mono text-xs">apps/api/schema.ts</code> to change
\t\t\t\t\tthe data model,{" "}
\t\t\t\t\t<code className="font-mono text-xs">apps/api/functions/</code> to add
\t\t\t\t\thandlers, or{" "}
\t\t\t\t\t<code className="font-mono text-xs">
\t\t\t\t\t\tapps/web/src/app/components/TodoList.tsx
\t\t\t\t\t</code>{" "}
\t\t\t\t\tfor the UI.
\t\t\t\t</p>
\t\t\t</header>

\t\t\t<TodoList initialTodos={todos} />
\t\t</main>
\t);
}
`,
);

write(
	"apps/web/src/app/components/TodoList.tsx",
	`"use client";

import { useState, useTransition, useRef, useEffect } from "react";
import { Button } from "@${projectName}/ui";
import { Input } from "@${projectName}/ui";
import {
\tDndContext,
\tclosestCenter,
\tKeyboardSensor,
\tPointerSensor,
\tuseSensor,
\tuseSensors,
\ttype DragEndEvent,
} from "@dnd-kit/core";
import {
\tarrayMove,
\tSortableContext,
\tsortableKeyboardCoordinates,
\tuseSortable,
\tverticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

type Todo = {
\tid: string;
\ttitle: string;
\tdone: boolean;
\tcreatedAt: string;
\tposition?: number;
};

/**
 * Optimistic todo list with drag-reorder, inline title edit, toggle,
 * and delete. All mutations are optimistic with revert-on-failure.
 * Drag uses @dnd-kit; on drop we compute the new row's position as
 * the midpoint between its new neighbors and POST it to reorderTodo.
 */
export function TodoList({ initialTodos }: { initialTodos: Todo[] }) {
\tconst [todos, setTodos] = useState(initialTodos);
\tconst [title, setTitle] = useState("");
\tconst [pending, startTransition] = useTransition();
\tconst sensors = useSensors(
\t\tuseSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
\t\tuseSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
\t);

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
\t\t\t\tsetTodos((prev) => [...prev, todo]);
\t\t\t}
\t\t});
\t}

\tasync function toggle(t: Todo) {
\t\tconst next = !t.done;
\t\tsetTodos((prev) =>
\t\t\tprev.map((row) => (row.id === t.id ? { ...row, done: next } : row)),
\t\t);
\t\tstartTransition(async () => {
\t\t\tconst res = await fetch("/api/fn/toggleTodo", {
\t\t\t\tmethod: "POST",
\t\t\t\theaders: { "Content-Type": "application/json" },
\t\t\t\tbody: JSON.stringify({ id: t.id, done: next }),
\t\t\t});
\t\t\tif (!res.ok) {
\t\t\t\tsetTodos((prev) =>
\t\t\t\t\tprev.map((row) => (row.id === t.id ? { ...row, done: t.done } : row)),
\t\t\t\t);
\t\t\t}
\t\t});
\t}

\tasync function remove(t: Todo) {
\t\tconst snapshot = todos;
\t\tsetTodos((prev) => prev.filter((row) => row.id !== t.id));
\t\tstartTransition(async () => {
\t\t\tconst res = await fetch("/api/fn/deleteTodo", {
\t\t\t\tmethod: "POST",
\t\t\t\theaders: { "Content-Type": "application/json" },
\t\t\t\tbody: JSON.stringify({ id: t.id }),
\t\t\t});
\t\t\tif (!res.ok) setTodos(snapshot);
\t\t});
\t}

\tasync function rename(t: Todo, newTitle: string) {
\t\tconst trimmed = newTitle.trim();
\t\tif (!trimmed || trimmed === t.title) return;
\t\tsetTodos((prev) =>
\t\t\tprev.map((row) => (row.id === t.id ? { ...row, title: trimmed } : row)),
\t\t);
\t\tstartTransition(async () => {
\t\t\tconst res = await fetch("/api/fn/editTodo", {
\t\t\t\tmethod: "POST",
\t\t\t\theaders: { "Content-Type": "application/json" },
\t\t\t\tbody: JSON.stringify({ id: t.id, title: trimmed }),
\t\t\t});
\t\t\tif (!res.ok) {
\t\t\t\tsetTodos((prev) =>
\t\t\t\t\tprev.map((row) => (row.id === t.id ? { ...row, title: t.title } : row)),
\t\t\t\t);
\t\t\t}
\t\t});
\t}

\tfunction onDragEnd(e: DragEndEvent) {
\t\tconst { active, over } = e;
\t\tif (!over || active.id === over.id) return;
\t\tconst oldIndex = todos.findIndex((t) => t.id === active.id);
\t\tconst newIndex = todos.findIndex((t) => t.id === over.id);
\t\tif (oldIndex < 0 || newIndex < 0) return;
\t\tconst reordered = arrayMove(todos, oldIndex, newIndex);
\t\tsetTodos(reordered);
\t\tconst prev = reordered[newIndex - 1];
\t\tconst next = reordered[newIndex + 1];
\t\tconst prevPos = prev?.position ?? Date.parse(prev?.createdAt ?? "") ?? 0;
\t\tconst nextPos = next?.position ?? Date.parse(next?.createdAt ?? "") ?? 0;
\t\tlet position: number;
\t\tif (prev && next) position = (prevPos + nextPos) / 2;
\t\telse if (prev) position = prevPos + 1024;
\t\telse if (next) position = nextPos - 1024;
\t\telse position = 1024;
\t\tconst movedId = String(active.id);
\t\tconst snapshot = todos;
\t\tstartTransition(async () => {
\t\t\tconst res = await fetch("/api/fn/reorderTodo", {
\t\t\t\tmethod: "POST",
\t\t\t\theaders: { "Content-Type": "application/json" },
\t\t\t\tbody: JSON.stringify({ id: movedId, position }),
\t\t\t});
\t\t\tif (!res.ok) setTodos(snapshot);
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
\t\t\t\t<Input
\t\t\t\t\tvalue={title}
\t\t\t\t\tonChange={(e) => setTitle(e.target.value)}
\t\t\t\t\tplaceholder="What needs doing?"
\t\t\t\t\tdisabled={pending}
\t\t\t\t\tclassName="flex-1"
\t\t\t\t/>
\t\t\t\t<Button
\t\t\t\t\ttype="submit"
\t\t\t\t\tvariant="primary"
\t\t\t\t\tdisabled={pending || !title.trim()}
\t\t\t\t>
\t\t\t\t\tAdd
\t\t\t\t</Button>
\t\t\t</form>

\t\t\t{todos.length === 0 ? (
\t\t\t\t<p className="text-sm text-neutral-500 dark:text-neutral-400 text-center py-8">
\t\t\t\t\tNo todos yet. Add one above.
\t\t\t\t</p>
\t\t\t) : (
\t\t\t\t<DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
\t\t\t\t\t<SortableContext items={todos.map((t) => t.id)} strategy={verticalListSortingStrategy}>
\t\t\t\t\t\t<ul className="divide-y divide-neutral-200 dark:divide-neutral-800 rounded-md border border-neutral-200 dark:border-neutral-800">
\t\t\t\t\t\t\t{todos.map((t) => (
\t\t\t\t\t\t\t\t<SortableRow
\t\t\t\t\t\t\t\t\tkey={t.id}
\t\t\t\t\t\t\t\t\ttodo={t}
\t\t\t\t\t\t\t\t\tpending={pending}
\t\t\t\t\t\t\t\t\tonToggle={() => toggle(t)}
\t\t\t\t\t\t\t\t\tonRemove={() => remove(t)}
\t\t\t\t\t\t\t\t\tonRename={(next) => rename(t, next)}
\t\t\t\t\t\t\t\t/>
\t\t\t\t\t\t\t))}
\t\t\t\t\t\t</ul>
\t\t\t\t\t</SortableContext>
\t\t\t\t</DndContext>
\t\t\t)}
\t\t</div>
\t);
}

function SortableRow({ todo, pending, onToggle, onRemove, onRename }: {
\ttodo: Todo;
\tpending: boolean;
\tonToggle: () => void;
\tonRemove: () => void;
\tonRename: (next: string) => void;
}) {
\tconst { attributes, listeners, setNodeRef, transform, transition, isDragging } =
\t\tuseSortable({ id: todo.id });
\tconst style = {
\t\ttransform: CSS.Transform.toString(transform),
\t\ttransition,
\t\topacity: isDragging ? 0.4 : 1,
\t};
\tconst [editing, setEditing] = useState(false);
\tconst [draft, setDraft] = useState(todo.title);
\tconst inputRef = useRef<HTMLInputElement>(null);

\tuseEffect(() => {
\t\tif (editing) {
\t\t\tsetDraft(todo.title);
\t\t\trequestAnimationFrame(() => {
\t\t\t\tinputRef.current?.focus();
\t\t\t\tinputRef.current?.select();
\t\t\t});
\t\t}
\t}, [editing, todo.title]);

\tfunction commit() {
\t\tsetEditing(false);
\t\tonRename(draft);
\t}

\treturn (
\t\t<li
\t\t\tref={setNodeRef}
\t\t\tstyle={style}
\t\t\tclassName="flex items-center gap-3 px-4 py-3 text-sm group bg-white dark:bg-neutral-950"
\t\t>
\t\t\t<button
\t\t\t\ttype="button"
\t\t\t\t{...attributes}
\t\t\t\t{...listeners}
\t\t\t\tclassName="cursor-grab active:cursor-grabbing text-neutral-300 hover:text-neutral-500 select-none touch-none"
\t\t\t\taria-label="Drag to reorder"
\t\t\t\ttabIndex={-1}
\t\t\t>
\t\t\t\t⋮⋮
\t\t\t</button>
\t\t\t<input
\t\t\t\ttype="checkbox"
\t\t\t\tchecked={todo.done}
\t\t\t\tonChange={onToggle}
\t\t\t\tdisabled={pending}
\t\t\t\tclassName="size-4 cursor-pointer"
\t\t\t\taria-label={\\\`Mark "\${todo.title}" as \${todo.done ? "not done" : "done"}\\\`}
\t\t\t/>
\t\t\t{editing ? (
\t\t\t\t<input
\t\t\t\t\tref={inputRef}
\t\t\t\t\tvalue={draft}
\t\t\t\t\tonChange={(e) => setDraft(e.target.value)}
\t\t\t\t\tonBlur={commit}
\t\t\t\t\tonKeyDown={(e) => {
\t\t\t\t\t\tif (e.key === "Enter") commit();
\t\t\t\t\t\telse if (e.key === "Escape") {
\t\t\t\t\t\t\tsetEditing(false);
\t\t\t\t\t\t\tsetDraft(todo.title);
\t\t\t\t\t\t}
\t\t\t\t\t}}
\t\t\t\t\tclassName="flex-1 bg-transparent border-b border-neutral-300 dark:border-neutral-700 outline-none text-sm"
\t\t\t\t\taria-label="Edit title"
\t\t\t\t/>
\t\t\t) : (
\t\t\t\t<button
\t\t\t\t\ttype="button"
\t\t\t\t\tonDoubleClick={() => setEditing(true)}
\t\t\t\t\tclassName={\\\`flex-1 text-left \${todo.done ? "line-through text-neutral-400" : ""}\\\`}
\t\t\t\t\ttitle="Double-click to edit"
\t\t\t\t>
\t\t\t\t\t{todo.title}
\t\t\t\t</button>
\t\t\t)}
\t\t\t<button
\t\t\t\ttype="button"
\t\t\t\tonClick={() => setEditing(true)}
\t\t\t\tclassName="opacity-0 group-hover:opacity-100 transition-opacity text-xs text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
\t\t\t\taria-label={\\\`Edit "\${todo.title}"\\\`}
\t\t\t>
\t\t\t\tEdit
\t\t\t</button>
\t\t\t<button
\t\t\t\ttype="button"
\t\t\t\tonClick={onRemove}
\t\t\t\tdisabled={pending}
\t\t\t\tclassName="opacity-0 group-hover:opacity-100 transition-opacity text-xs text-neutral-500 hover:text-red-500"
\t\t\t\taria-label={\\\`Delete "\${todo.title}"\\\`}
\t\t\t>
\t\t\t\tDelete
\t\t\t</button>
\t\t</li>
\t);
}
`,
);

write(
	"apps/web/next-env.d.ts",
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
	if (ua.startsWith("npm")) return "npm";
	return null;
}

/**
 * Per-package-manager workspace script syntax. Each PM exposes
 * "run X in workspace Y" differently:
 *   bun  bun run --filter ./apps/api dev
 *   pnpm pnpm --filter ./apps/api run dev
 *   yarn yarn workspace @<name>/api run dev
 *   npm  npm --workspace apps/api run dev
 *
 * The scaffold doesn't try to abstract the PM — it bakes the right
 * syntax into the generated scripts so `<pm> run dev` works
 * everywhere with no further config.
 */
function pmScripts(pm) {
	switch (pm) {
		case "bun":
			return {
				devApi: "bun run --filter './apps/api' dev",
				devWeb: "bun run --filter './apps/web' dev",
				build: "bun run --filter '*' build",
			};
		case "pnpm":
			return {
				devApi: "pnpm --filter './apps/api' run dev",
				devWeb: "pnpm --filter './apps/web' run dev",
				build: "pnpm --filter '*' run build",
			};
		case "yarn":
			return {
				devApi: `yarn workspace @${projectName}/api run dev`,
				devWeb: `yarn workspace @${projectName}/web run dev`,
				build: "yarn workspaces foreach -A run build",
			};
		case "npm":
		default:
			return {
				devApi: "npm --workspace apps/api run dev",
				devWeb: "npm --workspace apps/web run dev",
				build: "npm --workspaces run build --if-present",
			};
	}
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

Layout:
  apps/api    schema + functions/ handlers
  apps/web    Next.js 16 + React 19 + Tailwind v4
  packages/ui shared shadcn-style primitives

Next:
  - Edit apps/api/schema.ts to add entities + policies.
  - Drop handlers into apps/api/functions/ — auto-discovered by name.
  - Components go in apps/web/src/app/components/.

Docs: https://pylonsync.com/docs
`);
