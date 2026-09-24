// `pylon build` — the production artifact.
//
// The Rust CLI evaluates app.ts into `pylon.manifest.json`, then runs this
// module (through build-cli.ts) to write a directory that `pylon start <dir>`
// runs with no source tree and no `node_modules`:
//
//   <out>/
//     pylon-build.json      build info; `pylon start` detects an artifact by it
//     pylon.manifest.json   the app manifest
//     server/runner.js      entry: installs the stdout fence, loads the rest
//     server/chunks/        functions, SSR runtime, and app modules
//     server/assets/        files the server reads at run time (OG image wasm, fonts)
//     server/node_modules/  only the packages listed in build.server.external
//     client/               hydration bundle (served under /_pylon/build/)
//     public/               copy of public/
//     app/                  non-code files under app/ (icons, social images)
//
// The Rust CLI adds `static/` (static routes), `web/dist` (a web/ SPA), and
// studio files after this returns.

import { _doBuildInner, CLIENT_BUILD_DIR } from "./ssr-client-bundler";
import type { ManifestBuildConfig } from "./build-compat";

declare const Bun: {
  resolveSync(specifier: string, from: string): string;
  build(options: any): Promise<any>;
};

/** File name of the build info file. Its presence marks an artifact dir. */
export const BUILD_INFO_FILE = "pylon-build.json";

/** App modules the SSR runtime imports by convention, by base name. */
const APP_MODULE_NAMES = new Set([
  "page",
  "layout",
  "not-found",
  "error",
  "loading",
  "route",
  "opengraph-image",
  "sitemap",
  "robots",
  "llms",
]);
const CODE_EXTS = [".tsx", ".ts", ".jsx", ".js"];
/** Non-code files under app/ that the runtime reads from disk. */
const APP_ASSET_EXTS = new Set([
  ".png",
  ".jpg",
  ".jpeg",
  ".webp",
  ".gif",
  ".avif",
  ".svg",
  ".ico",
  ".txt",
  ".xml",
  ".webmanifest",
]);

export interface ProductionBuildOptions {
  /** Project root. */
  cwd: string;
  /** Absolute output directory. */
  outDir: string;
  /** App (routes) directory, relative to cwd. Default `app`. */
  appDir?: string;
  /** Functions directory, relative to cwd. Default `functions`. */
  functionsDir?: string;
  /** The app manifest. Default `<cwd>/pylon.manifest.json`. */
  manifestPath?: string;
}

export interface ProductionBuildResult {
  outDir: string;
  functions: number;
  workflows: number;
  appModules: number;
  clientRoutes: number;
  externals: string[];
  polyfills: boolean;
}

