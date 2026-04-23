// ---------------------------------------------------------------------------
// @pylonsync/workflows — define durable, multi-step workflows in TypeScript
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

export interface WorkflowContext {
  /** The workflow instance ID. */
  id: string;
  /** The workflow name. */
  name: string;
  /** The input data passed when starting the workflow. */
  input: any;
  /** Current step index. */
  currentStep: number;
  /** Results from previously completed steps. */
  completedSteps: StepResult[];
}

export interface StepResult {
  step_id: string;
  name: string;
  status: "pending" | "running" | "completed" | "failed" | "skipped";
  output?: any;
  error?: string;
  started_at?: string;
  completed_at?: string;
  duration_ms?: number;
  retry_count: number;
}

// ---------------------------------------------------------------------------
// Step function types
// ---------------------------------------------------------------------------

export type StepFn<T = any> = (ctx: WorkflowContext) => Promise<T>;

export interface WorkflowStep {
  name: string;
  fn: StepFn;
}

export interface SleepStep {
  type: "sleep";
  duration: string;
}

export interface WaitEventStep {
  type: "wait_event";
  event: string;
}

// ---------------------------------------------------------------------------
// Workflow helpers — passed to every workflow function
// ---------------------------------------------------------------------------

export interface WorkflowHelpers {
  /** Execute a named step. The step is durable — if the workflow restarts, completed steps are skipped. */
  step: <T>(name: string, fn: () => Promise<T>) => Promise<T>;
  /** Sleep for a duration. The workflow will be paused and resumed after the duration. */
  sleep: (duration: string) => Promise<void>;
  /** Wait for an external event. The workflow pauses until the event is received. */
  waitForEvent: (eventName: string) => Promise<any>;
}

// ---------------------------------------------------------------------------
// Workflow definition
// ---------------------------------------------------------------------------

export interface WorkflowDefinition {
  name: string;
  fn: (ctx: WorkflowContext, helpers: WorkflowHelpers) => Promise<any>;
}

/**
 * Define a workflow. Returns a workflow definition object.
 *
 * ```typescript
 * const myWorkflow = workflow('my-workflow', async (ctx, { step, sleep, waitForEvent }) => {
 *   const result = await step('fetch-data', async () => {
 *     return await fetchSomeData(ctx.input.url);
 *   });
 *
 *   await sleep('1h');
 *
 *   await step('process', async () => {
 *     return await processData(result);
 *   });
 *
 *   const approval = await waitForEvent('approval');
 *
 *   if (approval.approved) {
 *     await step('finalize', async () => {
 *       return await finalize();
 *     });
 *   }
 *
 *   return { done: true };
 * });
 * ```
 */
export function workflow(
  name: string,
  fn: (ctx: WorkflowContext, helpers: WorkflowHelpers) => Promise<any>,
): WorkflowDefinition {
  return { name, fn };
}

// ---------------------------------------------------------------------------
// Runner response types
// ---------------------------------------------------------------------------

export type RunnerResponse =
  | { action: "step_complete"; step_name: string; output: any; duration_ms?: number }
  | { action: "sleep"; duration: string }
  | { action: "wait_event"; event: string }
  | { action: "complete"; output: any }
  | { action: "fail"; step_name?: string; error: string };

// ---------------------------------------------------------------------------
// Internal error types for control flow
// ---------------------------------------------------------------------------

class WorkflowPausedError extends Error {
  constructor(reason: string) {
    super(`Workflow paused: ${reason}`);
    this.name = "WorkflowPausedError";
  }
}

class StepNotReachedError extends Error {
  constructor(stepName: string) {
    super(`Step not reached: ${stepName}`);
    this.name = "StepNotReachedError";
  }
}

// ---------------------------------------------------------------------------
// Workflow runner
// ---------------------------------------------------------------------------

/**
 * The workflow runner processes step-by-step execution requests from the
 * Rust engine.
 *
 * The engine sends a request describing the workflow state (completed steps,
 * current step index, input). The runner replays completed steps by returning
 * cached results, then executes the next pending step and returns one of:
 *
 * - `{ action: "step_complete", step_name, output }` — step finished
 * - `{ action: "sleep", duration }`                  — workflow wants to sleep
 * - `{ action: "wait_event", event }`                — workflow awaits an event
 * - `{ action: "complete", output }`                 — workflow finished
 * - `{ action: "fail", error }`                      — step or workflow failed
 */
export class WorkflowRunner {
  private registry: Map<string, WorkflowDefinition>;

  constructor(registry: Map<string, WorkflowDefinition>) {
    this.registry = registry;
  }

