// A page with `cache = "auth-bucketed"` is stored as ONE shared entry for
// every signed-in visitor, so its render must not depend on who rendered it.
// Safe serverData reads apply the caller's read policies on the host, so their
// rows depend on the caller: a signed-in render that reads serverData is
// identity-specific and must not carry the `x-pylon-bucket` proof. Before
// this check, Alice's render of a page listing "my notes" was stored in the
// signed-in bucket and served to Bob.
//
// Each render runs in a child process (ssr-runtime.ts imports runtime.ts,
// whose main() owns stdin/stdout). The test plays the host: it answers the
// child's `db` requests over stdin with the rows the policy would give the
// render's auth.

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

function makeApp(page: string): string {
  const dir = mkdtempSync(join(tmpdir(), "pylon-bucket-sd-"));
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
  writeFileSync(join(dir, "app", "page.tsx"), page);
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

const NOTES_PAGE = `export const cache = "auth-bucketed";
export const revalidate = 300;
export default async function Page({ serverData }: any) {
  const notes = await serverData.list("Note");
  return <ul>{notes.map((n: any) => <li key={n.id}>{"NOTE:" + n.body}</li>)}</ul>;
}
`;

const UNSAFE_NOTES_PAGE = NOTES_PAGE.replace("serverData.list", "serverData.unsafe.list");

/** Rows a `Note` read returns: the caller's own notes (owner policy) for a
 *  safe read, every note for `serverData.unsafe`. */
function notesFor(userId: string | null, unsafe: boolean) {
  const all = [
    { id: "n1", userId: "alice", body: "alice-secret" },
    { id: "n2", userId: "bob", body: "bob-secret" },
  ];
  return unsafe ? all : all.filter((n) => n.userId === userId);
}

async function render(dir: string, userId: string | null) {
  const msg = {
    type: "render_route",
    call_id: "c1",
    component: "app/page",
    layouts: ["app/layout"],
    route_path: "/",
    url: "/",
    params: {},
    search_params: {},
    headers: { host: "app.test" },
    cookies: {},
    host: "app.test",
    session_present: userId != null,
    auth: { user_id: userId, is_admin: false, tenant_id: null, roles: [] },
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
    env: { ...process.env, PYLON_DEV_MODE: "0" },
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  // Answer the child's db requests the way the host would.
  const answerDb = (async () => {
    const decoder = new TextDecoder();
    let buf = "";
    for await (const chunk of proc.stdout) {
      buf += decoder.decode(chunk);
      let nl: number;
      while ((nl = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, nl);
        buf = buf.slice(nl + 1);
        let m: any;
        try {
          m = JSON.parse(line);
        } catch {
          continue;
        }
        if (m.type !== "db") continue;
        const data = m.entity === "Note" ? notesFor(userId, m.unsafe_op === true) : [];
        proc.stdin.write(
          JSON.stringify({ type: "result", call_id: m.call_id, op_id: m.op_id, data }) + "\n",
        );
        proc.stdin.flush();
      }
    }
  })();
  const err = await new Response(proc.stderr).text();
  const code = await proc.exited;
  await answerDb;
  const line = err.split("\n").find((l) => l.includes("FRAMES "));
  if (code !== 0 || !line) throw new Error(`probe failed (exit ${code})\nstderr: ${err}`);
  const frames = JSON.parse(line.slice(line.indexOf("FRAMES ") + "FRAMES ".length)) as Frame[];
  const html = frames
    .filter((f) => f.type === "render_chunk")
    .map((f) => Buffer.from(f.data!, "base64").toString("utf8"))
    .join("");
  const start = frames.find((f) => f.type === "response_start");
  return { html, headers: start?.headers ?? {} };
}

test("a signed-in render that reads serverData is not stored in the shared bucket", async () => {
  const dir = makeApp(NOTES_PAGE);
  const alice = await render(dir, "alice");
  expect(alice.html).toContain("NOTE:alice-secret");
  // The proof the host needs to store this render under the signed-in
  // bucket, where Bob's request would read it.
  expect(alice.headers["x-pylon-bucket"]).toBeUndefined();
});

test("an anonymous render that reads serverData stays bucketable", async () => {
  // Every anonymous visitor gets the same rows, so the signed-out entry is safe.
  const dir = makeApp(NOTES_PAGE);
  const anon = await render(dir, null);
  expect(anon.html).not.toContain("NOTE:");
  expect(anon.headers["x-pylon-bucket"]).toBe("300");
});

test("serverData.unsafe reads do not depend on the caller and stay bucketable", async () => {
  const dir = makeApp(UNSAFE_NOTES_PAGE);
  const signedIn = await render(dir, "alice");
  expect(signedIn.html).toContain("NOTE:bob-secret");
  expect(signedIn.headers["x-pylon-bucket"]).toBe("300");
});
