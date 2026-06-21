// Tests for raw GET route handlers: a `route.ts` exporting `GET` returns
//   { body, contentType?, status?, headers? }
// which handleForm streams verbatim (response_start / render_chunk / render_done)
// with a custom content-type and NO React render / hydration tail. This is the
// GET analogue of app/sitemap.ts → /sitemap.xml, at an arbitrary route path.
//
// ssr-form-runtime.ts imports runtime.ts, which runs main() on import (it IS the
// bun runner entrypoint), so — like runtime-db.test.ts — the handler is exercised
// in a child process with a kept-open stdin pipe. The probe calls handleForm
// directly and reports the emitted frames over stderr (main() fences stdout for
// real protocol frames).

import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const FORM_RUNTIME = join(import.meta.dir, "ssr-form-runtime.ts");

interface Frame {
  type: string;
  status?: number;
  headers?: Record<string, string>;
  data?: string;
}

// Spawn a child that writes `app/<routeDir>/route.ts`, calls handleForm with a
// GET `handle_form` message, and prints the resulting frames on stderr.
async function runGet(
  routeDir: string,
  routeSrc: string,
  msgOverrides: Record<string, unknown> = {},
): Promise<Frame[]> {
  const dir = mkdtempSync(join(tmpdir(), "pylon-raw-get-"));
  const routeFull = join(dir, "app", routeDir);
  mkdirSync(routeFull, { recursive: true });
  writeFileSync(join(routeFull, "route.ts"), routeSrc);

  const msg = {
    type: "handle_form",
    call_id: "c1",
    component: `app/${routeDir}/route`,
    route_path: "/r",
    method: "GET",
    url: "/r",
    params: {},
    search_params: {},
    form: {},
    headers: {},
    cookies: {},
    auth: { user_id: null, is_admin: false, tenant_id: null, roles: [] },
    ...msgOverrides,
  };

  const probe = `
import { handleForm } from ${JSON.stringify(FORM_RUNTIME)};
const frames = [];
await handleForm(${JSON.stringify(msg)}, (m) => frames.push(m));
console.error("FRAMES " + JSON.stringify(frames));
setTimeout(() => process.exit(0), 50);
`;
  const scriptPath = join(dir, "probe.ts");
  writeFileSync(scriptPath, probe);

  const proc = Bun.spawn(["bun", scriptPath], {
    cwd: dir, // importModule resolves the component relative to cwd
    stdin: "pipe", // keep main()'s readerLoop alive until the probe self-exits
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stderr] = await Promise.all([
    new Response(proc.stderr).text(),
    new Response(proc.stdout).text(),
    proc.exited,
  ]);
  const line = stderr.split("\n").find((l) => l.includes("FRAMES "));
  if (!line) throw new Error(`probe produced no frames. stderr:\n${stderr}`);
  return JSON.parse(line.slice(line.indexOf("FRAMES ") + 7)) as Frame[];
}

const decode = (f?: Frame) =>
  f?.data ? Buffer.from(f.data, "base64").toString("utf8") : "";

test("GET export → 200, custom content-type + raw body, no hydration tail", async () => {
  const frames = await runGet(
    "appcast.xml",
    `export const GET = async () => ({
      body: '<?xml version="1.0"?><rss><channel><title>Yapless</title></channel></rss>',
      contentType: "application/xml; charset=utf-8",
      headers: { "cache-control": "public, max-age=300" },
    });`,
  );
  const start = frames.find((f) => f.type === "response_start");
  const body = decode(frames.find((f) => f.type === "render_chunk"));
  expect(start?.status).toBe(200);
  expect(start?.headers?.["content-type"]).toContain("application/xml");
  expect(start?.headers?.["cache-control"]).toBe("public, max-age=300");
  expect(body).toContain("<rss>");
  expect(body.trim().endsWith("</rss>")).toBe(true);
  // Critical: no hydration <script>/__PYLON_DATA__ tail (would corrupt XML).
  expect(body.includes("__PYLON_DATA__")).toBe(false);
  expect(body.includes("<script")).toBe(false);
  expect(frames.some((f) => f.type === "render_done")).toBe(true);
});

test("async GET reads params + returns a custom status", async () => {
  const frames = await runGet(
    "feed/[slug]",
    `export const GET = async (req) => ({
      body: "feed for " + req.params.slug,
      contentType: "text/plain; charset=utf-8",
      status: 201,
    });`,
    { route_path: "/feed/:slug", url: "/feed/abc", params: { slug: "abc" } },
  );
  const start = frames.find((f) => f.type === "response_start");
  expect(start?.status).toBe(201);
  expect(decode(frames.find((f) => f.type === "render_chunk"))).toBe("feed for abc");
});

test("GET with no content-type defaults to text/plain", async () => {
  const frames = await runGet("ping", `export const GET = () => ({ body: "ok" });`, {
    route_path: "/ping",
    url: "/ping",
  });
  const start = frames.find((f) => f.type === "response_start");
  expect(start?.status).toBe(200);
  expect(start?.headers?.["content-type"]).toContain("text/plain");
});

test("GET may response.redirect() instead of returning a body", async () => {
  const frames = await runGet(
    "go",
    `export const GET = (req) => { req.response.redirect("/dest"); };`,
    { route_path: "/go", url: "/go" },
  );
  const start = frames.find((f) => f.type === "response_start");
  expect(start?.status).toBeGreaterThanOrEqual(300);
  expect(start?.status).toBeLessThan(400);
  expect(start?.headers?.["location"]).toBe("/dest");
});

test("GET on a route.ts with no GET export → 405 advertising its methods", async () => {
  const frames = await runGet(
    "form-only",
    `export const POST = (req) => { req.response.redirect("/ok"); };`,
    { route_path: "/form-only", url: "/form-only" },
  );
  const start = frames.find((f) => f.type === "response_start");
  expect(start?.status).toBe(405);
  expect(start?.headers?.["allow"]).toContain("POST");
});
