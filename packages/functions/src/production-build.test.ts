// `pylon build` artifact (production-build.ts).
//
// The end-to-end test builds a fixture app, moves the artifact to a
// directory with no `node_modules` anywhere above it, and starts the bundled
// runner: it must register the app's functions from the bundle alone. The
// CLI-level smoke test (tools/smoke-production-build.sh) covers serving
// pages through `pylon start <dir>`.

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import {
  BUILD_INFO_FILE,
  buildProduction,
  checkInclude,
  collectAppModules,
  copyPackageClosure,
} from "./production-build";

const PKG_DIR = path.resolve(import.meta.dir, "..");
const dirs: string[] = [];
afterEach(() => {
  for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true });
});

/** A directory inside this package, so fixture imports of `react` resolve
 *  from the workspace `node_modules`. With `withDeps`, its `node_modules`
 *  links the packages of examples/ssr-hello, which has `@pylonsync/react`
 *  (the client runtime imports it), as in ssr-client-bundler.test.ts. */
function fixtureDir(files: Record<string, string>, withDeps = false): string {
  const dir = fs.mkdtempSync(path.join(PKG_DIR, ".prod-build-test-"));
  dirs.push(dir);
  if (withDeps) {
    const deps = path.resolve(PKG_DIR, "..", "..", "examples", "ssr-hello", "node_modules");
    if (!fs.existsSync(deps)) {
      throw new Error(`fixture deps missing at ${deps}; run \`bun install\` in examples/ssr-hello`);
    }
    // A real node_modules that links each shared package, so a test can
    // add its own packages next to them.
    fs.mkdirSync(path.join(dir, "node_modules"));
    for (const name of fs.readdirSync(deps)) {
      fs.symlinkSync(path.join(deps, name), path.join(dir, "node_modules", name));
    }
  }
  for (const [rel, body] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), body);
  }
  return dir;
}

const PAGE = (name: string) => `import React from "react";
export default function ${name}() { return <p>${name}</p>; }
`;
const LAYOUT = `import React from "react";
export default function Layout({ children }: { children: React.ReactNode }) {
  return <html><body>{children}</body></html>;
}
`;

function manifest(routes: Array<{ path: string; component: string }>): string {
  return JSON.stringify({
    manifest_version: 1,
    name: "fixture",
    version: "0.0.1",
    entities: [],
    routes: routes.map((r) => ({ ...r, mode: "ssr", layouts: ["app/layout"] })),
  });
}