export async function buildProduction(
  opts: ProductionBuildOptions,
): Promise<ProductionBuildResult> {
  const fs: any = await import("node:fs");
  const path: any = await import("node:path");
  const url: any = await import("node:url");
  const { cwd, outDir } = opts;
  const appDir = opts.appDir ?? "app";
  const functionsDir = opts.functionsDir ?? "functions";

  const manifestPath = opts.manifestPath ?? path.join(cwd, "pylon.manifest.json");
  if (!fs.existsSync(manifestPath)) {
    throw new Error(`${manifestPath} not found. Run \`pylon build\`, which writes it first.`);
  }
  const manifestRaw = fs.readFileSync(manifestPath, "utf8");
  const manifest = JSON.parse(manifestRaw);
  const build: ManifestBuildConfig = manifest.build ?? {};

  prepareOutDir(fs, path, cwd, outDir);

  // ---- client --------------------------------------------------------------
  const appAbs = path.join(cwd, appDir);
  const hasApp = fs.existsSync(appAbs) && fs.statSync(appAbs).isDirectory();
  let clientRoutes = 0;
  let polyfills = false;
  const clientOut = path.join(outDir, "client");
  if (hasApp && hasPageModules(fs, path, appAbs)) {
    const { manifestPath: clientManifest } = await _doBuildInner(fs, path, cwd, appDir, {
      outdir: clientOut,
      manifestOutdir: "client",
      buildConfig: build,
      strict: true,
      manifestPath,
    });
    const cm = JSON.parse(fs.readFileSync(clientManifest, "utf8"));
    clientRoutes = Object.keys(cm.routes ?? {}).length;
    polyfills = typeof cm.polyfills === "string";
    // `pylon start` serves this bundle as-is and never rebuilds it.
    fs.writeFileSync(path.join(clientOut, ".prebuilt"), "", "utf8");
  }

  // ---- server --------------------------------------------------------------
  const here = path.dirname(url.fileURLToPath(import.meta.url));
  const functions = listModules(fs, path, path.join(cwd, functionsDir));
  const workflows = listModules(fs, path, path.join(cwd, "workflows"));
  const appModules = hasApp ? collectAppModules(fs, path, cwd, appDir, manifest) : [];

  const stage = path.join(cwd, ".pylon", "server-entry");
  fs.rmSync(stage, { recursive: true, force: true });
  fs.mkdirSync(stage, { recursive: true });
  const spec = (abs: string) => JSON.stringify(abs.replace(/\\/g, "/"));

  const resvgWasm = Bun.resolveSync("@resvg/resvg-wasm/index_bg.wasm", here);
  const fontsDir = path.join(here, "..", "assets", "fonts");

  fs.writeFileSync(
    path.join(stage, "registry.ts"),
    generateRegistry({
      spec,
      path,
      serverBundleModule: path.join(here, "server-bundle.ts"),
      functions,
      workflows,
      appModules: appModules.map((m) => ({ key: m.key, file: m.file })),
      clientDir: hasApp && clientRoutes > 0 ? "client" : CLIENT_BUILD_DIR,
      resvgWasm,
      interRegular: path.join(fontsDir, "Inter-Regular.ttf"),
      interSemiBold: path.join(fontsDir, "Inter-SemiBold.ttf"),
    }),
    "utf8",
  );
  // Two stages, joined by dynamic imports so the order holds however Bun
  // splits the chunks (a static import is hoisted above the importing
  // module's own code):
  //   1. runner.ts installs the stdout fence, then loads main.ts. Everything
  //      main.ts reaches, including the static imports of external packages,
  //      is in later chunks, so nothing a package prints at load time can
  //      reach the protocol stream.
  //   2. main.ts sets the registry, then loads the runtime, whose top-level
  //      main() reads it.
  fs.writeFileSync(
    path.join(stage, "main.ts"),
    [
      "// Generated by `pylon build`.",
      `import "./registry.ts";`,
      `await import(${spec(path.join(here, "runtime.ts"))});`,
      "",
    ].join("\n"),
    "utf8",
  );
  const entry = path.join(stage, "runner.ts");
  fs.writeFileSync(
    entry,
    [
      "// Generated by `pylon build`.",
      `import { fenceStdout } from ${spec(path.join(here, "stdout-fence.ts"))};`,
      "fenceStdout();",
      `await import("./main.ts");`,
      "",
    ].join("\n"),
    "utf8",
  );

  const serverOut = path.join(outDir, "server");
  const bundleAll = build.server?.bundle !== false;
  const externals = bundleAll ? build.server?.external ?? [] : [];
  const result = await Bun.build({
    entrypoints: [entry],
    outdir: serverOut,
    root: cwd,
    target: "bun",
    format: "esm",
    // Keep identifiers: server stack traces stay readable.
    minify: { whitespace: true, syntax: true },
    sourcemap: build.sourcemap === true ? "linked" : "none",
    splitting: true,
    // `pkg/*` keeps subpath imports (`pkg/server`) external too.
    ...(bundleAll
      ? { external: externals.flatMap((e) => [e, `${e}/*`]) }
      : { packages: "external" }),
    define: { "process.env.NODE_ENV": JSON.stringify("production") },
    naming: {
      entry: "[name].js",
      chunk: "chunks/[name]-[hash].js",
      asset: "assets/[name]-[hash].[ext]",
    },
  });
  if (!result.success) {
    const msgs = (result.logs ?? [])
      .map((l: any) => `${l.level}: ${l.message}`)
      .join("\n");
    throw new Error(`server bundle failed:\n${msgs || "(no log messages)"}`);
  }

  // Packages left out of the bundle ship in server/node_modules with their
  // dependencies. `server.bundle: false` leaves out every package.
  const externalRoots = bundleAll
    ? externals
    : Object.keys(readPackageJson(fs, path, cwd)?.dependencies ?? {});
  const copied = copyPackageClosure(fs, path, cwd, externalRoots, path.join(serverOut, "node_modules"));

  // ---- static files ----------------------------------------------------------
  const publicDir = path.join(cwd, "public");
  if (fs.existsSync(publicDir)) {
    fs.cpSync(publicDir, path.join(outDir, "public"), { recursive: true, dereference: true });
  }
  if (hasApp) copyAppAssets(fs, path, appAbs, path.join(outDir, appDir));
  for (const inc of build.include ?? []) {
    const src = path.resolve(cwd, inc);
    const rel = checkInclude(path, cwd, outDir, inc, src);
    if (!fs.existsSync(src)) throw new Error(`build.include: "${inc}" does not exist`);
    fs.cpSync(src, path.join(outDir, rel), { recursive: true, dereference: true });
  }
  // Shard modules, at the paths app.ts gives, which the runtime reads at boot.
  for (const def of (manifest.shards ?? []) as Array<{ name: string; wasm: string }>) {
    const src = path.resolve(cwd, def.wasm);
    const rel = checkInclude(path, cwd, outDir, def.wasm, src, `shard "${def.name}" wasm`);
    if (!fs.existsSync(src)) {
      throw new Error(
        `shard "${def.name}": ${def.wasm} does not exist; run \`pylon shards build\` first`,
      );
    }
    fs.mkdirSync(path.dirname(path.join(outDir, rel)), { recursive: true });
    fs.copyFileSync(src, path.join(outDir, rel));
  }

  fs.writeFileSync(path.join(outDir, "pylon.manifest.json"), manifestRaw, "utf8");
  const pkg = JSON.parse(fs.readFileSync(path.join(here, "..", "package.json"), "utf8"));
  fs.writeFileSync(
    path.join(outDir, BUILD_INFO_FILE),
    JSON.stringify(
      {
        format: 1,
        app: manifest.name,
        appVersion: manifest.version,
        pylonFunctions: pkg.version,
        builtAt: new Date().toISOString(),
        server: "server/runner.js",
        client: clientRoutes > 0 ? "client" : null,
        target: build.target ?? null,
        polyfill: build.polyfill ?? false,
        externals: copied,
      },
      null,
      2,
    ) + "\n",
    "utf8",
  );
  fs.rmSync(stage, { recursive: true, force: true });

  return {
    outDir,
    functions: functions.length,
    workflows: workflows.length,
    appModules: appModules.length,
    clientRoutes,
    externals: copied,
    polyfills,
  };
}

