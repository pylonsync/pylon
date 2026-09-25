// `props.host`: the request host when the server trusts it, else "". The Rust
// host computes it (crates/runtime/tests/page_host.rs covers which hosts
// pass); these cover what the Bun runtime does with it:
//   - the page reads it,
//   - the hydration payload carries it, so the browser renders the same thing,
//   - a bucketed page's shared tail carries it too (it is in the cache key),
//   - reading it does not stop a page from being cached,
//   - an older host that sends no `host` gives "".
//
// ssr-runtime.ts pulls in runtime.ts (the bun runner entrypoint, which runs
// main() on import), so — like ssr-design-render.test.ts — each render runs in
// a child process and reports its frames over stderr.

import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SSR_RUNTIME = join(import.meta.dir, "ssr-runtime.ts");
const PKG_NODE_MODULES = join(import.meta.dir, "..", "node_modules");

interface Frame {
  type: string;
  status?: number;
  headers?: Record<string, string>;
  data?: string;
}

function makeApp(pageExtra = ""): string {
  const dir = mkdtempSync(join(tmpdir(), "pylon-page-host-"));
  mkdirSync(join(dir, "app"), { recursive: true });
  mkdirSync(join(dir, ".pylon", "client-build"), { recursive: true });
  symlinkSync(PKG_NODE_MODULES, join(dir, "node_modules"), "dir");
  writeFileSync(
    join(dir, "app", "layout.tsx"),
    `export default function Layout({ children }: any) {
  return <html><head><title>t</title></head><body>{children}</body></html>;
}
`,
  );
  writeFileSync(
    join(dir, "app", "page.tsx"),
    `${pageExtra}
export default function Page(props: any) {
  return <p id="host">{"host=" + props.host}</p>;
}
`,
  );
  writeFileSync(join(dir, ".pylon", "client-build", "entry.js"), "");
  writeFileSync(
    join(dir, ".pylon", "client-build", "manifest.json"),
    JSON.stringify({
      outdir: ".pylon/client-build",
      public_prefix: "/_pylon/build/",
      routes: { "app/page": { file: "entry.js", imports: [], css: [] } },
    }),
  );
  return dir;
}

async function render(
  dir: string,
  host: string | undefined,
  env: Record<string, string> = { PYLON_DEV_MODE: "1" },
): Promise<{ frames: Frame[]; html: string; data: any }> {
  const msg: Record<string, unknown> = {
    type: "render_route",
    call_id: "c1",
    component: "app/page",
    layouts: ["app/layout"],
    route_path: "/",
    url: "/",
    params: {},
    search_params: {},
    headers: { host: host ?? "ignored.example" },
    cookies: {},
    auth: { user_id: null, is_admin: false, tenant_id: null, roles: [] },
  };
  if (host !== undefined) msg.host = host;
  const probe = `
import { handleRenderRoute } from ${JSON.stringify(SSR_RUNTIME)};
const frames = [];
await handleRenderRoute(${JSON.stringify(msg)}, (m) => frames.push(m));
console.error("FRAMES " + JSON.stringify(frames));
setTimeout(() => process.exit(0), 50);
`;
  const probePath = join(dir, `probe-${Math.random().toString(36).slice(2)}.ts`);
  writeFileSync(probePath, probe);
  const proc = Bun.spawn(["bun", "run", probePath], {
    cwd: dir,
    env: { ...process.env, ...env },
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const [out, err] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  const code = await proc.exited;
  const line = err.split("\n").find((l) => l.includes("FRAMES "));
  if (code !== 0 || !line) {
    throw new Error(`probe failed (exit ${code})\nstdout: ${out}\nstderr: ${err}`);
  }
  const frames = JSON.parse(line.slice(line.indexOf("FRAMES ") + "FRAMES ".length)) as Frame[];
  const html = frames
    .filter((f) => f.type === "render_chunk")
    .map((f) => Buffer.from(f.data!, "base64").toString("utf8"))
    .join("");
  const match = html.match(/<script id="__PYLON_DATA__"[^>]*>([\s\S]*?)<\/script>/);
  const data = match ? JSON.parse(match[1]) : null;
  return { frames, html, data };
}

test("the page reads props.host, and the browser gets the same value", async () => {
  const dir = makeApp();
  const { html, data } = await render(dir, "feedback.acme.test");
  expect(html).toContain("host=feedback.acme.test");
  expect(data?.props?.host).toBe("feedback.acme.test");
  // The raw request headers still never reach the browser.
  expect(data?.props?.headers).toEqual({});
});

test("an untrusted host arrives as the empty string", async () => {
  const dir = makeApp();
  const { html, data } = await render(dir, "");
  expect(html).toContain('host=</p>');
  expect(data?.props?.host).toBe("");
});

test("a host that predates the field sends none: the page sees \"\"", async () => {
  const dir = makeApp();
  const { html, data } = await render(dir, undefined);
  expect(html).toContain('host=</p>');
  expect(html).not.toContain("ignored.example");
  expect(data?.props?.host).toBe("");
});

test("reading props.host keeps a page cacheable (the host is in the cache key)", async () => {
  const dir = makeApp("export const revalidate = 60;");
  const { frames, html } = await render(dir, "feedback.acme.test", { PYLON_DEV_MODE: "0" });
  expect(html).toContain("host=feedback.acme.test");
  // The runner's internal proof that the render is shareable; the Rust host
  // turns it into a public Cache-Control and strips it.
  const start = frames.find((f) => f.type === "response_start");
  expect(start?.headers?.["x-pylon-cacheable"]).toBe("60");
});

test("a bucketed page's shared tail carries the host", async () => {
  const dir = makeApp('export const cache = "auth-bucketed";');
  const { data } = await render(dir, "feedback.acme.test", { PYLON_DEV_MODE: "0" });
  expect(data?.props?.host).toBe("feedback.acme.test");
});
