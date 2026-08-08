/**
 * Legacy (call_id-keyed) RPCs QUEUE instead of rejecting.
 *
 * The contract under test:
 *
 *   1. An UN-AWAITED `ctx.auth.elevate(...)` followed by
 *      `ctx.scheduler.runAfter(...)` works: the elevate frame reaches
 *      the host BEFORE the schedule frame (wire order == call order),
 *      the schedule reply resolves with the real job id, and the call
 *      returns normally. Before the queue, this rejected with
 *      "Internal: concurrent RPC attempted on same call_id".
 *   2. `Promise.all` over two legacy RPCs serializes: both frames go
 *      out (second only after the first's reply) and each promise
 *      resolves with its own reply, not its neighbor's.
 *
 * Drives the REAL runtime.ts dispatcher in a child process via a full
 * `call` frame — the handler is loaded from a temp functions dir, and
 * we play the host over stdin/stdout NDJSON exactly like the Rust side.
 */
import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");

/** Collect NDJSON frames from a chunk of the child's stdout. */
function parseFrames(text: string): Record<string, unknown>[] {
  return text
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, unknown>);
}

/** Spawn the runtime against a temp functions dir holding `probe.ts`. */
function spawnRuntime(probeSource: string) {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-rpcq-"));
  mkdirSync(join(dir, "functions"));
  writeFileSync(join(dir, "functions", "probe.ts"), probeSource);
  const proc = Bun.spawn([process.execPath, RUNTIME, "./functions"], {
    cwd: dir,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  return proc;
}

/** Incremental frame reader over the child's stdout. */
function frameReader(proc: ReturnType<typeof Bun.spawn>) {
  const reader = (proc.stdout as ReadableStream<Uint8Array>).getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  return async function readUntil(
    pred: (frames: Record<string, unknown>[]) => boolean,
  ): Promise<Record<string, unknown>[]> {
    let frames = parseFrames(buffered);
    while (!pred(frames)) {
      const { done, value } = await reader.read();
      if (done) break;
      buffered += decoder.decode(value, { stream: true });
      frames = parseFrames(buffered);
    }
    return frames;
  };
}

test("un-awaited elevate then runAfter queues: elevate frame first, real job id back", async () => {
  const proc = spawnRuntime(`
export default {
  type: "action",
  handler: async (ctx) => {
    // Deliberately NOT awaited — the exact bug pattern under test.
    ctx.auth.elevate({ admin: true, reason: "test" });
    const jobId = await ctx.scheduler.runAfter(5000, "internalTarget", { n: 1 });
    return { jobId, isAdmin: ctx.auth.isAdmin };
  },
};
`);
  const readUntil = frameReader(proc);
  const send = (msg: Record<string, unknown>) =>
    (proc.stdin as import("bun").FileSink).write(JSON.stringify(msg) + "\n");

  await readUntil((fs) => fs.some((f) => f.type === "ready"));
  send({
    type: "call",
    call_id: "c_9",
    fn_name: "probe",
    fn_type: "action",
    args: {},
    auth: { user_id: "u1", is_admin: false, tenant_id: null },
  });

  // The elevate frame must arrive BEFORE any schedule frame — and with
  // its reply outstanding, the schedule frame must not have been sent.
  let frames = await readUntil((fs) => fs.some((f) => f.type === "elevate_auth"));
  expect(frames.filter((f) => f.type === "schedule")).toHaveLength(0);
  send({ type: "result", call_id: "c_9", data: { elevated: true } });

  frames = await readUntil((fs) => fs.some((f) => f.type === "schedule"));
  const schedule = frames.find((f) => f.type === "schedule")!;
  expect(schedule).toMatchObject({
    fn_name: "internalTarget",
    args: { n: 1 },
    delay_ms: 5000,
  });
  send({ type: "result", call_id: "c_9", data: { id: "job_123" } });

  frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  const ret = frames.find((f) => f.type === "return")!;
  expect(ret.value).toEqual({ jobId: "job_123", isAdmin: true });

  proc.kill();
  await proc.exited;
});

test("Promise.all over two legacy RPCs serializes and each resolves its own reply", async () => {
  const proc = spawnRuntime(`
export default {
  type: "action",
  handler: async (ctx) => {
    const [a, b] = await Promise.all([
      ctx.scheduler.runAfter(1000, "first", {}),
      ctx.scheduler.runAfter(2000, "second", {}),
    ]);
    return { a, b };
  },
};
`);
  const readUntil = frameReader(proc);
  const send = (msg: Record<string, unknown>) =>
    (proc.stdin as import("bun").FileSink).write(JSON.stringify(msg) + "\n");

  await readUntil((fs) => fs.some((f) => f.type === "ready"));
  send({
    type: "call",
    call_id: "c_10",
    fn_name: "probe",
    fn_type: "action",
    args: {},
    auth: { user_id: null, is_admin: true, tenant_id: null },
  });

  // First schedule frame out; the second must be held until we reply.
  let frames = await readUntil((fs) => fs.some((f) => f.type === "schedule"));
  expect(frames.filter((f) => f.type === "schedule")).toHaveLength(1);
  expect(frames.find((f) => f.type === "schedule")).toMatchObject({
    fn_name: "first",
  });
  send({ type: "result", call_id: "c_10", data: { id: "s1" } });

  frames = await readUntil(
    (fs) => fs.filter((f) => f.type === "schedule").length >= 2,
  );
  expect(frames.filter((f) => f.type === "schedule")[1]).toMatchObject({
    fn_name: "second",
  });
  send({ type: "result", call_id: "c_10", data: { id: "s2" } });

  frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  expect(frames.find((f) => f.type === "return")!.value).toEqual({
    a: "s1",
    b: "s2",
  });

  proc.kill();
  await proc.exited;
});