/**
 * Empty the output directory. Refuses a directory that is the project root,
 * contains it, or holds files and is not an earlier artifact: that is someone
 * else's data.
 */
function prepareOutDir(fs: any, path: any, cwd: string, outDir: string): void {
  // `cwd` relative to `outDir`: "" or a path without a leading ".." means
  // `outDir` is the project dir or an ancestor of it. On Windows, another
  // drive gives an absolute path, which is neither.
  const rel = path.relative(outDir, cwd);
  if (rel === "" || (!rel.startsWith("..") && !path.isAbsolute(rel))) {
    throw new Error(`--out ${outDir} is the project directory or contains it`);
  }
  if (fs.existsSync(outDir)) {
    const entries = fs.readdirSync(outDir);
    if (entries.length > 0 && !entries.includes(BUILD_INFO_FILE)) {
      throw new Error(
        `${outDir} is not empty and holds no ${BUILD_INFO_FILE}, so it is not an ` +
          `earlier build artifact. Remove it or choose another --out directory. ` +
          `(A pylon build before 0.12 wrote static pages there; they now go to static/ inside the artifact.)`,
      );
    }
    fs.rmSync(outDir, { recursive: true, force: true });
  }
  fs.mkdirSync(outDir, { recursive: true });
}

/** Top-level artifact names that `build.include` must not write into. */
const RESERVED_INCLUDE_NAMES = new Set([
  BUILD_INFO_FILE,
  "pylon.manifest.json",
  "server",
  "client",
  "static",
  "web",
  "bin",
  ".pylon",
]);

