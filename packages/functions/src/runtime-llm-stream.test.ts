/**
 * Tests for `ctx.llm.stream` and `ctx.rooms.broadcast` (runtime.ts).
 *
 * The contract under test:
 *
 *   1. `stream()` emits ONE `llm_stream` frame carrying an `op_id`.
 *   2. `llm_event` frames addressed to that op_id reach the caller's
 *      `onEvent` in order, WITHOUT settling the promise.
 *   3. The terminal `result` for that op_id resolves the promise with
 *      the assembled response.
 *   4. Events for a different op_id never leak into this stream's
 *      callback — two concurrent streams stay separated.
 *   5. A throwing `onEvent` doesn't abandon the RPC; the stream still
 *      resolves.
 *   6. `rooms.broadcast` emits a `room_broadcast` frame and resolves
 *      with the host's `{delivered}` verdict.
 *
 * runtime.ts runs main() on import (it IS the bun runner entrypoint),
 * so this drives the REAL dispatcher in a child process: we write host
 * frames to its stdin and read what it emits on stdout, exactly as the
 * Rust host would.
 */
import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");

const SCRIPT = `
import { buildLlm, buildRooms } from ${JSON.stringify(RUNTIME)};

const llm = buildLlm("c_1");
const rooms = buildRooms("c_2");

const seen = [];
const other = [];

// Stream A — the one under test. Its onEvent records every event.
const a = llm.stream({ messages: [] }, (e) => { seen.push(e); });
// Stream B — concurrent, and its callback THROWS. Neither may disturb A.
const b = llm.stream({ messages: [] }, (e) => {
  other.push(e);
  throw new Error("handler callback blew up");
});
const r = rooms.broadcast("room-1", "agent.delta", { text: "hi" });

const [aRes, bRes, rRes] = await Promise.all([a, b, r]);

console.error("RESULT " + JSON.stringify({
  seen,
  other,
  aStop: aRes.stop_reason,
  bStop: bRes.stop_reason,
  delivered: rRes.delivered,
}));
process.exit(0);
`;

/** Collect NDJSON frames from a chunk of the child's stdout. */
function parseFrames(text: string): Record<string, unknown>[] {
  return text
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, unknown>);
}

test("llm.stream routes events by op_id, resolves on result; rooms.broadcast round-trips", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-llm-"));
  const scriptPath = join(dir, "probe.ts");
  writeFileSync(scriptPath, SCRIPT);

  // Spawn the bun that's already running, by absolute path — resolving
  // "bun" off PATH depends on the process cwd, which other test files
  // in this suite move around.
  const proc = Bun.spawn([process.execPath, scriptPath], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });

  // Read the child's request frames incrementally — we need its op_ids
  // before we can address replies back to it.
  const reader = proc.stdout.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  // Wait for the three frames we care about. main() also emits a
  // one-shot `ready` handshake on import, so count by type rather than
  // by total frames.
  let requests: Record<string, unknown>[] = [];
  const wanted = (fs: Record<string, unknown>[]) =>
    fs.filter((f) => f.type === "llm_stream").length >= 2 &&
    fs.some((f) => f.type === "room_broadcast");
  while (!wanted(requests)) {
    const { done, value } = await reader.read();
    if (done) break;
    buffered += decoder.decode(value, { stream: true });
    requests = parseFrames(buffered);
  }

  const streams = requests.filter((f) => f.type === "llm_stream");
  const broadcasts = requests.filter((f) => f.type === "room_broadcast");
  expect(streams).toHaveLength(2);
  expect(broadcasts).toHaveLength(1);

  // 1. Each stream frame carries a distinct op_id — that's what makes
  //    concurrent streams routable.
  const opA = streams[0].op_id as string;
  const opB = streams[1].op_id as string;
  expect(opA).toBeTruthy();
  expect(opB).toBeTruthy();
  expect(opA).not.toBe(opB);

  // 2. Broadcast frame carries the room, topic, and payload verbatim.
  expect(broadcasts[0]).toMatchObject({
    room: "room-1",
    topic: "agent.delta",
    data: { text: "hi" },
  });

  const send = (msg: Record<string, unknown>) =>
    proc.stdin.write(JSON.stringify(msg) + "\n");

  // Interleave A's and B's events so a routing bug (e.g. keying on
  // call_id, or a single global sink) shows up as cross-talk.
  send({ type: "llm_event", call_id: "c_1", op_id: opA, event: { type: "text_delta", text: "He" } });
  send({ type: "llm_event", call_id: "c_1", op_id: opB, event: { type: "text_delta", text: "XX" } });
  send({ type: "llm_event", call_id: "c_1", op_id: opA, event: { type: "text_delta", text: "llo" } });
  send({
    type: "llm_event",
    call_id: "c_1",
    op_id: opA,
    event: { type: "tool_use_start", id: "t1", name: "search" },
  });
  send({
    type: "llm_event",
    call_id: "c_1",
    op_id: opA,
    event: { type: "tool_input_delta", partial_json: '{"q":' },
  });

  // Terminal results settle each stream.
  send({
    type: "result",
    call_id: "c_1",
    op_id: opA,
    data: { model: "m", content: [], stop_reason: "tool_use", usage: { input_tokens: 1, output_tokens: 2 } },
  });
  send({
    type: "result",
    call_id: "c_1",
    op_id: opB,
    data: { model: "m", content: [], stop_reason: "end_turn", usage: { input_tokens: 0, output_tokens: 0 } },
  });
  send({
    type: "result",
    call_id: "c_2",
    op_id: broadcasts[0].op_id,
    data: { delivered: true },
  });
  await proc.stdin.flush();

  const stderr = await new Response(proc.stderr).text();
  await proc.exited;

  const line = stderr.split("\n").find((l) => l.includes("RESULT "));
  expect(line).toBeDefined();
  const out = JSON.parse(line!.slice(line!.indexOf("RESULT ") + 7));

  // 3+4. A saw exactly its own events, in order — B's "XX" never leaked.
  expect(out.seen).toEqual([
    { type: "text_delta", text: "He" },
    { type: "text_delta", text: "llo" },
    { type: "tool_use_start", id: "t1", name: "search" },
    { type: "tool_input_delta", partial_json: '{"q":' },
  ]);
  expect(out.other).toEqual([{ type: "text_delta", text: "XX" }]);

  // 5. Both promises resolved with their OWN response — including B,
  //    whose callback threw on every event.
  expect(out.aStop).toBe("tool_use");
  expect(out.bStop).toBe("end_turn");

  // 6. rooms.broadcast surfaced the host's verdict.
  expect(out.delivered).toBe(true);
});
