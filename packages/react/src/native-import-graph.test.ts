// The React Native entry points must bundle under Metro. Metro resolves
// every static and dynamic import at build time and has no `import.meta`,
// so one Node-only module anywhere in the runtime import graph breaks
// every native build. This walks the graph from the two client entries
// through the workspace packages and fails on the first offender.
//
// It caught the sdk root (`discoverAppRoutes` uses `import.meta.url` and
// `node:module`) being pulled in through a `defineRoute` re-export.

import { expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const PACKAGES = resolve(import.meta.dir, "..", "..");
const ENTRIES = [
  join(PACKAGES, "react", "src", "index.ts"),
  join(PACKAGES, "react-native", "src", "index.ts"),
  join(PACKAGES, "plugins", "revenuecat", "src", "client.ts"),
];
const NODE_BUILTINS = new Set([
  "assert", "buffer", "child_process", "crypto", "events", "fs", "http",
  "https", "module", "net", "os", "path", "stream", "url", "util",
  "worker_threads", "zlib",
]);

function packageDir(name: string): string {
  const plugin = join(PACKAGES, "plugins", name);
  return existsSync(plugin) ? plugin : join(PACKAGES, name);
}

/** Resolve an import specifier to a workspace source file, or null for
 *  an external dependency (react, react-native, async-storage, ...). */
function resolveImport(spec: string, fromFile: string): string | null {
  if (spec.startsWith(".")) {
    const base = resolve(dirname(fromFile), spec);
    for (const candidate of [base, `${base}.ts`, `${base}.tsx`, join(base, "index.ts")]) {
      if (existsSync(candidate) && !candidate.endsWith("/")) {
        try {
          readFileSync(candidate);
          return candidate;
        } catch {
          /* directory */
        }
      }
    }
    throw new Error(`cannot resolve ${spec} from ${fromFile}`);
  }
  const m = /^@pylonsync\/([^/]+)(?:\/(.+))?$/.exec(spec);
  if (!m) return null;
  const dir = packageDir(m[1]);
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as {
    main?: string;
    exports?: Record<string, string | { default?: string }>;
  };
  const key = m[2] ? `./${m[2]}` : ".";
  const entry = pkg.exports?.[key];
  const target =
    typeof entry === "string" ? entry : entry?.default ?? (key === "." ? pkg.main : undefined);
  if (!target) throw new Error(`${spec}: no "${key}" export in ${dir}/package.json`);
  return resolve(dir, target);
}

test("the React Native entry points import no Node-only code", () => {
  const seen = new Set<string>();
  const queue = [...ENTRIES];
  const offenders: string[] = [];
  const rel = (f: string) => relative(PACKAGES, f);

  while (queue.length > 0) {
    const file = queue.pop()!;
    if (seen.has(file)) continue;
    seen.add(file);
    const source = readFileSync(file, "utf8");
    // Erase types first so `import type` / `export type ... from` do not
    // count — Babel drops them before Metro ever sees the specifier.
    const loader = file.endsWith(".tsx") ? "tsx" : "ts";
    const js = new Bun.Transpiler({ loader }).transformSync(source);
    if (/\bimport\.meta\b/.test(js)) offenders.push(`${rel(file)} uses import.meta`);
    for (const imp of new Bun.Transpiler({ loader: "js" }).scanImports(js)) {
      const spec = imp.path;
      if (spec.startsWith("node:") || NODE_BUILTINS.has(spec)) {
        offenders.push(`${rel(file)} imports ${spec}`);
        continue;
      }
      const next = resolveImport(spec, file);
      if (next) queue.push(next);
    }
  }

  expect(offenders).toEqual([]);
  // Sanity: the walk actually crossed into @pylonsync/sync and the sdk.
  expect([...seen].some((f) => f.includes("/sync/src/"))).toBe(true);
  expect([...seen].some((f) => f.endsWith("/sdk/src/route.ts"))).toBe(true);
  expect([...seen].some((f) => f.endsWith("/sdk/src/index.ts"))).toBe(false);
});
