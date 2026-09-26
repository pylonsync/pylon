import { mutation, v } from "@pylonsync/functions";
import { queueOutbound, toContact, toConversation } from "../lib/store";

// Close a turn claimed by beginTurn: settle its inbound texts, store the
// reply bubbles and queue them for delivery, record the agent run, and
// schedule another turn when texts arrived meanwhile. Internal: processTurn only.
export default mutation({
  internal: true,
  auth: "admin",
  args: {
    conversationId: v.string(),
    turnId: v.string(),
    outcome: v.union(v.literal("answered"), v.literal("awaiting_setup"), v.literal("failed")),
    detail: v.optional(v.string()),
    runId: v.optional(v.string()),
    chunks: v.optional(v.array(v.string())),
  },
  async handler(ctx, args) {
    const row = await ctx.db.get("Conversation", args.conversationId);
    if (!row) return { ok: false };
    const conversation = toConversation(row);
    // A sweep may have taken the turn over; a stale finisher must not clobber it.
    if (conversation.turnId !== args.turnId) return { ok: false };

    const turnMessages = await ctx.db.query("Message", {
      conversationId: conversation.id,
      turnId: args.turnId,
    });
    const now = new Date().toISOString();
    for (const m of turnMessages) {
      if (m.direction !== "in" || m.status !== "processing") continue;
      await ctx.db.update("Message", String(m.id), {
        status: args.outcome,
        detail: args.outcome === "answered" ? null : (args.detail ?? null),
        agentRunId: args.runId ?? null,
      });
    }

    const contactRow = await ctx.db.get("Contact", conversation.contactId);
    const contact = contactRow ? toContact(contactRow) : null;
    // Re-check at send time: the owner may have blocked the contact, or the
    // contact may have texted STOP, while the model was thinking.
    const maySend = contact !== null && contact.status === "allowed" && !contact.optedOut;
    if (args.outcome === "answered" && maySend) {
      for (const text of args.chunks ?? []) {
        await queueOutbound(ctx, {
          conversationId: conversation.id,
          contactId: conversation.contactId,
          handle: conversation.handle,
          text,
          kind: "agent",
          turnId: args.turnId,
          agentRunId: args.runId ?? null,
        });
      }
    }

    const patch: Record<string, unknown> = {
      turnState: "idle",
      turnId: null,
      turnStartedAt: null,
      needsAnotherTurn: false,
    };
    if (args.runId) patch.agentRunId = args.runId;
    if (args.outcome === "answered") {
      patch.lastError = null;
      patch.lastErrorAt = null;
    } else {
      patch.lastError = args.detail ?? args.outcome;
      patch.lastErrorAt = now;
    }
    await ctx.db.update("Conversation", conversation.id, patch);

    const waiting = await ctx.db.query("Message", {
      conversationId: conversation.id,
      status: "received",
      $limit: 1,
    });
    if (conversation.needsAnotherTurn || waiting.length > 0) {
      await ctx.scheduler.runAfter(0, "processTurn", { conversationId: conversation.id });
    }
    return { ok: true };
  },
});
