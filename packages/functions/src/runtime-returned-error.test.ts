/**
 * A handler that RETURNS `ctx.error(...)` instead of throwing it must
 * still fail the call. An Error has no JSON shape, so a returned one
 * used to reach the client as HTTP 200 with `{}` — a forged proof
 * "passed" because the author wrote `return` where `throw` belonged.
 *
 * Same child-process harness as runtime-db.test.ts: runtime.ts runs
 * main() on import, loads the functions dir from argv[2], and speaks
 * NDJSON on stdin/stdout like the Rust host.
 */
import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");
const DEFINE = join(import.meta.dir, "index.ts");

function frames(text: string): Record<string, unknown>[] {
  return text
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, unknown>);
}

test("a returned ctx.error rejects the call with its code", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-returned-error-"));
  mkdirSync(join(dir, "functions"));
  writeFileSync(
    join(dir, "functions", "checkProof.ts"),
    `import { query } from ${JSON.stringify(DEFINE)};
export default query({
  args: {},
  auth: "public",
  async handler(ctx) {
    // Wrong on purpose: return instead of throw.
    return ctx.error("INVALID_PROOF", "proof rejected") as unknown as { ok: boolean };
  },
});
`,
  );
  writeFileSync(
    join(dir, "functions", "plain.ts"),
    `import { query } from ${JSON.stringify(DEFINE)};
export default query({
  args: {},
  auth: "public",
  async handler() {
    return { ok: true };
  },
});
`,
  );

  const proc = Bun.spawn([process.execPath, RUNTIME, "functions"], {
    cwd: dir,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });

  const reader = proc.stdout.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  let seen: Record<string, unknown>[] = [];
  const until = async (pred: (f: Record<string, unknown>[]) => boolean) => {
    while (!pred(seen)) {
      const { done, value } = await reader.read();
      if (done) break;
      buffered += decoder.decode(value, { stream: true });
      seen = frames(buffered);
    }
  };

  await until((f) => f.some((m) => m.type === "ready"));
  const send = (msg: Record<string, unknown>) =>
    proc.stdin.write(JSON.stringify(msg) + "\n");
  const auth = { user_id: null, is_admin: false };
  send({ type: "call", call_id: "c_bad", fn_name: "checkProof", fn_type: "query", args: {}, auth });
  send({ type: "call", call_id: "c_ok", fn_name: "plain", fn_type: "query", args: {}, auth });
  await proc.stdin.flush();
  await until((f) => f.some((m) => m.call_id === "c_bad") && f.some((m) => m.call_id === "c_ok"));
  proc.kill();

  const bad = seen.find((m) => m.call_id === "c_bad");
  expect(bad).toMatchObject({ type: "error", code: "INVALID_PROOF", message: "proof rejected" });
  const ok = seen.find((m) => m.call_id === "c_ok");
  expect(ok).toMatchObject({ type: "return", value: { ok: true } });
});