  /**
   * Handle a step execution request from the engine.
   * Uses a continuation-passing approach: replays completed steps,
   * then executes the next one and returns the result.
   */
  async handleRequest(request: {
    workflow_id: string;
    workflow_name: string;
    input: any;
    current_step: number;
    completed_steps: StepResult[];
  }): Promise<RunnerResponse> {
    const def = this.registry.get(request.workflow_name);
    if (!def) {
      return { action: "fail", error: `Unknown workflow: ${request.workflow_name}` };
    }

    const ctx: WorkflowContext = {
      id: request.workflow_id,
      name: request.workflow_name,
      input: request.input,
      currentStep: request.current_step,
      completedSteps: request.completed_steps,
    };

    // Track which step we're on during replay.
    let stepIndex = 0;
    let pendingResponse: RunnerResponse | null = null;

    const helpers: WorkflowHelpers = {
      step: async <T>(name: string, fn: () => Promise<T>): Promise<T> => {
        const myIndex = stepIndex++;

        // If this step was already completed, return cached result.
        const existing = request.completed_steps.find(
          (s) => s.name === name && s.status === "completed",
        );
        if (existing && myIndex < request.current_step) {
          return existing.output as T;
        }

        // If we're at the current step index, this is the step to execute.
        if (myIndex === request.current_step) {
          const start = Date.now();
          try {
            const result = await fn();
            pendingResponse = {
              action: "step_complete",
              step_name: name,
              output: result,
              duration_ms: Date.now() - start,
            };
            return result;
          } catch (err: any) {
            pendingResponse = {
              action: "fail",
              step_name: name,
              error: err.message || String(err),
            };
            throw err; // Stop workflow execution.
          }
        }

        // Should not reach here in normal flow.
        throw new StepNotReachedError(name);
      },

      sleep: async (duration: string): Promise<void> => {
        const myIndex = stepIndex++;
        if (myIndex < request.current_step) {
          return; // Already slept.
        }
        if (myIndex === request.current_step) {
          pendingResponse = { action: "sleep", duration };
          throw new WorkflowPausedError("sleep");
        }
        throw new StepNotReachedError(`sleep:${duration}`);
      },

      waitForEvent: async (eventName: string): Promise<any> => {
        const myIndex = stepIndex++;

        // If event was already received, return its data.
        const existing = request.completed_steps.find(
          (s) => s.name === `event:${eventName}` && s.status === "completed",
        );
        if (existing && myIndex < request.current_step) {
          return existing.output;
        }

        if (myIndex === request.current_step) {
          pendingResponse = { action: "wait_event", event: eventName };
          throw new WorkflowPausedError("wait_event");
        }
        throw new StepNotReachedError(`event:${eventName}`);
      },
    };

    try {
      const output = await def.fn(ctx, helpers);
      // If we get here without a pending response, the workflow completed.
      if (pendingResponse) {
        return pendingResponse;
      }
      return { action: "complete", output };
    } catch (err) {
      if (err instanceof WorkflowPausedError && pendingResponse) {
        return pendingResponse;
      }
      if (err instanceof StepNotReachedError) {
        return { action: "fail", error: `Step sequencing error: ${err.message}` };
      }
      if (pendingResponse) {
        return pendingResponse;
      }
      return { action: "fail", error: (err as any).message || String(err) };
    }
  }

  /**
   * Start an HTTP server that handles workflow execution requests.
   * The Rust engine sends POST requests with step execution payloads.
   */
  serve(port: number = 4500): void {
    const runner = this;

    const server = Bun.serve({
      port,
      async fetch(req) {
        if (req.method !== "POST") {
          return new Response(JSON.stringify({ error: "Method not allowed" }), {
            status: 405,
            headers: { "Content-Type": "application/json" },
          });
        }

        try {
          const body = await req.json();
          const response = await runner.handleRequest(body);
          return new Response(JSON.stringify(response), {
            headers: { "Content-Type": "application/json" },
          });
        } catch (err: any) {
          return new Response(
            JSON.stringify({ action: "fail", error: err.message }),
            {
              status: 500,
              headers: { "Content-Type": "application/json" },
            },
          );
        }
      },
    });

    console.log(`Workflow runner listening on http://localhost:${server.port}`);
  }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/**
 * Create a workflow runner from an array of workflow definitions.
 *
 * ```typescript
 * const runner = createRunner([onboardingFlow, dataProcessingFlow]);
 * runner.serve(4500);
 * ```
 */
export function createRunner(workflows: WorkflowDefinition[]): WorkflowRunner {
  const registry = new Map<string, WorkflowDefinition>();
  for (const wf of workflows) {
    registry.set(wf.name, wf);
  }
  return new WorkflowRunner(registry);
}
