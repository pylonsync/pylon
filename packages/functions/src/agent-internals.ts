/**
 * Internal functions backing the `agent()` loop. Registered by the
 * runtime loader only when the app declares at least one agent.
 *
 * Both run under the CALLING user's auth (internal fns inherit the
 * wrapping handler's auth — they are not admin), so every op verifies
 * run ownership itself and stamps identity from `ctx.auth`, never from
 * args. `internal: true` keeps them off the HTTP surface; the
 * `__pylon_` prefix keeps their timeout out of the runner's
 * wedge-probe budget.
 */

import type { FnDefinition, MutationCtx, QueryCtx } from "./types";

interface RunRow {
  id: string;
  agent: string;
  status: string;
  userId: string | null;
  [key: string]: unknown;
}

/** Bounds on the steering queue. A user typing at a busy run must not
 *  be able to grow one row without limit, and the drained text is
 *  replayed into model context, so it is billed. */
const MAX_PENDING_INPUTS = 32;
const MAX_PENDING_CHARS = 32 * 1024;

function nowIso(): string {
  return new Date().toISOString();
}

/** The steering queue as a string list, tolerating a null/garbled column. */
function pendingInputOf(run: Record<string, unknown>): string[] {
  const raw = run.pendingInput;
  if (!Array.isArray(raw)) return [];
  return raw.filter((v): v is string => typeof v === "string");
}

/** Ownership fence: admins pass; otherwise the run must belong to the
 *  caller. Missing and foreign runs are indistinguishable (NOT_FOUND). */
function requireOwnedRun(
  ctx: QueryCtx | MutationCtx,
  run: Record<string, unknown> | null,
): RunRow {
  const owned =
    run &&
    (ctx.auth.isAdmin ||
      (typeof run.userId === "string" && run.userId === ctx.auth.userId));
  if (!owned) {
    const err = new Error("Run not found");
    (err as { code?: string }).code = "RUN_NOT_FOUND";
    throw err;
  }
  return run as unknown as RunRow;
}

