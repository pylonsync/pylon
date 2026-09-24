/**
 * `ctx.shards` — the frames the runtime sends to the host (`ticket`,
 * `create`, `stop`, `get`, `list`) and the values it returns. The host signs (it holds the secret); this
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

/** Run one function in a real runtime child; `host` answers its frames. */
async function runFunction(
  source: string,
  host: (frame: Record<string, unknown>) => unknown,
  hostFrameTypes: string[],
  fnType: "mutation" | "action" = "mutation",
): Promise<{ value: unknown; hostFrames: Record<string, unknown>[] }> {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-shards-"));
  mkdirSync(join(dir, "functions"));
  writeFileSync(join(dir, "functions", "run.ts"), source);
  const proc = Bun.spawn([process.execPath, RUNTIME, "./functions"], {
    cwd: dir,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const reader = (proc.stdout as ReadableStream<Uint8Array>).getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  let consumed = 0;
  const next = async (): Promise<Record<string, unknown>> => {
    for (;;) {
      const frames = parseFrames(buffered);
      if (frames.length > consumed) return frames[consumed++];
      const { done, value } = await reader.read();
      if (done) throw new Error("runtime exited");
      buffered += decoder.decode(value, { stream: true });
    }
  };
  const send = (msg: Record<string, unknown>) =>
    (proc.stdin as import("bun").FileSink).write(JSON.stringify(msg) + "\n");

  while ((await next()).type !== "ready");
  send({
    type: "call",
    call_id: "c_1",
    fn_name: "run",
    fn_type: fnType,
    args: {},
    auth: { user_id: "u1", is_admin: false, tenant_id: null },
  });
  const hostFrames: Record<string, unknown>[] = [];
  try {
    for (;;) {
      const frame = await next();
      if (frame.type === "return") return { value: frame.value, hostFrames };
      if (frame.type === "error") throw new Error(JSON.stringify(frame));
      if (hostFrameTypes.includes(frame.type as string)) {
        hostFrames.push(frame);
        send({ type: "result", call_id: "c_1", data: host(frame) });
      }
    }
  } finally {
    proc.kill();
  }
}

test("ctx.shards.ticket sends sign_shard_ticket and returns the host's ticket", async () => {
  const { value, hostFrames } = await runFunction(
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
    () => "v1.payload.sig",
    ["sign_shard_ticket"],
  );
  expect(hostFrames[0]).toMatchObject({
    call_id: "c_1",
    shard: "zone-3",
    subscriber_id: "char_12",
    claims: { realm: "north" },
    ttl_secs: 30,
  });
  expect(value).toEqual({ ticket: "v1.payload.sig" });
});

test("ctx.shards.create/get/list/stop in an action send shard_op frames", async () => {
  const info = { id: "m1", kind: "arena", tick: 0, subscribers: 0, running: true };
  const { value, hostFrames } = await runFunction(
    `export default {
  type: "action",
  handler: async (ctx) => {
    const created = await ctx.shards.create("arena", "m1", { fog: true });
    const got = await ctx.shards.get("m1");
    const all = await ctx.shards.list();
    const stopped = await ctx.shards.stop("m1");
    return { created, got, all, stopped };
  },
};
`,
    (frame) => {
      switch (frame.op) {
        case "create":
        case "get":
          return info;
        case "list":
          return [info];
        default:
          return true;
      }
    },
    ["shard_op"],
    "action",
  );
  expect(hostFrames.map((f) => f.op)).toEqual(["create", "get", "list", "stop"]);
  expect(hostFrames[0]).toMatchObject({ kind: "arena", id: "m1", params: { fog: true } });
  expect(hostFrames[3]).toMatchObject({ id: "m1" });
  expect(value).toEqual({ created: info, got: info, all: [info], stopped: true });
});
