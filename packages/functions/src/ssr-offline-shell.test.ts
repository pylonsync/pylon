// Offline apps (`buildManifest({ offline })`), render side:
//
//   - A render the service worker asked for (`x-pylon-offline-shell: 1`) is
//     marked keepable only when its HTML is the same for every signed-in
//     user, and then its hydration data carries the signed-in bit and no
//     identity. A page that reads props.auth, serverData, headers or cookies
//     is refused.
//   - Every page of an offline app gets the install tags and tells the
//     client to register the worker.
//   - The files the worker precaches for a route are exactly the files the
//     route's HTML loads (Plan 012 step 2).
//
// ssr-runtime.ts pulls in runtime.ts (the bun runner entrypoint, which runs
// main() on import), so each render runs in a child process, like
// ssr-page-host.test.ts.

import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { offlineWorkerConfig } from "./ssr-offline";
import { routeAssetUrls } from "./ssr-route-assets";

const SSR_RUNTIME = join(import.meta.dir, "ssr-runtime.ts");
const PKG_NODE_MODULES = join(import.meta.dir, "..", "node_modules");

interface Frame {
  type: string;
  headers?: Record<string, string>;
  data?: string;
}

const BUNDLE_MANIFEST = {
  build_id: "b1",
  outdir: ".pylon/client-build",
  public_prefix: "/_pylon/build/",
  polyfills: "polyfills-aaaa1111.js",
  routes: {
    "app/page": {
      file: "client-entry-app__page-bbbb2222.js",
      imports: ["chunks/chunk-cccc3333.js", "chunks/chunk-dddd4444.js"],
      css: ["styles-eeee5555.css"],
      path: "/",
    },
  },
  fonts: {
    faces: [],
    variables: [],
    preload: ["fonts/inter-ffff6666.woff2"],
  },
  offline: {
    name: "Field Sales",
    shortName: "Sales",
    themeColor: "#0b2545",
    icons: [
      { src: "/icons/192.png", sizes: "192x192", type: "image/png" },
      { src: "/icons/512.png", sizes: "512x512", type: "image/png" },
    ],
    startUrl: "/",
    routes: ["/"],
    version: "v1",
  },
};

function makeApp(pageBody: string, manifest: object = BUNDLE_MANIFEST): string {
  const dir = mkdtempSync(join(tmpdir(), "pylon-offline-shell-"));
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
    `export default function Page(props: any) {
${pageBody}
}
`,
  );
  writeFileSync(join(dir, ".pylon", "client-build", "styles-eeee5555.css"), "p{color:red}");
  writeFileSync(
    join(dir, ".pylon", "client-build", "manifest.json"),
    JSON.stringify(manifest),
  );
  return dir;
}

const SIGNED_IN = {
  user_id: "u_ana",
  is_admin: false,
  tenant_id: "t_veritas",
  roles: ["rep"],
};

async function render(
  dir: string,
  opts: { shell?: boolean; auth?: object; env?: Record<string, string> } = {},
): Promise<{ start: Frame | undefined; html: string; data: any }> {
  const headers: Record<string, string> = { host: "app.example" };
  if (opts.shell) headers["x-pylon-offline-shell"] = "1";
  const auth = opts.auth ?? SIGNED_IN;
  const msg = {
    type: "render_route",
    call_id: "c1",
    component: "app/page",
    layouts: ["app/layout"],
    route_path: "/",
    url: "/",
    host: "app.example",
    params: {},
    search_params: {},
    headers,
    cookies: { sid: "secret-session" },
    auth,
    session_present: !!(auth as any).user_id,
  };
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
    env: { ...process.env, PYLON_DEV_MODE: "0", ...(opts.env ?? {}) },
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
  const frames = JSON.parse(line.slice(line.indexOf("FRAMES ") + 7)) as Frame[];
  const html = frames
    .filter((f) => f.type === "render_chunk")
    .map((f) => Buffer.from(f.data!, "base64").toString("utf8"))
    .join("");
  const match = html.match(/<script id="__PYLON_DATA__"[^>]*>([\s\S]*?)<\/script>/);
  return {
    start: frames.find((f) => f.type === "response_start"),
    html,
    data: match ? JSON.parse(match[1]) : null,
  };
}

const SESSION_ONLY_PAGE = `return <p>{props.session.exists ? "signed in" : "signed out"}</p>;`;

