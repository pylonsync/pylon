/**
 * `ctx.shards.ticket` — the frame the runtime sends to the host, and the
 * ticket string it returns. The host signs (it holds the secret); this
 * checks the TS half. Same child-process NDJSON harness as
 * runtime-email.test.ts.
 */
import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");

function parseFrames(text: string): Record<string, unknown>[] {
  return text
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, unknown>);
}

test("ctx.shards.ticket sends sign_shard_ticket and returns the host's ticket", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-ticket-"));
  mkdirSync(join(dir, "functions"));
  writeFileSync(
    join(dir, "functions", "join.ts"),
    `export default {
  type: "mutation",
  handler: async (ctx) => {
    const ticket = await ctx.shards.ticket("zone-3", {
      subscriberId: "char_12",
      claims: { realm: "north" },
      ttlSecs: 30,
    });
    return { ticket };
  },
};
`,
  );
  const proc = Bun.spawn([process.execPath, RUNTIME, "./functions"], {
    cwd: dir,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const reader = (proc.stdout as ReadableStream<Uint8Array>).getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  const readUntil = async (pred: (f: Record<string, unknown>[]) => boolean) => {
    let frames = parseFrames(buffered);
    while (!pred(frames)) {
      const { done, value } = await reader.read();
      if (done) break;
      buffered += decoder.decode(value, { stream: true });
      frames = parseFrames(buffered);
    }
    return frames;
  };
  const send = (msg: Record<string, unknown>) =>
    (proc.stdin as import("bun").FileSink).write(JSON.stringify(msg) + "\n");

  await readUntil((fs) => fs.some((f) => f.type === "ready"));
  send({
    type: "call",
    call_id: "c_1",
    fn_name: "join",
    fn_type: "mutation",
    args: {},
    auth: { user_id: "u1", is_admin: false, tenant_id: null },
  });

  let frames = await readUntil((fs) => fs.some((f) => f.type === "sign_shard_ticket"));
  expect(frames.find((f) => f.type === "sign_shard_ticket")).toMatchObject({
    call_id: "c_1",
    shard: "zone-3",
    subscriber_id: "char_12",
    claims: { realm: "north" },
    ttl_secs: 30,
  });
  send({ type: "result", call_id: "c_1", data: "v1.payload.sig" });

  frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  expect(frames.find((f) => f.type === "return")).toMatchObject({
    call_id: "c_1",
    value: { ticket: "v1.payload.sig" },
  });
  proc.kill();
});
