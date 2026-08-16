/**
 * Tests for host-initiated call cancellation (runtime.ts `cancel` frames).
 *
 * The contract under test:
 *
 *   1. A `cancel` frame for a call REJECTS that call's in-flight RPC
 *      promises with code CALL_CANCELLED (the handler's `await ctx.db.*`
 *      unwinds instead of hanging on a reply that will never come).
 *   2. Any LATER ctx RPC from the cancelled call throws CALL_CANCELLED —
 *      this is what actually stops a typical handler mid-loop.
 *   3. A different call on the same runner is untouched: its RPCs still
 *      round-trip normally. This is the whole point of the change — the
 *      host used to kill the entire child on one call's timeout.
 *
 * Same child-process harness as runtime-llm-stream.test.ts: runtime.ts
 * runs main() on import, so we drive the REAL dispatcher over NDJSON on
 * stdin/stdout exactly as the Rust host would.
 */
import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");

const SCRIPT = `
import { buildDbReader } from ${JSON.stringify(RUNTIME)};

const victim = buildDbReader("c_1");
const bystander = buildDbReader("c_2");

const out = {};

// In-flight RPC on the call the host will cancel.
const victimFirst = victim.get("User", "u1").then(
  () => ({ outcome: "resolved" }),
  (e) => ({ outcome: "rejected", code: e.code }),
);
// Concurrent RPC on an UNRELATED call — must complete normally.
const bystanderGet = bystander.get("User", "u2").then(
  (data) => ({ outcome: "resolved", data }),
  (e) => ({ outcome: "rejected", code: e.code }),
);

out.victimFirst = await victimFirst;
out.bystander = await bystanderGet;

// A LATER RPC from the cancelled call must throw synchronously.
try {
  await victim.get("User", "u3");
  out.victimSecond = { outcome: "resolved" };
} catch (e) {
  out.victimSecond = { outcome: "rejected", code: e.code };
}

console.error("RESULT " + JSON.stringify(out));
process.exit(0);
`;

function parseFrames(text: string): Record<string, unknown>[] {
  return text
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, unknown>);
}

test("cancel rejects the call's RPCs, poisons later ones, and spares co-tenant calls", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-cancel-"));
  const scriptPath = join(dir, "probe.ts");
  writeFileSync(scriptPath, SCRIPT);

  const proc = Bun.spawn([process.execPath, scriptPath], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });

  // Wait for both db frames (c_1's and c_2's) so we know their op_ids.
  const reader = proc.stdout.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  let requests: Record<string, unknown>[] = [];
  const wanted = (fs: Record<string, unknown>[]) =>
    fs.filter((f) => f.type === "db").length >= 2;
  while (!wanted(requests)) {
    const { done, value } = await reader.read();
    if (done) break;
    buffered += decoder.decode(value, { stream: true });
    requests = parseFrames(buffered);
  }

  const dbFrames = requests.filter((f) => f.type === "db");
  const victimOp = dbFrames.find((f) => f.call_id === "c_1")?.op_id as string;
  const bystanderOp = dbFrames.find((f) => f.call_id === "c_2")
    ?.op_id as string;
  expect(victimOp).toBeTruthy();
  expect(bystanderOp).toBeTruthy();

  const send = (msg: Record<string, unknown>) =>
    proc.stdin.write(JSON.stringify(msg) + "\n");

  // Cancel c_1 while its RPC is in flight; answer c_2 normally.
  send({ type: "cancel", call_id: "c_1", reason: "idle timeout 30s exceeded" });
  send({
    type: "result",
    call_id: "c_2",
    op_id: bystanderOp,
    data: { id: "u2", name: "Bystander" },
  });
  await proc.stdin.flush();

  const stderr = await new Response(proc.stderr).text();
  await proc.exited;

  const line = stderr.split("\n").find((l) => l.includes("RESULT "));
  expect(line).toBeDefined();
  const out = JSON.parse(line!.slice(line!.indexOf("RESULT ") + 7));

  // 1. The in-flight RPC rejected with the cancellation code — no hang,
  //    no generic RPC-timeout 60s later.
  expect(out.victimFirst).toEqual({
    outcome: "rejected",
    code: "CALL_CANCELLED",
  });

  // 2. The later RPC threw immediately with the same code.
  expect(out.victimSecond).toEqual({
    outcome: "rejected",
    code: "CALL_CANCELLED",
  });

  // 3. The co-tenant call was untouched.
  expect(out.bystander).toEqual({
    outcome: "resolved",
    data: { id: "u2", name: "Bystander" },
  });
});
