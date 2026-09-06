// Design render (`RenderRouteMessage.design: true`): the document the design
// canvas loads into a `srcdoc` iframe. Compared with the same page rendered
// normally in dev mode, it must have
//   - a `<base href>` first in the injected head blob,
//   - the stylesheet inlined even when it is over PYLON_SSR_INLINE_CSS_MAX,
//   - no modulepreload links, no `__PYLON_DATA__`, no module script,
//   - no live-reload snippet and no dev HUD.
//
// ssr-runtime.ts pulls in runtime.ts (the bun runner entrypoint, which runs
// main() on import), so — like ssr-raw-get.test.ts — the render runs in a child
// process with a kept-open stdin pipe and reports its frames over stderr.

import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { designBaseHref } from "./ssr-runtime";

const SSR_RUNTIME = join(import.meta.dir, "ssr-runtime.ts");
const PKG_NODE_MODULES = join(import.meta.dir, "..", "node_modules");

interface Frame {
  type: string;
  status?: number;
  headers?: Record<string, string>;
  data?: string;
}

function makeApp(): string {
  const dir = mkdtempSync(join(tmpdir(), "pylon-design-render-"));
  mkdirSync(join(dir, "app"), { recursive: true });
  mkdirSync(join(dir, ".pylon", "client-build"), { recursive: true });
  // react / react-dom resolve from this package's install.
  symlinkSync(PKG_NODE_MODULES, join(dir, "node_modules"), "dir");
  writeFileSync(
    join(dir, "app", "layout.tsx"),
    `export default function Layout({ children }: any) {
  return <html><head><title>t</title></head><body><main id="root">{children}</main></body></html>;
}
`,
  );
  writeFileSync(
    join(dir, "app", "page.tsx"),
    `export default function Page() {
  return <section className="hero"><h1>Hello</h1><img src="/logo.png" /></section>;
}
`,
  );
  // A stylesheet larger than the default 32KB inline cap.
  const css = ".hero{padding:1rem}\n" + "/* pad */\n".repeat(5000);
  writeFileSync(join(dir, ".pylon", "client-build", "app.css"), css);
  writeFileSync(join(dir, ".pylon", "client-build", "entry.js"), "");
  writeFileSync(
    join(dir, ".pylon", "client-build", "manifest.json"),
    JSON.stringify({
      outdir: ".pylon/client-build",
      public_prefix: "/_pylon/build/",
      routes: {
        "app/page": { file: "entry.js", imports: ["chunks/shared.js"], css: ["app.css"] },
      },
    }),
  );
  return dir;
}

async function render(dir: string, design: boolean): Promise<{ frames: Frame[]; html: string }> {
  const msg = {
    type: "render_route",
    call_id: "c1",
    component: "app/page",
    layouts: ["app/layout"],
    route_path: "/",
    url: "/",
    params: {},
    search_params: {},
    headers: { host: "127.0.0.1:4599" },
    cookies: {},
    auth: { user_id: null, is_admin: false, tenant_id: null, roles: [] },
    design,
  };
  const probe = `
import { handleRenderRoute } from ${JSON.stringify(SSR_RUNTIME)};
const frames = [];
await handleRenderRoute(${JSON.stringify(msg)}, (m) => frames.push(m));
console.error("FRAMES " + JSON.stringify(frames));
setTimeout(() => process.exit(0), 50);
`;
  const probePath = join(dir, design ? "probe-design.ts" : "probe-plain.ts");
  writeFileSync(probePath, probe);
  const proc = Bun.spawn(["bun", "run", probePath], {
    cwd: dir,
    env: { ...process.env, PYLON_DEV_MODE: "1" },
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const [out, err] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  const code = await proc.exited;
  // runtime.ts prefixes console.error lines with "[error] ".
  const line = err.split("\n").find((l) => l.includes("FRAMES "));
  if (code !== 0 || !line) {
    throw new Error(`probe failed (exit ${code})\nstdout: ${out}\nstderr: ${err}`);
  }
  const frames = JSON.parse(line.slice(line.indexOf("FRAMES ") + "FRAMES ".length)) as Frame[];
  const html = frames
    .filter((f) => f.type === "render_chunk")
    .map((f) => Buffer.from(f.data!, "base64").toString("utf8"))
    .join("");
  return { frames, html };
}

test("designBaseHref uses the request host, else 127.0.0.1:PYLON_PORT", () => {
  expect(designBaseHref({ host: "localhost:4321" })).toBe('<base href="http://localhost:4321/">');
  expect(designBaseHref({}, { PYLON_PORT: "5000" })).toBe('<base href="http://127.0.0.1:5000/">');
  expect(designBaseHref({}, {})).toBe('<base href="http://127.0.0.1:4321/">');
});

test("design render: base href, inlined CSS, no hydration, no dev chrome", async () => {
  const dir = makeApp();
  const { frames, html } = await render(dir, true);
  expect(frames[0].type).toBe("response_start");
  expect(frames[0].status).toBe(200);
  expect(frames[frames.length - 1].type).toBe("render_done");

  // Page DOM is intact.
  expect(html).toContain('<section class="hero"');
  expect(html).toContain('<img src="/logo.png"');
  // <base href> is the first injected head tag.
  const headEnd = html.indexOf("</head>");
  const base = html.indexOf('<base href="http://127.0.0.1:4599/">');
  expect(base).toBeGreaterThan(-1);
  expect(base).toBeLessThan(headEnd);
  const style = html.indexOf('<style data-pylon-css="app.css">');
  expect(style).toBeGreaterThan(base);
  expect(html).toContain(".hero{padding:1rem}");
  expect(html).not.toContain('<link rel="stylesheet"');
  expect(html).not.toContain("modulepreload");
  // No hydration, no dev chrome.
  expect(html).not.toContain("__PYLON_DATA__");
  expect(html).not.toContain('<script type="module"');
  expect(html).not.toContain("/_pylon/dev/live");
  expect(html).not.toContain("__pylon_hud");
  expect(html).not.toMatch(/<script/);
});

test("the same page rendered normally in dev mode keeps hydration and dev chrome", async () => {
  const dir = makeApp();
  const { html } = await render(dir, false);
  expect(html).not.toContain("<base href");
  // Over the 32KB cap → the link, not an inline sheet.
  expect(html).toContain('<link rel="stylesheet" href="/_pylon/build/app.css">');
  expect(html).toContain("modulepreload");
  expect(html).toContain("__PYLON_DATA__");
  expect(html).toContain('<script type="module"');
  expect(html).toContain("/_pylon/dev/live");
});
