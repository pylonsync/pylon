/**
 * Tests for the workflow slice executor (workflows.ts).
 *
 * The replay contract under test, in terms of the Rust engine's
 * request/response protocol:
 *   - a step at the current index EXECUTES and yields step_complete
 *   - a step below the current index REPLAYS its recorded output
 *   - sleep / waitForEvent at the frontier pause the run
 *   - a delivered event (recorded as `event:<name>`) resumes with data
 *   - falling off the end of the workflow fn yields complete
 *   - a throwing step yields fail with the step's name
 *   - a nondeterministic replay (missing recorded step) yields fail
 */
import { describe, expect, test } from "bun:test";
import {
  executeWorkflowSlice,
  isWorkflowDefinition,
  workflow,
  type WorkflowRunRequest,
  type WorkflowStepResult,
} from "./workflows";
import type { ActionCtx } from "./types";

// The executor never touches ctx itself — it only passes it through to
// step closures. A cast-through-unknown stub is enough.
const ctx = {} as unknown as ActionCtx;

function req(
  currentStep: number,
  completed: WorkflowStepResult[],
  input: unknown = { userId: "u1" },
): WorkflowRunRequest {
  return {
    workflow_id: "wf_1",
    workflow_name: "onboarding",
    input,
    current_step: currentStep,
    completed_steps: completed,
  };
}

function done(name: string, output: unknown): WorkflowStepResult {
  return { step_id: "s", name, status: "completed", output };
}

const executed: string[] = [];

const def = workflow("onboarding", async (wf, _ctx) => {
  const user = await wf.step("load-user", () => {
    executed.push("load-user");
    return { email: "a@b.co" };
  });
  await wf.step("send-welcome", () => {
    executed.push("send-welcome");
    return { sent: true, to: user.email };
  });
  await wf.sleep("24h");
  const confirmation = await wf.waitForEvent<{ ok: boolean }>("confirmed");
  return { finished: true, ok: confirmation.ok };
});

