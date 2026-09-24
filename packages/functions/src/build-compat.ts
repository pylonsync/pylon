// Browser compatibility for the client bundle: syntax lowering, core-js
// polyfills, and CSS targets. Driven by the manifest's `build` block
// (`buildManifest({ build: { target, polyfill, css } })`).
//
// Bun.build cannot lower syntax to an older target, so lowering runs as a
// pass over Bun's output:
//
//   1. SWC transforms every emitted .js file with `env.targets` (syntax
//      lowering + minify). In `usage` mode SWC also adds an
//      `import "core-js/modules/<feature>.js"` line for each feature the file
//      uses that the targets lack. The pass strips those lines and collects
//      them.
//   2. The collected set (or, in `entry` mode, every feature the targets
//      lack) is bundled once into `polyfills-<hash>.js`. The SSR head loads it
//      before the route entry.
//   3. Every .js output gets a new name whose hash covers the transform
//      settings, so a target change never serves a stale immutable file.
//
// The tools (@swc/core, core-js, browserslist, lightningcss) are resolved
// from the app, like @tailwindcss/cli. An app that sets no `build.target`
// needs none of them.

declare const Bun: {
  resolveSync(specifier: string, from: string): string;
  build(options: any): Promise<any>;
};

/** The manifest's `build` block, as the SDK writes it. */
export interface ManifestBuildConfig {
  target?: string | string[];
  polyfill?: false | "usage" | "entry";
  css?: { target?: string | string[] };
  sourcemap?: boolean;
  server?: { bundle?: boolean; external?: string[] };
  include?: string[];
}

/** Resolved client compatibility settings. `null` means Bun's output ships
 *  unchanged (no target set). */
export interface ClientCompat {
  /** Browserslist queries for JS. Empty when only `css.target` is set. */
  jsTargets: string[];
  /** Browserslist queries for CSS. */
  cssTargets: string[];
  polyfill: false | "usage" | "entry";
  sourcemap: boolean;
}

/**
 * Browser versions for each ECMAScript edition target. `es2019` becomes the
 * oldest browsers with full ES2019 syntax support, so SWC and Lightning CSS
 * (which work from browser versions) can use it.
 */
export const ES_TARGET_BROWSERS: Record<string, string[]> = {
  es2018: ["chrome 64", "edge 79", "firefox 67", "safari 12", "ios 12"],
  es2019: ["chrome 73", "edge 79", "firefox 67", "safari 12.1", "ios 12.2"],
  es2020: ["chrome 80", "edge 80", "firefox 80", "safari 14.1", "ios 14.5"],
  es2021: ["chrome 85", "edge 85", "firefox 80", "safari 14.1", "ios 14.5"],
  es2022: ["chrome 94", "edge 94", "firefox 93", "safari 16.4", "ios 16.4"],
};

/**
 * The oldest version of each browser that can run the Pylon client runtime.
 * The runtime ships as ES modules and loads route entries with dynamic
 * `import()`. Syntax lowering cannot make an older browser load a page.
 */
export const RUNTIME_BROWSER_FLOOR: Record<string, number> = {
  chrome: 63,
  and_chr: 63,
  edge: 79,
  firefox: 67,
  and_ff: 67,
  safari: 11.1,
  ios_saf: 11.3,
  opera: 50,
  op_mob: 46,
  samsung: 8,
  android: 63,
};

/** Normalize a `target` value to browserslist queries. Throws on a value the
 *  build cannot honor. */
export function targetQueries(target: string | string[], field: string): string[] {
  const list = Array.isArray(target) ? target : [target];
  if (list.length === 0) throw new Error(`${field}: must not be empty`);
  const out: string[] = [];
  for (const raw of list) {
    if (typeof raw !== "string" || raw.trim() === "") {
      throw new Error(`${field}: every entry must be a non-empty string`);
    }
    const t = raw.trim();
    const es = t.match(/^es(\d{4})$/i);
    if (es) {
      const key = `es${es[1]}`;
      const browsers = ES_TARGET_BROWSERS[key];
      if (!browsers) {
        throw new Error(
          `${field}: "${t}" is not supported. Use one of ${Object.keys(ES_TARGET_BROWSERS).join(", ")}, or browserslist queries. ` +
            `Targets below es2018 cannot run the Pylon client runtime (ES modules and dynamic import()).`,
        );
      }
      out.push(...browsers);
    } else {
      out.push(t);
    }
  }
  return out;
}

