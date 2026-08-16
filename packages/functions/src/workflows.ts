/**
 * Durable workflows: authoring surface + the per-slice executor.
 *
 * A workflow is a TS function that composes `step` / `sleep` /
 * `waitForEvent` calls. The Rust WorkflowEngine owns the durable state
 * (instance, step results, wake timers — persisted in SQLite); this
 * module executes ONE slice at a time by REPLAY: on every advance the
 * whole workflow function re-runs, already-completed steps return their
 * recorded outputs without executing, and the first not-yet-done
 * primitive either executes (a `step` at the current index) or pauses
 * the run (`sleep` / `waitForEvent`).
 *
 * Determinism contract for authors: the sequence of step/sleep/
 * waitForEvent calls must be identical on every replay for the same
 * input + recorded outputs. Branch on `input` and on step OUTPUTS all
 * you like — never on wall-clock time, randomness, or external state
 * read outside a step.
 *
 * Files live in the app's `workflows/` directory (sibling of
 * `functions/`), one default-exported `workflow(...)` per file. The
 * runtime reports them in its ready handshake; the host registers them
 * and drives execution back through the function pool as an internal
 * `__pylon_workflow_run` action call — so step code runs with a full
 * ActionCtx (ctx.db, ctx.llm, ctx.scheduler, the idle timeout, and
 * cancellation) rather than in a bespoke side-channel process.
 */

import type { ActionCtx } from "./types";

/** Wire shape of one recorded step — mirrors Rust `StepResult` exactly. */
export interface WorkflowStepResult {
  step_id: string;
  name: string;
  status: "pending" | "running" | "completed" | "failed" | "skipped";
  output?: unknown;
  error?: string | null;
  started_at?: string | null;
  completed_at?: string | null;
  duration_ms?: number | null;
  retry_count?: number;
}

/** The advance request the Rust engine sends for one slice. */
export interface WorkflowRunRequest {
  workflow_id: string;
  workflow_name: string;
  input: unknown;
  current_step: number;
  completed_steps: WorkflowStepResult[];
}

/** The verdict of one slice — mirrors Rust `apply_response`'s actions. */
export type WorkflowRunnerResponse =
  | { action: "step_complete"; step_name: string; output: unknown; duration_ms: number }
  | { action: "sleep"; duration: string }
  | { action: "wait_event"; event: string }
  | { action: "complete"; output: unknown }
  | { action: "fail"; error: string; step_name?: string };

/** What a workflow function receives, besides the per-slice ActionCtx. */
export interface WorkflowRun<TInput = unknown> {
  /** The workflow instance id (stable across slices + restarts). */
  id: string;
  /** The input `start()` was called with. */
  input: TInput;
  /**
   * Run a named step exactly once. On replay a completed step returns
   * its recorded output without executing. Step names must be unique
   * within one workflow run — the replay cache is name-keyed.
   */
  step<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  /** Pause the workflow for a duration ("30s", "5m", "24h", "7d"). */
  sleep(duration: string): Promise<void>;
  /**
   * Pause until `POST /api/workflows/<id>/event` delivers this event.
   * Resolves with the event's data payload.
   */
  waitForEvent<T = unknown>(eventName: string): Promise<T>;
}

export interface WorkflowDefinition<TInput = unknown> {
  readonly __pylonWorkflow: true;
  name: string;
  description?: string;
  /** Max retries per step before the workflow fails (engine default: 3). */
  maxRetries?: number;
  fn: (wf: WorkflowRun<TInput>, ctx: ActionCtx) => Promise<unknown>;
}

/**
 * Declare a workflow. Default-export the result from a file in the
 * app's `workflows/` directory:
 *
 * ```ts
 * // workflows/onboarding.ts
 * import { workflow } from "@pylonsync/functions";
 *
 * export default workflow("onboarding", async (wf, ctx) => {
 *   const user = await wf.step("load-user", () =>
 *     ctx.runQuery("getUser", { id: wf.input.userId }),
 *   );
 *   await wf.step("send-welcome", () =>
 *     ctx.email.send({ to: user.email, subject: "Welcome!", text: "..." }),
 *   );
 *   await wf.sleep("24h");
 *   const confirmed = await wf.waitForEvent("email_confirmed");
 *   return { done: true, confirmed };
 * });
 * ```
 */