describe("executeWorkflowSlice", () => {
  test("workflow() output passes the loader's shape check", () => {
    expect(isWorkflowDefinition(def)).toBe(true);
    expect(isWorkflowDefinition({ name: "x" })).toBe(false);
  });

  test("slice 0: executes the first step only", async () => {
    executed.length = 0;
    const res = await executeWorkflowSlice(def, req(0, []), ctx);
    expect(res).toMatchObject({
      action: "step_complete",
      step_name: "load-user",
      output: { email: "a@b.co" },
    });
    expect(executed).toEqual(["load-user"]);
  });

  test("slice 1: replays step 0's output, executes step 1", async () => {
    executed.length = 0;
    const res = await executeWorkflowSlice(
      def,
      req(1, [done("load-user", { email: "recorded@b.co" })]),
      ctx,
    );
    expect(res).toMatchObject({
      action: "step_complete",
      step_name: "send-welcome",
      // Proof the replay fed the RECORDED output into the live closure.
      output: { sent: true, to: "recorded@b.co" },
    });
    expect(executed).toEqual(["send-welcome"]);
  });

  test("slice 2: pauses at sleep without re-executing steps", async () => {
    executed.length = 0;
    const res = await executeWorkflowSlice(
      def,
      req(2, [
        done("load-user", { email: "a@b.co" }),
        done("send-welcome", { sent: true }),
      ]),
      ctx,
    );
    expect(res).toEqual({ action: "sleep", duration: "24h" });
    expect(executed).toEqual([]);
  });

  test("slice 3 (woken): pauses at waitForEvent", async () => {
    const res = await executeWorkflowSlice(
      def,
      req(3, [
        done("load-user", { email: "a@b.co" }),
        done("send-welcome", { sent: true }),
      ]),
      ctx,
    );
    expect(res).toEqual({ action: "wait_event", event: "confirmed" });
  });

  test("slice 4 (event delivered): completes with the event data", async () => {
    const res = await executeWorkflowSlice(
      def,
      req(4, [
        done("load-user", { email: "a@b.co" }),
        done("send-welcome", { sent: true }),
        done("event:confirmed", { ok: true }),
      ]),
      ctx,
    );
    expect(res).toEqual({
      action: "complete",
      output: { finished: true, ok: true },
    });
  });

  test("a throwing step fails with its name — engine retry accounting keys on it", async () => {
    const failing = workflow("boom", async (wf) => {
      await wf.step("explode", () => {
        throw new Error("provider 500");
      });
    });
    const res = await executeWorkflowSlice(failing, req(0, []), ctx);
    expect(res).toEqual({
      action: "fail",
      error: "provider 500",
      step_name: "explode",
    });
  });

  test("nondeterministic replay (missing recorded step) fails loudly", async () => {
    // current_step says 2 steps are done, but only one was recorded
    // under a DIFFERENT name — the author reordered/renamed steps.
    const res = await executeWorkflowSlice(
      def,
      req(2, [done("some-old-name", {})]),
      ctx,
    );
    expect(res.action).toBe("fail");
    expect((res as { error: string }).error).toContain("replay mismatch");
  });

  test("duplicate step names fail loudly instead of replaying the wrong output", async () => {
    // The replay cache is name-keyed: without the uniqueness check, the
    // second "fetch" would silently replay the FIRST record's output on
    // every later slice — wrong data in a durability primitive.
    const dup = workflow("dup", async (wf) => {
      await wf.step("fetch", () => "A");
      await wf.step("fetch", () => "B");
    });
    const res = await executeWorkflowSlice(
      dup,
      req(1, [done("fetch", "A")]),
      ctx,
    );
    expect(res.action).toBe("fail");
    expect((res as { error: string }).error).toContain("duplicate step name");
  });

  test("the same event name can be awaited repeatedly; each wait replays in order", async () => {
    const cadence = workflow("cadence", async (wf) => {
      const first = await wf.waitForEvent<{ n: number }>("reply");
      const second = await wf.waitForEvent<{ n: number }>("reply");
      return { first: first.n, second: second.n };
    });
    const paused = await executeWorkflowSlice(
      cadence,
      req(1, [done("event:reply", { n: 1 })]),
      ctx,
    );
    expect(paused).toEqual({ action: "wait_event", event: "reply" });
    const res = await executeWorkflowSlice(
      cadence,
      req(2, [done("event:reply", { n: 1 }), done("event:reply", { n: 2 })]),
      ctx,
    );
    expect(res).toEqual({ action: "complete", output: { first: 1, second: 2 } });
  });

  test("waitForEvent with a timeout sends the timeout and resolves null on timeout", async () => {
    const lead = workflow("lead", async (wf) => {
      await wf.step("send-sms", () => "sent");
      const reply = await wf.waitForEvent("seller_replied", { timeout: "60s" });
      if (reply === null) {
        await wf.step("call", () => "called");
        return "called";
      }
      return "replied";
    });
    const paused = await executeWorkflowSlice(lead, req(1, [done("send-sms", "sent")]), ctx);
    expect(paused).toEqual({ action: "wait_event", event: "seller_replied", timeout: "60s" });

    const timedOut = await executeWorkflowSlice(
      lead,
      req(2, [done("send-sms", "sent"), done("timeout:seller_replied", null)]),
      ctx,
    );
    expect(timedOut).toMatchObject({ action: "step_complete", step_name: "call" });

    const replied = await executeWorkflowSlice(
      lead,
      req(2, [done("send-sms", "sent"), done("event:seller_replied", { body: "hi" })]),
      ctx,
    );
    expect(replied).toEqual({ action: "complete", output: "replied" });
  });

  test("an invalid timeout or sleep duration fails the slice", async () => {
    const badWait = workflow("bad-wait", async (wf) => {
      await wf.waitForEvent("x", { timeout: "soon" });
    });
    const res = await executeWorkflowSlice(badWait, req(0, []), ctx);
    expect(res.action).toBe("fail");
    expect((res as { error: string }).error).toContain("invalid duration");

    const badSleep = workflow("bad-sleep", async (wf) => {
      await wf.sleep("5 minutes");
    });
    const res2 = await executeWorkflowSlice(badSleep, req(0, []), ctx);
    expect(res2.action).toBe("fail");
  });

  test("a replayed wait with no record fails loudly", async () => {
    const w = workflow("w", async (wf) => {
      await wf.waitForEvent("x");
    });
    const res = await executeWorkflowSlice(w, req(1, [done("other", 1)]), ctx);
    expect(res.action).toBe("fail");
    expect((res as { error: string }).error).toContain("replay mismatch");
  });

  test("step names that collide with event records are rejected", async () => {
    const w = workflow("w", async (wf) => {
      await wf.step("event:x", () => 1);
    });
    const res = await executeWorkflowSlice(w, req(0, []), ctx);
    expect(res.action).toBe("fail");
    expect((res as { error: string }).error).toContain("reserved");
  });

  test("a stepless workflow completes on its first slice", async () => {
    const trivial = workflow("noop", async (wf) => ({ echoed: wf.input }));
    const res = await executeWorkflowSlice(trivial, req(0, [], 42), ctx);
    expect(res).toEqual({ action: "complete", output: { echoed: 42 } });
  });
});