/** The artifact-relative path for one `build.include` entry. Throws when the
 *  entry cannot be copied into the artifact as-is. */
export function checkInclude(
  path: any,
  cwd: string,
  outDir: string,
  inc: string,
  src: string,
  label = "build.include",
): string {
  const rel = path.relative(cwd, src);
  const isOutside = (r: string) =>
    r === ".." || r.startsWith(`..${path.sep}`) || r.startsWith("../") || path.isAbsolute(r);
  if (rel === "") throw new Error(`${label}: "${inc}" is the project directory`);
  if (isOutside(rel)) {
    throw new Error(`${label}: "${inc}" is outside the project directory`);
  }
  // Neither the output directory nor anything that contains it or is in it.
  if (!isOutside(path.relative(src, outDir)) || !isOutside(path.relative(outDir, src))) {
    throw new Error(`${label}: "${inc}" overlaps the output directory`);
  }
  const top = rel.split(/[\\/]/)[0];
  if (RESERVED_INCLUDE_NAMES.has(top)) {
    throw new Error(`${label}: "${inc}" would write into the artifact's own "${top}"`);
  }
  return rel;
}

function hasPageModules(fs: any, path: any, dir: string): boolean {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.isDirectory()) {
      if (e.name.startsWith(".") || e.name === "node_modules") continue;
      if (hasPageModules(fs, path, path.join(dir, e.name))) return true;
    } else if (CODE_EXTS.some((x) => e.name === `page${x}`)) {
      return true;
    }
  }
  return false;
}

/** `.ts` / `.js` files directly in `dir`, as the runtime loads them. */
function listModules(fs: any, path: any, dir: string): Array<{ name: string; file: string }> {
  if (!fs.existsSync(dir)) return [];
  return fs
    .readdirSync(dir)
    .filter((f: string) => f.endsWith(".ts") || f.endsWith(".js"))
    .sort()
    .map((f: string) => ({
      name: path.basename(f, f.endsWith(".ts") ? ".ts" : ".js"),
      file: path.join(dir, f),
    }));
}

/**
 * Every app module the SSR runtime can import: files named by convention
 * anywhere under the app dir, plus every component and layout the manifest
 * routes name. Keys are the runtime's module paths (`app/blog/page`).
 */