/** Start `runner.js` and return its first protocol frame (the `ready`). */
async function readyFrame(runner: string, cwd: string): Promise<any> {
  const proc = Bun.spawn(["bun", runner], {
    cwd,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const reader = proc.stdout.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  const deadline = Date.now() + 15_000;
  try {
    while (!buf.includes("\n")) {
      if (Date.now() > deadline) throw new Error("runner sent no frame");
      const { value, done } = await reader.read();
      if (done) {
        const err = await new Response(proc.stderr).text();
        throw new Error(`runner exited before ready:\n${err}`);
      }
      buf += decoder.decode(value, { stream: true });
    }
    return JSON.parse(buf.slice(0, buf.indexOf("\n")));
  } finally {
    proc.kill();
  }
}

describe("buildProduction", () => {
  test("the artifact runs its functions with no source tree or node_modules", async () => {
    const cwd = fixtureDir({
      "app/layout.tsx": LAYOUT,
      "app/page.tsx": PAGE("Home"),
      "app/(marketing)/about/page.tsx": PAGE("About"),
      "app/not-found.tsx": PAGE("Missing"),
      "app/components/card.tsx": PAGE("Card"),
      "app/icon.png": "png-bytes",
      // A function with no imports: default-exported definition shape.
      "functions/hello.ts": `export default { type: "query", handler: async () => "hi" };`,
      "functions/_helper.ts": `export const x = 1;`,
      // Throws while loading. In source mode the runtime logs and skips it;
      // the artifact must too, not fail every other function with it.
      "functions/broken.ts": `throw new Error("boom at load");`,
      // Imports an external package that prints on load. The stdout fence
      // must be up first, or the line lands in the protocol stream.
      "functions/noisy.ts": `import { tag } from "noisy-pkg";
export default { type: "query", handler: async () => tag };`,
      "public/robots.txt": "User-agent: *\n",
      "content/doc.md": "# doc\n",
      "pylon.manifest.json": manifest([
        { path: "/", component: "app/page" },
        { path: "/about", component: "app/(marketing)/about/page" },
      ]),
    }, true);
    // Add the build block the SDK would write.
    const m = JSON.parse(fs.readFileSync(path.join(cwd, "pylon.manifest.json"), "utf8"));
    m.build = { include: ["content"], server: { external: ["noisy-pkg"] } };
    fs.writeFileSync(path.join(cwd, "pylon.manifest.json"), JSON.stringify(m));

    const noisy = path.join(cwd, "node_modules", "noisy-pkg");
    fs.mkdirSync(noisy);
    fs.writeFileSync(
      path.join(noisy, "package.json"),
      JSON.stringify({ name: "noisy-pkg", version: "1.0.0", main: "index.js" }),
    );
    fs.writeFileSync(
      path.join(noisy, "index.js"),
      `console.log("noisy-pkg loaded"); exports.tag = "noisy";`,
    );

    const outDir = path.join(cwd, "dist");
    const res = await buildProduction({ cwd, outDir });
    expect(res.clientRoutes).toBe(3);
    expect(res.functions).toBe(4);
    expect(res.externals).toEqual(["noisy-pkg"]);

    const info = JSON.parse(fs.readFileSync(path.join(outDir, BUILD_INFO_FILE), "utf8"));
    expect(info.server).toBe("server/runner.js");
    expect(info.client).toBe("client");
    const clientManifest = JSON.parse(
      fs.readFileSync(path.join(outDir, "client", "manifest.json"), "utf8"),
    );
    expect(clientManifest.outdir).toBe("client");
    expect(Object.keys(clientManifest.routes).sort()).toEqual([
      "app/(marketing)/about/page",
      "app/not-found",
      "app/page",
    ]);
    expect(fs.existsSync(path.join(outDir, "client", ".prebuilt"))).toBe(true);
    expect(fs.readFileSync(path.join(outDir, "public", "robots.txt"), "utf8")).toContain("User-agent");
    expect(fs.existsSync(path.join(outDir, "app", "icon.png"))).toBe(true);
    expect(fs.existsSync(path.join(outDir, "app", "page.tsx"))).toBe(false);
    expect(fs.existsSync(path.join(outDir, "content", "doc.md"))).toBe(true);
    // The build's staging files are gone.
    expect(fs.existsSync(path.join(cwd, ".pylon", "server-entry"))).toBe(false);

    // Move the artifact where nothing above it has node_modules.
    const runDir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-artifact-run-"));
    dirs.push(runDir);
    fs.cpSync(outDir, path.join(runDir, "dist"), { recursive: true });
    const artifact = path.join(runDir, "dist");
    // readyFrame parses the FIRST stdout line: a stray package log fails it.
    const ready = await readyFrame(path.join(artifact, "server", "runner.js"), artifact);
    expect(ready.type).toBe("ready");
    // `__`-prefixed names are the runtime's built-in functions.
    const appFns = ready.functions
      .map((f: any) => f.name)
      .filter((n: string) => !n.startsWith("__"))
      .sort();
    expect(appFns).toEqual(["hello", "noisy"]);
  }, 60_000);

  test("refuses to delete a directory that is not an earlier artifact", async () => {
    const cwd = fixtureDir({
      "functions/hello.ts": `export default { type: "query", handler: async () => "hi" };`,
      "pylon.manifest.json": manifest([]),
      "out/notes.txt": "keep me",
    });
    await expect(buildProduction({ cwd, outDir: path.join(cwd, "out") })).rejects.toThrow(
      "not empty",
    );
    expect(fs.readFileSync(path.join(cwd, "out", "notes.txt"), "utf8")).toBe("keep me");
    await expect(buildProduction({ cwd, outDir: cwd })).rejects.toThrow("project directory");
  });
});

test("collectAppModules keeps route modules and skips other components", () => {
  const cwd = fixtureDir({
    "app/page.tsx": "",
    "app/page.js": "",
    "app/(g)/x/layout.ts": "",
    "app/blog/[slug]/opengraph-image.tsx": "",
    "app/sitemap.ts": "",
    "app/components/button.tsx": "",
    "app/page.test.tsx": "",
  });
  const keys = collectAppModules(fs, path, cwd, "app", { routes: [] }).map((m) => m.key);
  expect(keys).toEqual([
    "app/(g)/x/layout",
    "app/blog/[slug]/opengraph-image",
    "app/page",
    "app/sitemap",
  ]);
  const page = collectAppModules(fs, path, cwd, "app", { routes: [] }).find(
    (m) => m.key === "app/page",
  )!;
  expect(page.file.endsWith("page.tsx")).toBe(true);
});

test("copyPackageClosure copies peers and nests a second version under its dependent", () => {
  const pkg = (name: string, version: string, deps: Record<string, string> = {}) =>
    JSON.stringify({ name, version, dependencies: deps });
  const cwd = fixtureDir({
    "node_modules/a/package.json": JSON.stringify({
      name: "a",
      version: "1.0.0",
      dependencies: { b: "^1", c: "^2" },
      peerDependencies: { p: "*", q: "*" },
      peerDependenciesMeta: { q: { optional: true } },
    }),
    "node_modules/p/package.json": pkg("p", "1.0.0"),
    "node_modules/a/index.js": "",
    "node_modules/b/package.json": pkg("b", "1.0.0", { c: "^1" }),
    "node_modules/b/node_modules/c/package.json": pkg("c", "1.0.0"),
    "node_modules/c/package.json": pkg("c", "2.0.0"),
  });
  const dest = path.join(cwd, "out", "node_modules");
  const names = copyPackageClosure(fs, path, cwd, ["a"], dest);
  // The required peer ships; the optional peer `q` is not installed.
  expect(names).toEqual(["a", "b", "c", "p"]);
  const ver = (p: string) => JSON.parse(fs.readFileSync(path.join(dest, p, "package.json"), "utf8")).version;
  expect(ver("c")).toBe("2.0.0");
  expect(ver("b/node_modules/c")).toBe("1.0.0");
  expect(() => copyPackageClosure(fs, path, cwd, ["missing"], path.join(cwd, "o2"))).toThrow(
    "not installed",
  );
});

test("checkInclude allows project files and refuses the rest", () => {
  const cwd = path.join(os.tmpdir(), "proj");
  const out = path.join(cwd, "dist");
  const check = (inc: string) => checkInclude(path, cwd, out, inc, path.resolve(cwd, inc));
  expect(check("content")).toBe("content");
  expect(check("..data")).toBe("..data");
  expect(check("data/seed.json")).toBe(path.join("data", "seed.json"));
  expect(() => check(".")).toThrow("is the project directory");
  expect(() => check("../elsewhere")).toThrow("outside the project");
  expect(() => check("/etc")).toThrow("outside the project");
  expect(() => check("dist")).toThrow("overlaps the output");
  expect(() => check("dist/x")).toThrow("overlaps the output");
  expect(() => check("server")).toThrow('own "server"');
  expect(() => check("pylon.manifest.json")).toThrow("own");
  // An output dir inside an included dir.
  expect(() =>
    checkInclude(path, cwd, path.join(cwd, "build", "out"), "build", path.join(cwd, "build")),
  ).toThrow("overlaps the output");
});
