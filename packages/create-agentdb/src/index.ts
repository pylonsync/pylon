#!/usr/bin/env bun

import { mkdirSync, writeFileSync, readFileSync } from "fs";
import { join, dirname } from "path";

// Read the real SDK source to embed in generated projects.
const SDK_SOURCE = (() => {
  try {
    const here = dirname(new URL(import.meta.url).pathname);
    return readFileSync(join(here, "../../../sdk/src/index.ts"), "utf-8");
  } catch {
    return "// SDK source not found. Copy from agentdb/packages/sdk/src/index.ts\nexport {};\n";
  }
})();

const args = process.argv.slice(2);
const projectName = args[0] || "my-agentdb-app";
const root = join(process.cwd(), projectName);

console.log(`\nCreating agentdb project: ${projectName}\n`);

// ---------------------------------------------------------------------------
// Directory structure
// ---------------------------------------------------------------------------

const dirs = [
  "",
  "apps/web/src/app/todos",
  "apps/web/src/lib",
  "apps/web/public",
  "packages/api/src",
  "packages/db/src",
  "packages/ui/src/components/ui",
  "packages/ui/src/lib",
];

for (const dir of dirs) {
  mkdirSync(join(root, dir), { recursive: true });
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

write("package.json", JSON.stringify({
  name: projectName,
  private: true,
  packageManager: "bun@1.2.19",
  workspaces: ["apps/*", "packages/*"],
  scripts: {
    dev: "turbo dev",
    build: "turbo build",
    lint: "turbo lint",
  },
  devDependencies: {
    turbo: "^2",
  },
}, null, 2));

write("turbo.json", JSON.stringify({
  $schema: "https://turbo.build/schema.json",
  tasks: {
    dev: { cache: false, persistent: true },
    build: {
      dependsOn: ["^build"],
      outputs: [".next/**", "!.next/cache/**", "dist/**"],
    },
    lint: {},
  },
}, null, 2));

write(".gitignore", `node_modules/
.next/
dist/
.turbo/
agentdb.dev.db
agentdb.client.ts
agentdb.manifest.json
*.db
`);

// ---------------------------------------------------------------------------
// packages/api
// ---------------------------------------------------------------------------

write("packages/api/package.json", JSON.stringify({
  name: `@${projectName}/api`,
  version: "0.1.0",
  private: true,
  type: "module",
  scripts: {
    dev: "agentdb dev src/app.ts --port 4321",
    build: "agentdb codegen src/app.ts --out src/agentdb.manifest.json && agentdb codegen client src/agentdb.manifest.json --out ../db/src/agentdb.client.ts",
    "schema:push": "agentdb schema push src/agentdb.manifest.json --sqlite dev.db",
    "schema:inspect": "agentdb schema inspect --sqlite dev.db",
  },
}, null, 2));

write("packages/api/tsconfig.json", JSON.stringify({
  compilerOptions: {
    target: "ES2022",
    module: "ESNext",
    moduleResolution: "Bundler",
    strict: true,
    skipLibCheck: true,
  },
  include: ["src"],
}, null, 2));

write("packages/api/src/app.ts", `import { entity, field, defineRoute, query, action, policy, buildManifest } from "./sdk";

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const User = entity("User", {
  email: field.string().unique(),
  displayName: field.string(),
  createdAt: field.datetime(),
});

const Todo = entity("Todo", {
  title: field.string(),
  done: field.bool(),
  authorId: field.id("User"),
  createdAt: field.datetime(),
}, {
  indexes: [
    { name: "by_author", fields: ["authorId"], unique: false },
  ],
});

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

const allTodos = query("allTodos", {
  input: [{ name: "done", type: "bool", optional: true }],
});

const todoById = query("todoById", {
  input: [{ name: "todoId", type: "id(Todo)" }],
});

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

const createTodo = action("createTodo", {
  input: [
    { name: "title", type: "string" },
    { name: "authorId", type: "id(User)" },
  ],
});

const toggleTodo = action("toggleTodo", {
  input: [{ name: "todoId", type: "id(Todo)" }],
});

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

const authenticatedCreate = policy({
  name: "authenticatedCreate",
  action: "createTodo",
  allow: "auth.userId != null",
});

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

const home = defineRoute({
  path: "/",
  mode: "server",
  query: "allTodos",
  auth: "public",
});

const todoDetail = defineRoute({
  path: "/todos/:todoId",
  mode: "server",
  query: "todoById",
  auth: "public",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "${projectName}",
  version: "0.1.0",
  entities: [User, Todo],
  queries: [allTodos, todoById],
  actions: [createTodo, toggleTodo],
  policies: [authenticatedCreate],
  routes: [home, todoDetail],
});

console.log(JSON.stringify(manifest, null, 2));
`);

write("packages/api/src/sdk.ts", SDK_SOURCE);

// ---------------------------------------------------------------------------
// packages/db
// ---------------------------------------------------------------------------

write("packages/db/package.json", JSON.stringify({
  name: `@${projectName}/db`,
  version: "0.1.0",
  private: true,
  type: "module",
  main: "src/index.ts",
  types: "src/index.ts",
  exports: {
    ".": "./src/index.ts",
    "./client": "./src/agentdb.client.ts",
  },
}, null, 2));

write("packages/db/tsconfig.json", JSON.stringify({
  compilerOptions: {
    target: "ES2022",
    module: "ESNext",
    moduleResolution: "Bundler",
    strict: true,
    skipLibCheck: true,
  },
  include: ["src"],
}, null, 2));

write("packages/db/src/index.ts", `import { createClient } from "./agentdb.client";
import type { AgentDBClient } from "./agentdb.client";

export type { AgentDBClient };
export { createClient };

const AGENTDB_URL = process.env.AGENTDB_URL ?? "http://localhost:4321";

/** Server-side client (RSC, Server Actions, API routes). */
export function createServerClient(token?: string): AgentDBClient {
  return createClient(AGENTDB_URL, token);
}

/** Browser client (uses Next.js rewrite proxy). */
export function createBrowserClient(token?: string): AgentDBClient {
  return createClient("", token);
}
`);

write("packages/db/src/agentdb.client.ts", `// Generated by agentdb. Run 'turbo build' to regenerate from schema.
// This placeholder works out of the box.

export type EntityName = string;
export type QueryName = string;
export type ActionName = string;

export interface ActionResult { action: string; input: Record<string, unknown>; executed: boolean; }
export interface Actions { [key: string]: (input: Record<string, unknown>) => Promise<ActionResult>; }

export interface AgentDBClient {
  list(entity: string): Promise<Record<string, unknown>[]>;
  get(entity: string, id: string): Promise<Record<string, unknown> | null>;
  insert(entity: string, data: Record<string, unknown>): Promise<{ id: string }>;
  update(entity: string, id: string, data: Record<string, unknown>): Promise<{ updated: boolean }>;
  remove(entity: string, id: string): Promise<{ deleted: boolean }>;
  action(name: string, input: Record<string, unknown>): Promise<ActionResult>;
  actions: Actions;
}

async function req(baseUrl: string, method: string, path: string, body?: unknown, token?: string): Promise<unknown> {
  const headers: Record<string, string> = {};
  if (body) headers["Content-Type"] = "application/json";
  if (token) headers["Authorization"] = \`Bearer \${token}\`;
  const res = await fetch(\`\${baseUrl}\${path}\`, { method, headers, body: body ? JSON.stringify(body) : undefined });
  if (!res.ok) {
    const err = await res.json().catch(() => ({})) as Record<string, unknown>;
    const errorObj = err?.error as Record<string, unknown> | undefined;
    throw new Error((errorObj?.message as string) ?? \`HTTP \${res.status}\`);
  }
  return res.json();
}

export function createClient(baseUrl = "http://localhost:4321", token?: string): AgentDBClient {
  const r = (method: string, path: string, body?: unknown) => req(baseUrl, method, path, body, token);
  return {
    list: (entity) => r("GET", \`/api/entities/\${entity}\`) as Promise<Record<string, unknown>[]>,
    get: (entity, id) => r("GET", \`/api/entities/\${entity}/\${id}\`).then(x => x as Record<string, unknown>).catch(() => null),
    insert: (entity, data) => r("POST", \`/api/entities/\${entity}\`, data) as Promise<{ id: string }>,
    update: (entity, id, data) => r("PATCH", \`/api/entities/\${entity}/\${id}\`, data) as Promise<{ updated: boolean }>,
    remove: (entity, id) => r("DELETE", \`/api/entities/\${entity}/\${id}\`) as Promise<{ deleted: boolean }>,
    action: (name, input) => r("POST", \`/api/actions/\${name}\`, input) as Promise<ActionResult>,
    actions: new Proxy({} as Actions, { get: (_, n: string) => (input: Record<string, unknown>) => r("POST", \`/api/actions/\${n}\`, input) }),
  };
}

export function createServerClient(baseUrl?: string, token?: string): AgentDBClient {
  return createClient(baseUrl ?? process.env.AGENTDB_URL ?? "http://localhost:4321", token);
}
`);

// ---------------------------------------------------------------------------
// packages/ui — shared UI components (shadcn compatible)
// ---------------------------------------------------------------------------

write("packages/ui/package.json", JSON.stringify({
  name: `@${projectName}/ui`,
  version: "0.1.0",
  private: true,
  type: "module",
  main: "src/index.ts",
  types: "src/index.ts",
  exports: {
    ".": "./src/index.ts",
    "./components/*": "./src/components/*",
    "./lib/*": "./src/lib/*",
  },
  dependencies: {
    "class-variance-authority": "^0.7",
    clsx: "^2",
    "tailwind-merge": "^3",
  },
}, null, 2));

write("packages/ui/tsconfig.json", JSON.stringify({
  compilerOptions: {
    target: "ES2022",
    module: "ESNext",
    moduleResolution: "Bundler",
    strict: true,
    skipLibCheck: true,
    jsx: "preserve",
    paths: { "@/*": ["./src/*"] },
  },
  include: ["src"],
}, null, 2));

write("packages/ui/src/index.ts", `export { cn } from "./lib/utils";
export { Button } from "./components/ui/button";
export { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from "./components/ui/card";
export { Input } from "./components/ui/input";
`);

write("packages/ui/src/lib/utils.ts", `import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
`);

// Basic shadcn-style components
write("packages/ui/src/components/ui/button.tsx", `import { cn } from "../../lib/utils";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "destructive" | "outline" | "ghost";
  size?: "default" | "sm" | "lg";
}

export function Button({ className, variant = "default", size = "default", ...props }: ButtonProps) {
  return (
    <button
      className={cn(
        "inline-flex items-center justify-center rounded-md font-medium transition-colors",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2",
        "disabled:pointer-events-none disabled:opacity-50",
        variant === "default" && "bg-primary text-primary-foreground hover:bg-primary/90",
        variant === "destructive" && "bg-destructive text-destructive-foreground hover:bg-destructive/90",
        variant === "outline" && "border border-input bg-background hover:bg-accent hover:text-accent-foreground",
        variant === "ghost" && "hover:bg-accent hover:text-accent-foreground",
        size === "default" && "h-10 px-4 py-2 text-sm",
        size === "sm" && "h-9 px-3 text-xs",
        size === "lg" && "h-11 px-8 text-base",
        className,
      )}
      {...props}
    />
  );
}
`);

write("packages/ui/src/components/ui/card.tsx", `import { cn } from "../../lib/utils";

export function Card({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("rounded-lg border bg-card text-card-foreground shadow-sm", className)} {...props} />;
}

export function CardHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("flex flex-col space-y-1.5 p-6", className)} {...props} />;
}

export function CardTitle({ className, ...props }: React.HTMLAttributes<HTMLHeadingElement>) {
  return <h3 className={cn("text-2xl font-semibold leading-none tracking-tight", className)} {...props} />;
}

export function CardDescription({ className, ...props }: React.HTMLAttributes<HTMLParagraphElement>) {
  return <p className={cn("text-sm text-muted-foreground", className)} {...props} />;
}

export function CardContent({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("p-6 pt-0", className)} {...props} />;
}

export function CardFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("flex items-center p-6 pt-0", className)} {...props} />;
}
`);

write("packages/ui/src/components/ui/input.tsx", `import { cn } from "../../lib/utils";

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {}

export function Input({ className, ...props }: InputProps) {
  return (
    <input
      className={cn(
        "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm",
        "ring-offset-background placeholder:text-muted-foreground",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}
`);

// shadcn components.json for the ui package
write("packages/ui/components.json", JSON.stringify({
  $schema: "https://ui.shadcn.com/schema.json",
  style: "new-york",
  rsc: true,
  tsx: true,
  tailwind: {
    config: "",
    css: "../../apps/web/src/app/globals.css",
    baseColor: "neutral",
    cssVariables: true,
  },
  aliases: {
    components: "@/components",
    utils: "@/lib/utils",
    ui: "@/components/ui",
  },
}, null, 2));

// ---------------------------------------------------------------------------
// apps/web
// ---------------------------------------------------------------------------

write("apps/web/package.json", JSON.stringify({
  name: `@${projectName}/web`,
  version: "0.1.0",
  private: true,
  scripts: {
    dev: "next dev --port 3000",
    build: "next build",
    start: "next start",
    lint: "next lint",
  },
  dependencies: {
    [`@${projectName}/db`]: "workspace:*",
    [`@${projectName}/ui`]: "workspace:*",
    next: "^16.0.0",
    react: "^19.0.0",
    "react-dom": "^19.0.0",
    tailwindcss: "^4.0.0",
    "@tailwindcss/postcss": "^4.0.0",
  },
}, null, 2));

write("apps/web/tsconfig.json", JSON.stringify({
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
  include: ["next-env.d.ts", "src/**/*.ts", "src/**/*.tsx"],
  exclude: ["node_modules"],
}, null, 2));

write("apps/web/postcss.config.mjs", `/** @type {import('postcss-load-config').Config} */
const config = {
  plugins: {
    "@tailwindcss/postcss": {},
  },
};
export default config;
`);

write("apps/web/src/app/globals.css", `@import "tailwindcss";

@theme {
  --color-background: #ffffff;
  --color-foreground: #0a0a0a;
  --color-card: #ffffff;
  --color-card-foreground: #0a0a0a;
  --color-primary: #171717;
  --color-primary-foreground: #fafafa;
  --color-secondary: #f5f5f5;
  --color-secondary-foreground: #171717;
  --color-muted: #f5f5f5;
  --color-muted-foreground: #737373;
  --color-accent: #f5f5f5;
  --color-accent-foreground: #171717;
  --color-destructive: #ef4444;
  --color-destructive-foreground: #fafafa;
  --color-border: #e5e5e5;
  --color-input: #e5e5e5;
  --color-ring: #0a0a0a;
  --radius: 0.5rem;
}

body {
  background: var(--color-background);
  color: var(--color-foreground);
  font-family: system-ui, sans-serif;
}
`);

write("apps/web/next.config.ts", `import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  transpilePackages: ["@${projectName}/db", "@${projectName}/ui"],
  async rewrites() {
    return [{ source: "/api/:path*", destination: "http://localhost:4321/api/:path*" }];
  },
};

export default nextConfig;
`);

write("apps/web/src/app/layout.tsx", `import "./globals.css";

export const metadata = {
  title: "${projectName}",
  description: "Built with agentdb + Next.js",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="min-h-screen bg-background antialiased">
        <main className="max-w-2xl mx-auto px-4 py-10">
          {children}
        </main>
      </body>
    </html>
  );
}
`);

write("apps/web/src/app/page.tsx", `import { createServerClient } from "@${projectName}/db";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@${projectName}/ui";
import { TodoList } from "./todos/TodoList";
import { AddTodoForm } from "./todos/AddTodoForm";

export default async function Home() {
  const db = createServerClient();
  const todos = await db.list("Todo");

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">${projectName}</h1>
        <p className="text-muted-foreground mt-1">Built with agentdb + Next.js + Turborepo</p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>Todos</CardTitle>
          <CardDescription>Manage your tasks with real-time sync.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <AddTodoForm />
          <TodoList initialTodos={todos} />
        </CardContent>
      </Card>
    </div>
  );
}
`);

write("apps/web/src/app/actions.ts", `"use server";

import { createServerClient } from "@${projectName}/db";
import { revalidatePath } from "next/cache";

const db = createServerClient();

export async function createTodo(formData: FormData) {
  const title = formData.get("title") as string;
  if (!title) return;

  await db.insert("Todo", {
    title,
    done: false,
    authorId: "default-user",
    createdAt: new Date().toISOString(),
  });

  revalidatePath("/");
}

export async function toggleTodo(todoId: string) {
  const todo = await db.get("Todo", todoId);
  if (!todo) return;

  await db.update("Todo", todoId, {
    done: todo.done === 1 || todo.done === true ? false : true,
  });

  revalidatePath("/");
}

export async function deleteTodo(todoId: string) {
  await db.remove("Todo", todoId);
  revalidatePath("/");
}
`);

write("apps/web/src/app/todos/TodoList.tsx", `"use client";

import { toggleTodo, deleteTodo } from "../actions";
import { Button } from "@${projectName}/ui";

type Row = Record<string, unknown>;

export function TodoList({ initialTodos }: { initialTodos: Row[] }) {
  return (
    <div className="space-y-1">
      {initialTodos.length === 0 && (
        <p className="text-sm text-muted-foreground py-4 text-center">No todos yet. Add one above!</p>
      )}
      {initialTodos.map((todo) => {
        const done = todo.done === 1 || todo.done === true;
        return (
          <div key={todo.id as string} className="flex items-center gap-3 py-2 border-b border-border last:border-0">
            <button onClick={() => toggleTodo(todo.id as string)} className="text-lg">
              {done ? "✅" : "⬜"}
            </button>
            <span className={\`flex-1 text-sm \${done ? "line-through text-muted-foreground" : ""}\`}>
              {todo.title as string}
            </span>
            <Button variant="ghost" size="sm" onClick={() => deleteTodo(todo.id as string)} className="text-destructive hover:text-destructive">
              ✕
            </Button>
          </div>
        );
      })}
    </div>
  );
}
`);

write("apps/web/src/app/todos/AddTodoForm.tsx", `"use client";

import { createTodo } from "../actions";
import { useRef } from "react";
import { Button, Input } from "@${projectName}/ui";

export function AddTodoForm() {
  const formRef = useRef<HTMLFormElement>(null);

  return (
    <form
      ref={formRef}
      action={async (formData: FormData) => {
        await createTodo(formData);
        formRef.current?.reset();
      }}
      className="flex gap-2"
    >
      <Input name="title" placeholder="What needs to be done?" required className="flex-1" />
      <Button type="submit">Add</Button>
    </form>
  );
}
`);

write("README.md", `# ${projectName}

Built with [agentdb](https://github.com/agentdb/agentdb) + Next.js + Turborepo.

## Structure

\`\`\`
${projectName}/
├── apps/
│   └── web/                 Next.js 16 frontend
│       └── src/
│           ├── app/         App Router pages + Server Actions
│           └── lib/         Utilities
├── packages/
│   ├── api/                 agentdb schema definition
│   │   └── src/
│   │       └── app.ts       entities, queries, actions, policies
│   └── db/                  shared database client
│       └── src/
│           ├── index.ts     server/browser client helpers
│           └── agentdb.client.ts  generated typed client
├── turbo.json
└── package.json
\`\`\`

## Getting Started

\`\`\`bash
bun install
bun run dev
\`\`\`

- **App**: http://localhost:3000
- **Studio**: http://localhost:4321/studio
- **API**: http://localhost:4321/api/entities/<entity>
`);

// ---------------------------------------------------------------------------
// Done
// ---------------------------------------------------------------------------

console.log(`Created ${projectName}/`);
console.log();
console.log(`  apps/`);
console.log(`    web/src/             Next.js 16 frontend`);
console.log(`  packages/`);
console.log(`    api/src/             agentdb schema + dev server`);
console.log(`    db/src/              shared database client`);
console.log(`    ui/src/              shadcn-style UI components`);
console.log(`  turbo.json             Turborepo config`);
console.log();
console.log(`Next steps:`);
console.log(`  cd ${projectName}`);
console.log(`  bun install`);
console.log(`  bun run dev`);
console.log();

function write(path: string, content: string) {
  writeFileSync(join(root, path), content);
}