export function collectAppModules(
  fs: any,
  path: any,
  cwd: string,
  appDir: string,
  manifest: any,
): Array<{ key: string; file: string }> {
  const found = new Map<string, string>();
  const keyOf = (abs: string) =>
    path.relative(cwd, abs).replace(/\\/g, "/").replace(/\.(tsx?|jsx?)$/, "");
  const walk = (dir: string) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const abs = path.join(dir, e.name);
      if (e.isDirectory()) {
        if (e.name.startsWith(".") || e.name === "node_modules") continue;
        walk(abs);
        continue;
      }
      const ext = CODE_EXTS.find((x) => e.name.endsWith(x));
      if (!ext) continue;
      const base = e.name.slice(0, -ext.length);
      if (!APP_MODULE_NAMES.has(base)) continue;
      const key = keyOf(abs);
      // Same precedence as the runtime's extension probe: .tsx, .ts, .jsx, .js.
      const prev = found.get(key);
      if (!prev || CODE_EXTS.indexOf(ext) < CODE_EXTS.findIndex((x) => prev.endsWith(x))) {
        found.set(key, abs);
      }
    }
  };
  walk(path.join(cwd, appDir));

  const named = new Set<string>();
  for (const r of manifest.routes ?? []) {
    if (typeof r.component === "string") named.add(r.component);
    for (const l of r.layouts ?? []) if (typeof l === "string") named.add(l);
  }
  for (const n of named) {
    const key = n.replace(/\\/g, "/");
    if (found.has(key)) continue;
    const ext = CODE_EXTS.find((x) => fs.existsSync(path.join(cwd, `${key}${x}`)));
    if (ext) found.set(key, path.join(cwd, `${key}${ext}`));
  }
  return [...found.entries()]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([key, file]) => ({ key, file }));
}

function generateRegistry(args: {
  spec: (abs: string) => string;
  path: any;
  serverBundleModule: string;
  functions: Array<{ name: string; file: string }>;
  workflows: Array<{ name: string; file: string }>;
  appModules: Array<{ key: string; file: string }>;
  clientDir: string;
  resvgWasm: string;
  interRegular: string;
  interSemiBold: string;
}): string {
  const { spec } = args;
  const lines: string[] = [
    "// Generated by `pylon build`. The production server's module registry.",
    `import { setServerBundle } from ${spec(args.serverBundleModule)};`,
    `import { isAbsolute } from "node:path";`,
    `import { fileURLToPath } from "node:url";`,
    `import * as React from "react";`,
    `import * as ReactDomServer from "react-dom/server.browser";`,
    `import resvgWasm from ${spec(args.resvgWasm)} with { type: "file" };`,
    `import interRegular from ${spec(args.interRegular)} with { type: "file" };`,
    `import interSemiBold from ${spec(args.interSemiBold)} with { type: "file" };`,
  ];
  const obj = (items: string[]) => (items.length ? `{\n${items.join("\n")}\n  }` : "{}");
  lines.push(
    "",
    "// A file import is a path relative to this bundle once bundled.",
    "const assetPath = (p: string) =>",
    "  isAbsolute(p) ? p : fileURLToPath(new URL(p, import.meta.url));",
    "",
    "setServerBundle({",
    `  functions: ${obj(args.functions.map((f) => `    ${JSON.stringify(f.name)}: () => import(${spec(f.file)}),`))},`,
    `  workflows: ${obj(args.workflows.map((w) => `    ${JSON.stringify(w.name)}: () => import(${spec(w.file)}),`))},`,
    `  modules: ${obj(args.appModules.map((m) => `    ${JSON.stringify(m.key)}: () => import(${spec(m.file)}),`))},`,
    "  react: React,",
    "  reactDomServer: ReactDomServer,",
    `  clientDir: ${JSON.stringify(args.clientDir)},`,
    "  ogAssets: {",
    "    resvgWasm: assetPath(resvgWasm),",
    "    interRegular: assetPath(interRegular),",
    "    interSemiBold: assetPath(interSemiBold),",
    "  },",
    "});",
    "",
  );
  return lines.join("\n");
}

function readPackageJson(fs: any, path: any, dir: string): any {
  const p = path.join(dir, "package.json");
  if (!fs.existsSync(p)) return null;
  return JSON.parse(fs.readFileSync(p, "utf8"));
}

/** The directory of an installed package, resolved from `fromDir` the way
 *  Node and Bun resolve: the nearest `node_modules/<name>` walking up. */
