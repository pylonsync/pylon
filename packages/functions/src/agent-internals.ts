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

function nowIso(): string {
  return new Date().toISOString();
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
          // The loop's pre-flight RUN_BUSY check races with itself
          // across concurrent turns; this in-transaction guard is the
          // authoritative claim. A "running" row older than staleMs is
          // a dead generation (process crash) and may be taken over.
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
          const patch: Record<string, unknown> = {
            status: String(args.status ?? "idle"),
            updatedAt: nowIso(),
          };
          if (args.error !== undefined) patch.error = args.error;
          if (args.streamId !== undefined) patch.streamId = args.streamId;
          const updated = await ctx.db.update("AgentRun", runId, patch);
          return { updated };
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
