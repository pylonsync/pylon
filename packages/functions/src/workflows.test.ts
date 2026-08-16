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

  test("a stepless workflow completes on its first slice", async () => {
    const trivial = workflow("noop", async (wf) => ({ echoed: wf.input }));
    const res = await executeWorkflowSlice(trivial, req(0, [], 42), ctx);
    expect(res).toEqual({ action: "complete", output: { echoed: 42 } });
  });
});