/**
 * Resolve the client compatibility settings from the manifest's `build`
 * block. Returns null when no JS or CSS target is set. `polyfill` without
 * `target` is an error: it needs the browsers to polyfill for.
 */
export function resolveClientCompat(
  build: ManifestBuildConfig | undefined,
): ClientCompat | null {
  if (!build) return null;
  const polyfill = build.polyfill ?? false;
  if (polyfill !== false && polyfill !== "usage" && polyfill !== "entry") {
    throw new Error(`build.polyfill: must be false, "usage", or "entry"`);
  }
  const sourcemap = build.sourcemap === true;
  if (build.target === undefined) {
    if (polyfill !== false) {
      throw new Error(`build.polyfill needs build.target (the browsers to polyfill for)`);
    }
    if (build.css?.target === undefined) return null;
    const cssTargets = targetQueries(build.css.target, "build.css.target");
    return { jsTargets: [], cssTargets, polyfill: false, sourcemap };
  }
  const jsTargets = targetQueries(build.target, "build.target");
  const cssTargets =
    build.css?.target !== undefined
      ? targetQueries(build.css.target, "build.css.target")
      : jsTargets;
  return { jsTargets, cssTargets, polyfill, sourcemap };
}

/** The npm package name of a module specifier (`@swc/core/x` → `@swc/core`). */
function packageName(spec: string): string {
  const parts = spec.split("/");
  return spec.startsWith("@") ? parts.slice(0, 2).join("/") : parts[0];
}

/** Load a build tool from the app's dependencies, with an install hint. */
async function loadAppTool(cwd: string, spec: string, why: string): Promise<any> {
  let resolved: string;
  try {
    resolved = Bun.resolveSync(spec, cwd);
  } catch {
    throw new Error(
      `${why} needs "${packageName(spec)}" in the app's dependencies. Run: bun add -d ${packageName(spec)}`,
    );
  }
  const mod = await import(resolved);
  // CommonJS packages (browserslist) arrive as `{ default: fn }`.
  return typeof mod.default === "function" ? mod.default : mod;
}

/** The browsers (from a browserslist result) that cannot run the client
 *  runtime. Browsers with no known floor are included. */
export function browsersBelowFloor(browsers: string[]): string[] {
  const below: string[] = [];
  for (const b of browsers) {
    const [name, version] = b.split(" ");
    const floor = RUNTIME_BROWSER_FLOOR[name];
    // A range such as "15.2-15.3": compare its lower bound.
    const v = parseFloat((version ?? "").split("-")[0]);
    if (floor === undefined || Number.isNaN(v) || v < floor) below.push(b);
  }
  return below;
}

const CORE_JS_IMPORT_RE = /import\s*["'](core-js\/modules\/[^"']+)["'];?/g;

/** Remove core-js feature imports from `code` and add them to `into`. */
export function extractCoreJsImports(code: string, into: Set<string>): string {
  return code.replace(CORE_JS_IMPORT_RE, (_m, spec: string) => {
    into.add(spec);
    return "";
  });
}

/** Give a hashed file name (`name-<hash>.js`) a new hash derived from its old
 *  one and `salt`. Names without a hash are returned unchanged. */
export function rehashName(
  base: string,
  salt: string,
  hasher: (s: string) => string,
): string {
  const m = base.match(/^(.*)-([A-Za-z0-9]+)\.js$/);
  if (!m) return base;
  return `${m[1]}-${hasher(`${m[2]}:${salt}`).slice(0, 10)}.js`;
}

export interface CompatResult {
  /** Old outdir-relative path → new outdir-relative path, for every renamed
   *  output. */
  renamed: Map<string, string>;
  /** Polyfill bundle, outdir-relative. Absent when polyfill is off or the
   *  targets need none. */
  polyfills?: string;
  /** Browsers the targets include that cannot run the client runtime. */
  unsupported: string[];
}