export function registerAgentInternals(
  registry: Map<string, FnDefinition>,
): void {
  registry.set("__pylon_agent_read", {
    type: "query",
    internal: true,
    auth: "user",
    handler: async (ctx: QueryCtx, args: Record<string, unknown>) => {
      const runId = String(args.runId ?? "");
      const run = await ctx.db.get("AgentRun", runId);
      const owned = requireOwnedRun(ctx, run);
      const messages = await ctx.db.query("AgentMessage", {
        runId,
        $order: { seq: "asc" },
      });
      return { run: owned, messages };
    },
  } as unknown as FnDefinition);

  // Liveness probe for a loop that is mid-turn. Deliberately does NOT
  // load the transcript the way __pylon_agent_read does — the loop
  // calls this on a timer while a tool handler runs, and pulling every
  // message each time would scale the poll cost with the conversation.
  registry.set("__pylon_agent_poll", {
    type: "query",
    internal: true,
    auth: "user",
    handler: async (ctx: QueryCtx, args: Record<string, unknown>) => {
      const run = requireOwnedRun(
        ctx,
        await ctx.db.get("AgentRun", String(args.runId ?? "")),
      );
      return {
        status: run.status,
        cancelRequested: run.cancelRequested === true,
        pending: pendingInputOf(run).length,
      };
    },
  } as unknown as FnDefinition);

  registry.set("__pylon_agent_write", {
    type: "mutation",
    internal: true,
    auth: "user",
    handler: async (ctx: MutationCtx, args: Record<string, unknown>) => {
      const op = String(args.op ?? "");
      switch (op) {
        case "createRun": {
          // Owner-scoped or nothing: a null-owner row would be
          // invisible to everyone (policies guard with
          // `auth.userId != null`), and before that guard existed it
          // was readable by ANONYMOUS callers (null == null evaluates
          // true in the policy engine). Refuse instead of storing an
          // orphan — cron/system invocations must call the agent under
          // a service user's auth.
          const userId = ctx.auth.userId;
          if (typeof userId !== "string" || userId === "") {
            const err = new Error(
              "Agent runs require a signed-in user; invoke the agent under a service user for system/cron calls",
            );
            (err as { code?: string }).code = "AGENT_REQUIRES_USER";
            throw err;
          }
          const id = await ctx.db.insert("AgentRun", {
            agent: String(args.agent ?? "agent"),
            status: "idle",
            userId,
            title: args.title ?? null,
            streamId: null,
            error: null,
            pendingInput: [],
            cancelRequested: false,
            steps: 0,
            createdAt: nowIso(),
            updatedAt: nowIso(),
          });
          return { id };
        }
        case "appendMessage": {
          const runId = String(args.runId ?? "");
          const run = requireOwnedRun(
            ctx,
            await ctx.db.get("AgentRun", runId),
          );
          // Single-writer per run (the loop rejects concurrent turns),
          // so a read-then-insert seq is race-free in practice; the
          // mutation's transaction covers the rest.
          const last = await ctx.db.query("AgentMessage", {
            runId,
            $order: { seq: "desc" },
            $limit: 1,
          });
          const seq =
            last.length > 0 ? (Number(last[0].seq) || 0) + 1 : 1;
          const id = await ctx.db.insert("AgentMessage", {
            runId,
            userId: run.userId,
            seq,
            role: String(args.role ?? "user"),
            content: args.content ?? null,
            createdAt: nowIso(),
          });
          await ctx.db.update("AgentRun", runId, { updatedAt: nowIso() });
          return { id, seq };
        }
        case "setStatus": {
          const runId = String(args.runId ?? "");
          const run = requireOwnedRun(ctx, await ctx.db.get("AgentRun", runId));
          // The authoritative claim on the run. A caller that finds
          // the run live queues its message instead of reaching here,
          // so this guards the narrower race: two turns that both read
          // a non-running row and both try to start generating. A
          // "running" row older than staleMs is a dead generation
          // (process crash) and may be taken over.
          if (args.guardNotRunning === true && run.status === "running") {
            const staleMs = Number(args.staleMs) || 0;
            const updatedAt = Date.parse(String(run.updatedAt ?? "")) || 0;
            if (Date.now() - updatedAt < staleMs) {
              const err = new Error(
                "This run is already generating — wait for it to finish",
              );
              (err as { code?: string }).code = "RUN_BUSY";
              throw err;
            }
          }
          const status = String(args.status ?? "idle");
          const patch: Record<string, unknown> = {
            status,
            updatedAt: nowIso(),
          };
          if (args.error !== undefined) patch.error = args.error;
          if (args.streamId !== undefined) patch.streamId = args.streamId;
          if (args.steps !== undefined) patch.steps = Number(args.steps) || 0;
          // Claiming the run starts a new generation, which supersedes
          // a cancel left set by one that died before acting on it.
          // Terminal statuses clear it because it has been honoured.
          if (status === "running" || status === "cancelled") {
            patch.cancelRequested = false;
          }
          const updated = await ctx.db.update("AgentRun", runId, patch);
          return { updated };
        }
        case "enqueueInput": {
          // Steering: the caller sent a message while a generation was
          // in flight. Queue it on the run instead of refusing it; the
          // loop folds it into the transcript at the next turn
          // boundary, where the message order stays replayable.
          const runId = String(args.runId ?? "");
          const run = requireOwnedRun(ctx, await ctx.db.get("AgentRun", runId));
          const input = String(args.input ?? "");
          if (input === "") {
            const err = new Error("Steering input must not be empty");
            (err as { code?: string }).code = "AGENT_INPUT_REQUIRED";
            throw err;
          }
          const queue = pendingInputOf(run);
          const chars =
            queue.reduce((n, s) => n + s.length, 0) + input.length;
          if (queue.length >= MAX_PENDING_INPUTS || chars > MAX_PENDING_CHARS) {
            const err = new Error(
              "Too much input queued for this run — wait for the agent to catch up",
            );
            (err as { code?: string }).code = "AGENT_INPUT_QUEUE_FULL";
            throw err;
          }
          queue.push(input);
          // Deliberately NOT touching updatedAt: that column is the
          // liveness clock the staleness takeover reads. Bumping it
          // here would let a user typing at a dead generation keep it
          // looking alive forever.
          await ctx.db.update("AgentRun", runId, { pendingInput: queue });
          return { queued: queue.length };
        }
        case "drainInput": {
          // One atomic turn-boundary check: take everything queued,
          // report whether cancel was requested, and (at the terminal
          // boundary) settle the run as completed only if nothing
          // arrived in the meantime. Doing all three in one
          // transaction is what closes the race where a message lands
          // between the last turn and the completed write.
          const runId = String(args.runId ?? "");
          const run = requireOwnedRun(ctx, await ctx.db.get("AgentRun", runId));
          const queue = pendingInputOf(run);
          const cancelRequested = run.cancelRequested === true;
          const patch: Record<string, unknown> = { updatedAt: nowIso() };
          if (queue.length > 0) patch.pendingInput = [];
          if (args.steps !== undefined) patch.steps = Number(args.steps) || 0;
          let completed = false;
          if (
            queue.length === 0 &&
            !cancelRequested &&
            args.completeIfEmpty === true
          ) {
            patch.status = "completed";
            completed = true;
          }
          await ctx.db.update("AgentRun", runId, patch);
          return { input: queue, cancelRequested, completed };
        }
        case "requestCancel": {
          const runId = String(args.runId ?? "");
          const run = requireOwnedRun(ctx, await ctx.db.get("AgentRun", runId));
          const agent = args.agent === undefined ? null : String(args.agent);
          if (agent !== null && run.agent !== agent) {
            const err = new Error(
              `Run ${runId} belongs to agent "${run.agent}"`,
            );
            (err as { code?: string }).code = "AGENT_MISMATCH";
            throw err;
          }
          if (run.status === "running") {
            // A generation is live. Raise the flag and let the loop
            // honour it at its next boundary — it owns the terminal
            // write, so racing it here would garble the transcript.
            // updatedAt stays put so the staleness clock keeps running.
            await ctx.db.update("AgentRun", runId, { cancelRequested: true });
            return { status: "running", accepted: true };
          }
          // Nothing is generating, so there is no loop to honour it:
          // settle terminally and drop anything queued.
          await ctx.db.update("AgentRun", runId, {
            status: "cancelled",
            cancelRequested: false,
            pendingInput: [],
            updatedAt: nowIso(),
          });
          return { status: "cancelled", accepted: true };
        }
        default: {
          const err = new Error(`Unknown agent write op "${op}"`);
          (err as { code?: string }).code = "INVALID_OP";
          throw err;
        }
      }
    },
  } as unknown as FnDefinition);
}
