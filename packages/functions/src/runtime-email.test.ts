/**
 * `ctx.email.send` — options overload and size caps.
 *
 * The contract under test:
 *
 *   1. The positional form still emits the legacy frame shape.
 *   2. The options form emits `html` and snake_case attachments, with
 *      the `contentType` value passed through VERBATIM (parameterized
 *      types like `text/calendar; method=REQUEST` are load-bearing).
 *   3. Over-limit attachments throw BEFORE any frame is emitted.
 *
 * Same child-process NDJSON harness as runtime-rpc-queue.test.ts.
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

function spawnRuntime(probeSource: string) {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-email-"));
  mkdirSync(join(dir, "functions"));
  writeFileSync(join(dir, "functions", "probe.ts"), probeSource);
  return Bun.spawn([process.execPath, RUNTIME, "./functions"], {
    cwd: dir,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
}

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

test("options form emits html + snake_case attachments with verbatim contentType", async () => {
  const proc = spawnRuntime(`
export default {
  type: "action",
  handler: async (ctx) => {
    // Positional form first, then options form.
    await ctx.email.send("a@t.com", "Plain", "plain body");
    await ctx.email.send({
      to: "b@t.com",
      subject: "Invite",
      text: "You're invited",
      html: "<p>hi</p>",
      attachments: [{
        filename: "invite.ics",
        contentType: "text/calendar; method=REQUEST",
        content: "QkVHSU46VkNBTEVOREFS",
      }],
    });
    return { ok: true };
  },
};
`);
  const readUntil = frameReader(proc);
  const send = (msg: Record<string, unknown>) =>
    (proc.stdin as import("bun").FileSink).write(JSON.stringify(msg) + "\n");

  await readUntil((fs) => fs.some((f) => f.type === "ready"));
  send({
    type: "call",
    call_id: "c_1",
    fn_name: "probe",
    fn_type: "action",
    args: {},
    auth: { user_id: "u1", is_admin: false, tenant_id: null },
  });

  let frames = await readUntil((fs) => fs.some((f) => f.type === "send_email"));
  const first = frames.find((f) => f.type === "send_email")!;
  expect(first).toMatchObject({ to: "a@t.com", subject: "Plain", body: "plain body" });
  send({ type: "result", call_id: "c_1", data: { sent: true } });

  frames = await readUntil(
    (fs) => fs.filter((f) => f.type === "send_email").length >= 2,
  );
  const second = frames.filter((f) => f.type === "send_email")[1];
  expect(second).toMatchObject({
    to: "b@t.com",
    subject: "Invite",
    body: "You're invited",
    html: "<p>hi</p>",
    attachments: [
      {
        filename: "invite.ics",
        content_type: "text/calendar; method=REQUEST",
        content: "QkVHSU46VkNBTEVOREFS",
      },
    ],
  });
  send({ type: "result", call_id: "c_1", data: { sent: true } });

  frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  expect(frames.find((f) => f.type === "return")!.value).toEqual({ ok: true });

  proc.kill();
  await proc.exited;
});

test("oversized attachments throw before any frame is emitted", async () => {
  const proc = spawnRuntime(`
export default {
  type: "action",
  handler: async (ctx) => {
    try {
      await ctx.email.send({
        to: "b@t.com",
        subject: "Big",
        text: "x",
        attachments: [{
          filename: "huge.bin",
          contentType: "application/octet-stream",
          content: "A".repeat(16 * 1024 * 1024),
        }],
      });
      return { threw: false };
    } catch (err) {
      return { threw: true, message: String(err.message) };
    }
  },
};
`);
  const readUntil = frameReader(proc);
  const send = (msg: Record<string, unknown>) =>
    (proc.stdin as import("bun").FileSink).write(JSON.stringify(msg) + "\n");

  await readUntil((fs) => fs.some((f) => f.type === "ready"));
  send({
    type: "call",
    call_id: "c_2",
    fn_name: "probe",
    fn_type: "action",
    args: {},
    auth: { user_id: "u1", is_admin: false, tenant_id: null },
  });

  const frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  // The reject happened locally: no send_email frame ever hit the wire.
  expect(frames.filter((f) => f.type === "send_email")).toHaveLength(0);
  const ret = frames.find((f) => f.type === "return")!.value as {
    threw: boolean;
    message: string;
  };
  expect(ret.threw).toBe(true);
  expect(ret.message).toContain("exceeding");

  proc.kill();
  await proc.exited;
});