/**
 * Lower every .js output under `outdir` to the JS targets, collect and bundle
 * polyfills, and rename the outputs. `jsFiles` are outdir-relative,
 * "/"-separated paths of Bun's .js outputs.
 */
export async function applyClientCompat(opts: {
  fs: any;
  path: any;
  cwd: string;
  outdir: string;
  jsFiles: string[];
  compat: ClientCompat;
}): Promise<CompatResult> {
  const { fs, path, cwd, outdir, jsFiles, compat } = opts;
  const renamed = new Map<string, string>();
  if (compat.jsTargets.length === 0) return { renamed, unsupported: [] };

  const why = "build.target";
  const swc = await loadAppTool(cwd, "@swc/core", why);
  const browserslist = await loadAppTool(cwd, "browserslist", why);
  // The resolved browsers, not the queries: `defaults` or `last 2 versions`
  // resolve to new browsers when browserslist's data updates, and the output
  // (and so the file names) must change with them.
  const browsers: string[] = browserslist(compat.jsTargets);
  const unsupported = browsersBelowFloor(browsers);

  let coreJsVersion = "";
  if (compat.polyfill !== false) {
    let pkgPath: string;
    try {
      pkgPath = Bun.resolveSync("core-js/package.json", cwd);
    } catch {
      throw new Error(
        `build.polyfill needs "core-js" in the app's dependencies. Run: bun add core-js`,
      );
    }
    const v = JSON.parse(fs.readFileSync(pkgPath, "utf8")).version as string;
    coreJsVersion = v.split(".").slice(0, 2).join(".");
  }

  const envFor = (mode: "usage" | "entry" | undefined) => ({
    targets: compat.jsTargets,
    ...(mode ? { mode, coreJs: coreJsVersion } : {}),
  });

  const features = new Set<string>();
  for (const rel of jsFiles) {
    const abs = path.join(outdir, rel);
    const code = fs.readFileSync(abs, "utf8");
    const mapPath = `${abs}.map`;
    const inputMap =
      compat.sourcemap && fs.existsSync(mapPath)
        ? fs.readFileSync(mapPath, "utf8")
        : undefined;
    const out = await swc.transform(code, {
      filename: path.basename(abs),
      isModule: true,
      sourceMaps: compat.sourcemap,
      ...(inputMap ? { inputSourceMap: inputMap } : {}),
      jsc: {
        parser: { syntax: "ecmascript" },
        minify: { compress: true, mangle: true },
      },
      env: envFor(compat.polyfill === "usage" ? "usage" : undefined),
      module: { type: "es6" },
      minify: true,
    });
    const lowered = extractCoreJsImports(out.code, features);
    fs.writeFileSync(abs, stripSourceMapComment(lowered), "utf8");
    if (out.map) fs.writeFileSync(mapPath, out.map, "utf8");
  }

  if (compat.polyfill === "entry") {
    const out = await swc.transform(`import "core-js/stable";`, {
      filename: "polyfills-entry.js",
      isModule: true,
      jsc: { parser: { syntax: "ecmascript" } },
      env: envFor("entry"),
      module: { type: "es6" },
    });
    extractCoreJsImports(out.code, features);
  }

  const cryptoMod: any = await import("node:crypto");
  const hasher = (s: string) =>
    cryptoMod.createHash("sha256").update(s).digest("hex");
  const salt = JSON.stringify([
    browsers,
    compat.polyfill,
    compat.sourcemap,
    swc.version ?? "",
    coreJsVersion,
  ]);

  // Rename every .js output, then rewrite the references between them. Hashed
  // names are unique tokens, so a plain string replace is exact.
  const baseRenames = new Map<string, string>();
  for (const rel of jsFiles) {
    const base = path.basename(rel);
    const next = rehashName(base, salt, hasher);
    if (next !== base) baseRenames.set(base, next);
  }
  for (const rel of jsFiles) {
    const abs = path.join(outdir, rel);
    let code = fs.readFileSync(abs, "utf8");
    for (const [from, to] of baseRenames) {
      if (code.includes(from)) code = code.split(from).join(to);
    }
    const base = path.basename(rel);
    const nextBase = baseRenames.get(base) ?? base;
    const nextRel = rel.slice(0, rel.length - base.length) + nextBase;
    const nextAbs = path.join(outdir, nextRel);
    if (fs.existsSync(`${abs}.map`)) {
      code += `//# sourceMappingURL=${nextBase}.map\n`;
      if (nextAbs !== abs) fs.renameSync(`${abs}.map`, `${nextAbs}.map`);
    }
    fs.writeFileSync(nextAbs, code, "utf8");
    if (nextAbs !== abs) {
      fs.rmSync(abs);
      renamed.set(rel, nextRel);
    }
  }

  let polyfills: string | undefined;
  if (features.size > 0) {
    polyfills = await buildPolyfills({
      fs,
      path,
      cwd,
      outdir,
      features: [...features].sort(),
      swc,
      env: envFor(undefined),
      hasher,
      salt,
    });
  }

  return { renamed, polyfills, unsupported };
}

