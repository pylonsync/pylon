import { mutation, v } from "@pylonsync/functions";
import { buildTurnInput, MAX_RUN_MESSAGES } from "../lib/assistant";
import { loadSettings, toContact, toConversation, TURN_STALE_MS } from "../lib/store";

const MAX_TEXTS_PER_TURN = 20;
const CONTEXT_LINES = 12;

// Claim the next agent turn for a conversation, in one transaction:
//   - a turn already running → flag the conversation so that turn runs again
//     when it finishes, and return "busy"
//   - nothing waiting → "idle"
//   - otherwise mark the waiting texts "processing" under a new turn id and
//     return the agent input built from them.
// Internal: called by processTurn only.
export default mutation({
  internal: true,
  auth: "admin",
  args: { conversationId: v.string() },
  async handler(ctx, args) {
    const row = await ctx.db.get("Conversation", args.conversationId);
    if (!row) return { state: "idle" as const };
    const conversation = toConversation(row);
    const now = new Date();

    if (conversation.turnState === "running") {
      const started = Date.parse(conversation.turnStartedAt ?? "") || 0;
      if (now.getTime() - started < TURN_STALE_MS) {
        await ctx.db.update("Conversation", conversation.id, { needsAnotherTurn: true });
        return { state: "busy" as const };
      }
    }

    const pending = await ctx.db.query("Message", {
      conversationId: conversation.id,
      status: "received",
      $order: { createdAt: "asc" },
      $limit: MAX_TEXTS_PER_TURN,
    });
    if (pending.length === 0) {
      await ctx.db.update("Conversation", conversation.id, { turnState: "idle", needsAnotherTurn: false });
      return { state: "idle" as const };
    }

    const turnId = crypto.randomUUID();
    for (const m of pending) {
      await ctx.db.update("Message", String(m.id), { status: "processing", turnId });
    }
    await ctx.db.update("Conversation", conversation.id, {
      turnState: "running",
      turnId,
      turnStartedAt: now.toISOString(),
      needsAnotherTurn: false,
    });

    const contactRow = await ctx.db.get("Contact", conversation.contactId);
    const contact = contactRow ? toContact(contactRow) : null;
    const settings = await loadSettings(ctx);

    // One AgentRun holds the model transcript. When it grows past the cap, the
    // next turn starts a new run and carries recent texts as plain context.
    let runId = conversation.agentRunId;
    let context: string[] = [];
    if (runId) {
      const transcript = await ctx.db.query("AgentMessage", { runId, $limit: MAX_RUN_MESSAGES + 1 });
      if (transcript.length > MAX_RUN_MESSAGES) runId = null;
    }
    if (!runId) {
      const recent = await ctx.db.query("Message", {
        conversationId: conversation.id,
        $order: { createdAt: "desc" },
        $limit: CONTEXT_LINES + pending.length,
      });
      const pendingIds = new Set(pending.map((m) => String(m.id)));
      context = recent
        .filter((m) => !pendingIds.has(String(m.id)) && (m.direction === "out" || m.status === "answered"))
        .slice(0, CONTEXT_LINES)
        .reverse()
        .map((m) => `${m.direction === "in" ? "Them" : "You"}: ${String(m.text).replace(/\s+/g, " ").slice(0, 300)}`);
    }

    const input = buildTurnInput(
      {
        ownerName: settings.ownerName || null,
        contactName: contact?.displayName ?? null,
        handle: conversation.handle,
        timezone: settings.timezone,
        now,
      },
      pending.map((m) => String(m.text)),
      context,
    );

    return {
      state: "started" as const,
      turnId,
      conversationId: conversation.id,
      runId,
      input,
    };
  },
});