function findPackageDir(fs: any, path: any, name: string, fromDir: string): string | null {
  let dir = fromDir;
  for (;;) {
    const candidate = path.join(dir, "node_modules", name);
    if (fs.existsSync(path.join(candidate, "package.json"))) return fs.realpathSync(candidate);
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

/**
 * Copy `roots` and their dependency closure (dependencies, optional
 * dependencies that are installed, and peers) into `dest` (a node_modules
 * dir).
 * Each package resolves its own dependencies from its install location, so
 * npm, pnpm, and Bun layouts all work. A package goes to the top of `dest`
 * unless another version already holds that name; then it nests under the
 * dependent package's own `node_modules`, where run-time resolution finds it
 * first. Returns the copied package names.
 */
export function copyPackageClosure(
  fs: any,
  path: any,
  cwd: string,
  roots: string[],
  dest: string,
): string[] {
  // Installed package dir → the dir it was copied to.
  const copiedTo = new Map<string, string>();
  // Top-level name in `dest` → the installed dir that holds it.
  const topLevel = new Map<string, string>();
  const names = new Set<string>();
  type Item = { name: string; from: string; parentCopy: string | null; required: boolean };
  const queue: Item[] = roots.map((name) => ({ name, from: cwd, parentCopy: null, required: true }));
  while (queue.length > 0) {
    const { name, from, parentCopy, required } = queue.shift()!;
    const dir = findPackageDir(fs, path, name, from);
    if (!dir) {
      if (required) {
        throw new Error(
          `package "${name}" (needed by ${from === cwd ? "build.server" : from}) is not installed`,
        );
      }
      continue; // an optional dependency that is not installed
    }
    let target: string;
    const holder = topLevel.get(name);
    if (holder === undefined) {
      target = path.join(dest, name);
      topLevel.set(name, dir);
    } else if (holder === dir) {
      continue; // this exact install is already at the top level
    } else if (parentCopy !== null) {
      target = path.join(parentCopy, "node_modules", name);
    } else {
      throw new Error(`two installs of "${name}" are both build roots`);
    }
    if (copiedTo.get(dir) === target || fs.existsSync(target)) continue;
    copiedTo.set(dir, target);
    names.add(name);
    fs.cpSync(dir, target, {
      recursive: true,
      dereference: true,
      filter: (src: string) =>
        !src.slice(dir.length).split(/[\\/]/).includes("node_modules"),
    });
    const pkg = readPackageJson(fs, path, dir) ?? {};
    for (const dep of Object.keys(pkg.dependencies ?? {})) {
      queue.push({ name: dep, from: dir, parentCopy: target, required: true });
    }
    for (const dep of Object.keys(pkg.optionalDependencies ?? {})) {
      queue.push({ name: dep, from: dir, parentCopy: target, required: false });
    }
    // A package requires its peers from its own location at run time, so
    // they ship too. The copy is separate from any bundled copy of the same
    // package (see the build docs on shared state).
    const peerMeta = pkg.peerDependenciesMeta ?? {};
    for (const dep of Object.keys(pkg.peerDependencies ?? {})) {
      queue.push({
        name: dep,
        from: dir,
        parentCopy: target,
        required: peerMeta[dep]?.optional !== true,
      });
    }
  }
  return [...names].sort();
}

function copyAppAssets(fs: any, path: any, srcDir: string, destDir: string): void {
  for (const e of fs.readdirSync(srcDir, { withFileTypes: true })) {
    const src = path.join(srcDir, e.name);
    if (e.isDirectory()) {
      if (e.name.startsWith(".") || e.name === "node_modules") continue;
      copyAppAssets(fs, path, src, path.join(destDir, e.name));
      continue;
    }
    const ext = path.extname(e.name).toLowerCase();
    if (!APP_ASSET_EXTS.has(ext)) continue;
    fs.mkdirSync(destDir, { recursive: true });
    fs.copyFileSync(src, path.join(destDir, e.name));
  }
}