function stripSourceMapComment(code: string): string {
  return code.replace(/\n?\/\/# sourceMappingURL=\S+\s*$/, "") + "\n";
}

/** Bundle the core-js feature modules into one lowered file in `outdir`.
 *  Returns its outdir-relative path. */
async function buildPolyfills(opts: {
  fs: any;
  path: any;
  cwd: string;
  outdir: string;
  features: string[];
  swc: any;
  env: Record<string, unknown>;
  hasher: (s: string) => string;
  salt: string;
}): Promise<string> {
  const { fs, path, cwd, outdir, features, swc, env, hasher, salt } = opts;
  const stageDir = path.join(cwd, ".pylon");
  fs.mkdirSync(stageDir, { recursive: true });
  const entry = path.join(stageDir, "polyfills-entry.js");
  fs.writeFileSync(
    entry,
    features.map((f) => `import ${JSON.stringify(f)};`).join("\n") + "\n",
    "utf8",
  );
  const tmpOut = path.join(stageDir, "polyfills-build");
  fs.rmSync(tmpOut, { recursive: true, force: true });
  const result = await Bun.build({
    entrypoints: [entry],
    outdir: tmpOut,
    root: cwd,
    target: "browser",
    format: "esm",
    minify: true,
  });
  if (!result.success) {
    const msgs = (result.logs ?? []).map((l: any) => l.message).join("\n");
    throw new Error(`polyfill bundle failed:\n${msgs}`);
  }
  const built = result.outputs.find((o: any) => o.path.endsWith(".js"));
  const code = fs.readFileSync(built.path, "utf8");
  const out = await swc.transform(code, {
    filename: "polyfills.js",
    isModule: true,
    jsc: {
      parser: { syntax: "ecmascript" },
      minify: { compress: true, mangle: true },
    },
    env,
    module: { type: "es6" },
    minify: true,
  });
  const name = `polyfills-${hasher(`${out.code}:${salt}`).slice(0, 10)}.js`;
  fs.writeFileSync(path.join(outdir, name), out.code, "utf8");
  fs.rmSync(tmpOut, { recursive: true, force: true });
  fs.rmSync(entry, { force: true });
  return name;
}

/**
 * Apply Lightning CSS to compiled CSS for the CSS targets: vendor prefixes,
 * nesting and color-syntax lowering, minification.
 */
export async function transformCss(
  cwd: string,
  css: string,
  filename: string,
  cssTargets: string[],
): Promise<string> {
  const why = "build.css.target (or build.target)";
  const lightningcss = await loadAppTool(cwd, "lightningcss", why);
  const browserslist = await loadAppTool(cwd, "browserslist", why);
  const targets = lightningcss.browserslistToTargets(browserslist(cssTargets));
  const out = lightningcss.transform({
    filename,
    code: Buffer.from(css),
    minify: true,
    targets,
  });
  return out.code.toString();
}
