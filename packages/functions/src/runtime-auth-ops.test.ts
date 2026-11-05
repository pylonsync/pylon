/**
 * `ctx.auth.providerToken` and `ctx.auth.verifyOidcToken` send `auth_op`
 * frames and hand back the host's reply.
 *
 * Drives the REAL runtime.ts dispatcher in a child process via a full
 * `call` frame and plays the host over stdin/stdout NDJSON, like the Rust
 * side. The host-side gate and store work are tested in Rust
 * (`auth_op_gate` in pylon-functions, `app_auth_ops` in pylon-runtime).
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
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-authop-"));
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

async function startCall(probeSource: string, callId: string) {
  const proc = spawnRuntime(probeSource);
  const readUntil = frameReader(proc);
  const send = (msg: Record<string, unknown>) =>
    (proc.stdin as import("bun").FileSink).write(JSON.stringify(msg) + "\n");
  await readUntil((fs) => fs.some((f) => f.type === "ready"));
  send({
    type: "call",
    call_id: callId,
    fn_name: "probe",
    fn_type: "action",
    args: {},
    auth: { user_id: "u1", is_admin: false, tenant_id: null },
  });
  return { proc, readUntil, send };
}

test("providerToken sends provider and userId and returns the host's token", async () => {
  const { proc, readUntil, send } = await startCall(
    `
export default {
  type: "action",
  handler: async (ctx) => {
    const mine = await ctx.auth.providerToken("stack0");
    const theirs = await ctx.auth.providerToken("stack0", { userId: "u2" });
    return { mine, theirs };
  },
};
`,
    "c_pt",
  );

  let frames = await readUntil((fs) => fs.some((f) => f.type === "auth_op"));
  expect(frames.find((f) => f.type === "auth_op")).toEqual({
    type: "auth_op",
    call_id: "c_pt",
    op: "provider_token",
    provider: "stack0",
    user_id: null,
  });
  send({
    type: "result",
    call_id: "c_pt",
    data: { accessToken: "at_1", expiresAt: 1_900_000_000, scope: "openid offline_access" },
  });

  frames = await readUntil(
    (fs) => fs.filter((f) => f.type === "auth_op").length >= 2,
  );
  expect(frames.filter((f) => f.type === "auth_op")[1]).toMatchObject({
    op: "provider_token",
    provider: "stack0",
    user_id: "u2",
  });
  // No linked account: the host replies null.
  send({ type: "result", call_id: "c_pt", data: null });

  frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  expect(frames.find((f) => f.type === "return")!.value).toEqual({
    mine: { accessToken: "at_1", expiresAt: 1_900_000_000, scope: "openid offline_access" },
    theirs: null,
  });
  proc.kill();
  await proc.exited;
});

test("providerToken surfaces REAUTH_REQUIRED as an error with that code", async () => {
  const { proc, readUntil, send } = await startCall(
    `
export default {
  type: "action",
  handler: async (ctx) => {
    try {
      await ctx.auth.providerToken("stack0");
      return { threw: false };
    } catch (e) {
      return { threw: true, code: e.code };
    }
  },
};
`,
    "c_re",
  );
  await readUntil((fs) => fs.some((f) => f.type === "auth_op"));
  send({
    type: "result",
    call_id: "c_re",
    error: { code: "REAUTH_REQUIRED", message: "sign in again" },
  });
  const frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  expect(frames.find((f) => f.type === "return")!.value).toEqual({
    threw: true,
    code: "REAUTH_REQUIRED",
  });
  proc.kill();
  await proc.exited;
});

test("verifyOidcToken sends the token and returns the host's answer", async () => {
  const { proc, readUntil, send } = await startCall(
    `
export default {
  type: "action",
  handler: async (ctx) => {
    const good = await ctx.auth.verifyOidcToken("tok_good");
    const bad = await ctx.auth.verifyOidcToken("tok_bad");
    return { good, bad };
  },
};
`,
    "c_v",
  );
  let frames = await readUntil((fs) => fs.some((f) => f.type === "auth_op"));
  expect(frames.find((f) => f.type === "auth_op")).toEqual({
    type: "auth_op",
    call_id: "c_v",
    op: "verify_oidc_token",
    token: "tok_good",
  });
  const who = { userId: "u9", clientId: "studio", scopes: ["openid"], expiresAt: 1_900_000_000 };
  send({ type: "result", call_id: "c_v", data: who });
  frames = await readUntil(
    (fs) => fs.filter((f) => f.type === "auth_op").length >= 2,
  );
  send({ type: "result", call_id: "c_v", data: null });
  frames = await readUntil((fs) => fs.some((f) => f.type === "return"));
  expect(frames.find((f) => f.type === "return")!.value).toEqual({ good: who, bad: null });
  proc.kill();
  await proc.exited;
});