export function workflow<TInput = unknown>(
  name: string,
  fn: (wf: WorkflowRun<TInput>, ctx: ActionCtx) => Promise<unknown>,
  opts?: { description?: string; maxRetries?: number },
): WorkflowDefinition<TInput> {
  return {
    __pylonWorkflow: true,
    name,
    description: opts?.description,
    maxRetries: opts?.maxRetries,
    fn,
  };
}

/** Runtime shape check for a `workflows/` file's default export. */
export function isWorkflowDefinition(v: unknown): v is WorkflowDefinition {
  const w = v as WorkflowDefinition | undefined;
  return (
    !!w &&
    w.__pylonWorkflow === true &&
    typeof w.name === "string" &&
    typeof w.fn === "function"
  );
}

/** Control-flow sentinel: the slice reached a pause point. */
class WorkflowPaused extends Error {
  constructor(public response: WorkflowRunnerResponse) {
    super("workflow paused");
  }
}

/** Control-flow sentinel: a step past the current index was reached. */
class SliceBoundary extends Error {
  constructor(public response: WorkflowRunnerResponse) {
    super("slice boundary");
  }
}

/**
 * Execute one slice of `def` for `request`, returning the engine verdict.
 * Never throws for handler errors — a step failure becomes
 * `{action: "fail"}` so the engine's retry accounting runs.
 */
export async function executeWorkflowSlice(
  def: WorkflowDefinition,
  request: WorkflowRunRequest,
  ctx: ActionCtx,
): Promise<WorkflowRunnerResponse> {
  let index = 0;
  let currentStepName = "";

  const wf: WorkflowRun = {
    id: request.workflow_id,
    input: request.input,

    async step<T>(name: string, fn: () => Promise<T> | T): Promise<T> {
      const myIndex = index++;
      if (myIndex < request.current_step) {
        // Replay: return the recorded output. Name-keyed lookup so an
        // author inserting a step ahead of a recorded one fails loudly
        // (missing name) instead of silently reusing the wrong output.
        const done = request.completed_steps.find(
          (s) => s.name === name && s.status === "completed",
        );
        if (!done) {
          throw new Error(
            `workflow replay mismatch: step "${name}" (index ${myIndex}) has no recorded result — ` +
              "the step sequence must be deterministic across replays",
          );
        }
        return done.output as T;
      }
      if (myIndex === request.current_step) {
        currentStepName = name;
        const started = Date.now();
        const output = await fn();
        throw new SliceBoundary({
          action: "step_complete",
          step_name: name,
          output: output ?? null,
          duration_ms: Date.now() - started,
        });
      }
      throw new SliceBoundary({
        action: "fail",
        error: `internal: step "${name}" (index ${myIndex}) reached past the current slice`,
        step_name: name,
      });
    },

    async sleep(duration: string): Promise<void> {
      const myIndex = index++;
      if (myIndex < request.current_step) return; // already slept
      throw new WorkflowPaused({ action: "sleep", duration });
    },

    async waitForEvent<T>(eventName: string): Promise<T> {
      const myIndex = index++;
      // A delivered event is recorded as a completed step named
      // `event:<name>` (the Rust engine's send_event writes it).
      const delivered = request.completed_steps.find(
        (s) => s.name === `event:${eventName}` && s.status === "completed",
      );
      if (myIndex < request.current_step && delivered) {
        return delivered.output as T;
      }
      throw new WorkflowPaused({ action: "wait_event", event: eventName });
    },
  };

  try {
    const output = await def.fn(wf, ctx);
    return { action: "complete", output: output ?? null };
  } catch (err) {
    if (err instanceof SliceBoundary) return err.response;
    if (err instanceof WorkflowPaused) return err.response;
    const message =
      err instanceof Error ? err.message : String(err ?? "unknown error");
    return {
      action: "fail",
      error: message,
      step_name: currentStepName || undefined,
    };
  }
}