test("a shell request for a page that reads only the signed-in bit is kept, without identity", async () => {
  const dir = makeApp(SESSION_ONLY_PAGE);
  const { start, html, data } = await render(dir, { shell: true });
  expect(start?.headers?.["x-pylon-offline-shell"]).toBe("1");
  expect(html).toContain("signed in");
  expect(data.props.auth).toEqual({ signedIn: true });
  expect(data.props.session).toEqual({ exists: true });
  expect(html).not.toContain("u_ana");
  expect(html).not.toContain("t_veritas");
  expect(html).not.toContain("secret-session");
});

test("a page that reads props.auth is refused, with the reason", async () => {
  const dir = makeApp(`return <p>{props.auth.userId ?? "anon"}</p>;`);
  const { start } = await render(dir, { shell: true });
  expect(start?.headers?.["x-pylon-offline-shell"]).toBe("refused; the page read props.auth");
});

test("a page that reads serverData is refused", async () => {
  const dir = makeApp(`props.serverData.list("Lead"); return <p>x</p>;`);
  const { start } = await render(dir, { shell: true });
  expect(start?.headers?.["x-pylon-offline-shell"]).toBe("refused; the page read serverData");
});

test("a page that reads cookies is refused", async () => {
  const dir = makeApp(`return <p>{String(props.cookies.theme)}</p>;`);
  const { start } = await render(dir, { shell: true });
  expect(start?.headers?.["x-pylon-offline-shell"]).toBe(
    "refused; the page read request headers or cookies",
  );
});

test("an ordinary request carries no shell verdict and keeps its real auth", async () => {
  const dir = makeApp(SESSION_ONLY_PAGE);
  const { start, data } = await render(dir);
  expect(start?.headers?.["x-pylon-offline-shell"]).toBeUndefined();
  expect(data.props.auth.user_id).toBe("u_ana");
});

test("every page of an offline app gets the install tags and the register flag", async () => {
  const dir = makeApp(SESSION_ONLY_PAGE);
  const { html, data } = await render(dir);
  expect(html).toContain('<link rel="manifest" href="/manifest.webmanifest">');
  expect(html).toContain('<meta name="theme-color" content="#0b2545">');
  expect(html).toContain('<meta name="apple-mobile-web-app-title" content="Sales">');
  expect(html).toContain('<link rel="apple-touch-icon" href="/icons/512.png">');
  expect(data.offline).toEqual({ dev: false });

  const dev = await render(dir, { env: { PYLON_DEV_MODE: "1" } });
  expect(dev.data.offline).toEqual({ dev: true });
});

test("an app without offline gets neither", async () => {
  const { offline: _o, ...plain } = BUNDLE_MANIFEST;
  const dir = makeApp(SESSION_ONLY_PAGE, plain);
  const { html, data } = await render(dir);
  expect(html).not.toContain('rel="manifest"');
  expect(data.offline).toBeUndefined();
});

test("the worker precaches exactly the files the route's HTML loads", async () => {
  const dir = makeApp(SESSION_ONLY_PAGE);
  const { html } = await render(dir);
  // Every build file the page references: stylesheets, modulepreloads,
  // font preloads, module scripts.
  const loaded = new Set<string>();
  for (const m of html.matchAll(/<(?:link|script)\b[^>]*?(?:href|src)="(\/_pylon\/build\/[^"]+)"/g)) {
    loaded.add(m[1]);
  }
  // The stylesheet is a link by default; record it whether linked or inlined.
  for (const m of html.matchAll(/data-pylon-css="([^"]+)"/g)) loaded.add(`/_pylon/build/${m[1]}`);
  const listed = routeAssetUrls(BUNDLE_MANIFEST, "app/page");
  expect(new Set(listed)).toEqual(loaded);
  expect(loaded.size).toBe(6);

  const { config } = await offlineWorkerConfig(BUNDLE_MANIFEST, {
    name: "Field Sales",
    startUrl: "/",
    display: "standalone",
    routes: ["/"],
    icons: BUNDLE_MANIFEST.offline.icons,
  });
  for (const url of loaded) expect(config.precache).toContain(url);
  expect(config.precache).toContain("/_pylon/build/manifest.json");
  expect(config.precache).toContain("/manifest.webmanifest");
  expect(config.precache).toContain("/icons/512.png");
  expect(config.shells).toEqual(["/"]);
});
