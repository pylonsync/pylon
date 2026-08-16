/**
 * End-to-end protocol test for workflow execution through the REAL
 * runtime (runtime.ts child process) — the seam the 2026-08 audit
 * flagged: the old workflow surface type-checked everywhere and
 * executed nowhere.
 *
 * The contract under test, wire-for-wire what the Rust host does:
 *
 *   1. A `workflows/` dir next to the functions dir is discovered at
 *      boot; the ready handshake declares each workflow AND registers
 *      the internal `__pylon_workflow_run` action.
 *   2. A `call` frame for `__pylon_workflow_run` with the engine's
 *      advance request executes ONE slice and returns the runner
 *      verdict as a normal `return` frame.
 *   3. Step code runs with a live ActionCtx — proven by a step that
 *      round-trips `ctx.runQuery` through the host (this test) before
 *      producing its output.
 */
import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");
const WORKFLOWS_MOD = join(import.meta.dir, "workflows.ts");

const WORKFLOW_FILE = `
import { workflow } from ${JSON.stringify(WORKFLOWS_MOD)};

export default workflow("greet", async (wf, ctx) => {
  const user = await wf.step("load-user", () =>
    ctx.runQuery("getUser", { id: wf.input.userId }),
  );
  const greeting = await wf.step("compose", () => \`Hello \${user.name}\`);
  return { greeting };
});
`;

function parseFrames(text: string): Record<string, unknown>[] {
  return text
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, unknown>);
}

test("workflows/ dir boots, declares in ready, and slices execute with a live ctx", async () => {
  // App root: empty functions/ + one workflow file.
  const appDir = mkdtempSync(join(tmpdir(), "pylon-wf-e2e-"));
  mkdirSync(join(appDir, "functions"));
  mkdirSync(join(appDir, "workflows"));
  writeFileSync(join(appDir, "workflows", "greet.ts"), WORKFLOW_FILE);

  // Spawn the real runtime exactly as the host does: cwd = app root,
  // argv[2] = functions dir.
  const proc = Bun.spawn([process.execPath, "run", RUNTIME, "functions"], {
    cwd: appDir,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });

  const reader = proc.stdout.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  const frames: Record<string, unknown>[] = [];
  async function readUntil(
    pred: (fs: Record<string, unknown>[]) => boolean,
  ): Promise<void> {
    while (!pred(frames)) {
      const { done, value } = await reader.read();
      if (done) throw new Error(`child exited early; frames: ${buffered}`);
      buffered += decoder.decode(value, { stream: true });
      frames.length = 0;
      frames.push(...parseFrames(buffered));
    }
  }
  const send = (msg: Record<string, unknown>) =>
    proc.stdin.write(JSON.stringify(msg) + "\n");

  // 1. Ready declares the workflow + the implicit runner action.
  await readUntil((fs) => fs.some((f) => f.type === "ready"));
  const ready = frames.find((f) => f.type === "ready") as {
    functions: Array<{ name: string; internal: boolean; auth: string }>;
    workflows: Array<{ name: string; max_retries: number | null }>;
  };
  expect(ready.workflows).toEqual([
    { name: "greet", description: "", max_retries: null },
  ]);
  const runFn = ready.functions.find((f) => f.name === "__pylon_workflow_run");
  expect(runFn).toBeDefined();
  expect(runFn!.internal).toBe(true);
  expect(runFn!.auth).toBe("admin");

  // 2. Slice 0: executes "load-user", whose body calls ctx.runQuery —
  //    answer it like the host's RunFn arm would.
  send({
    type: "call",
    call_id: "c_1",
    fn_name: "__pylon_workflow_run",
    fn_type: "action",
    auth: { user_id: null, is_admin: true },
    args: {
      workflow_id: "wf_1",
      workflow_name: "greet",
      input: { userId: "u1" },
      current_step: 0,
      completed_steps: [],
    },
  });
  await readUntil((fs) => fs.some((f) => f.type === "run_fn"));
  const runReq = frames.find((f) => f.type === "run_fn") as {
    call_id: string;
    fn_name: string;
    args: unknown;
  };
  expect(runReq.fn_name).toBe("getUser");
  expect(runReq.args).toEqual({ id: "u1" });
  send({
    type: "result",
    call_id: runReq.call_id,
    data: { id: "u1", name: "Ada" },
  });

  await readUntil((fs) =>
    fs.some((f) => f.type === "return" && f.call_id === "c_1"),
  );
  const slice0 = frames.find(
    (f) => f.type === "return" && f.call_id === "c_1",
  ) as { value: Record<string, unknown> };
  expect(slice0.value).toMatchObject({
    action: "step_complete",
    step_name: "load-user",
    output: { id: "u1", name: "Ada" },
  });

  // 3. Slice 1: replays load-user from the recorded output (NO run_fn
  //    round-trip this time), executes "compose".
  send({
    type: "call",
    call_id: "c_2",
    fn_name: "__pylon_workflow_run",
    fn_type: "action",
    auth: { user_id: null, is_admin: true },
    args: {
      workflow_id: "wf_1",
      workflow_name: "greet",
      input: { userId: "u1" },
      current_step: 1,
      completed_steps: [
        {
          step_id: "step_0",
          name: "load-user",
          status: "completed",
          output: { id: "u1", name: "Ada" },
        },
      ],
    },
  });
  await readUntil((fs) =>
    fs.some((f) => f.type === "return" && f.call_id === "c_2"),
  );
  const slice1 = frames.find(
    (f) => f.type === "return" && f.call_id === "c_2",
  ) as { value: Record<string, unknown> };
  expect(slice1.value).toMatchObject({
    action: "step_complete",
    step_name: "compose",
    output: "Hello Ada",
  });
  // Replay must not have re-fetched the user.
  expect(frames.filter((f) => f.type === "run_fn")).toHaveLength(1);

  // 4. Slice 2: both steps replay; the workflow completes.
  send({
    type: "call",
    call_id: "c_3",
    fn_name: "__pylon_workflow_run",
    fn_type: "action",
    auth: { user_id: null, is_admin: true },
    args: {
      workflow_id: "wf_1",
      workflow_name: "greet",
      input: { userId: "u1" },
      current_step: 2,
      completed_steps: [
        {
          step_id: "step_0",
          name: "load-user",
          status: "completed",
          output: { id: "u1", name: "Ada" },
        },
        {
          step_id: "step_1",
          name: "compose",
          status: "completed",
          output: "Hello Ada",
        },
      ],
    },
  });
  await readUntil((fs) =>
    fs.some((f) => f.type === "return" && f.call_id === "c_3"),
  );
  const slice2 = frames.find(
    (f) => f.type === "return" && f.call_id === "c_3",
  ) as { value: Record<string, unknown> };
  expect(slice2.value).toEqual({
    action: "complete",
    output: { greeting: "Hello Ada" },
  });

  proc.kill();
  await proc.exited;
}, 30_000);
