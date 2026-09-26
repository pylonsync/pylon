import { mutation, v } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";
import {
  buildHistory,
  isStaleStream,
  STALE_STREAM_MS,
  RATE_WINDOW_MS,
  replyLimit,
  sortMessages,
  type LlmTurn,
  type MessageRow,
} from "../lib/chat";

// Internal: the write half of starting a reply, called by `respond`. In one
// transaction it checks ownership, the hourly limits, and that the caller has
// no other generation running; stores the user's message (or, on
// regenerate, removes the last reply); and inserts the assistant message the
// stream fills. Returns the transcript to send to the model.
export default mutation({
  internal: true,
  auth: "guest",
  args: {
    conversationId: v.string(),
    content: v.optional(v.string()),
    regenerate: v.optional(v.boolean()),
    model: v.string(),
    streamId: v.optional(v.string()),
  },
  async handler(ctx, args): Promise<{ messageId: string; history: LlmTurn[] }> {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("UNAUTHENTICATED", "A session is required.");

    const convo = (await ctx.db.unsafe.get("Conversation", args.conversationId)) as
      | { userId: string; example?: boolean }
      | null;
    if (!convo || convo.userId !== userId) {
      throw ctx.error("NOT_FOUND", "Conversation not found.");
    }
    if (convo.example) {
      throw ctx.error("EXAMPLE_READ_ONLY", "Example conversations are read-only. Start a new chat.");
    }

    // Serializes turn starts, so two tabs (or two guest sessions) cannot both
    // pass the limit and busy checks below at the same moment.
    await ctx.db.advisoryLock(ctx.auth.isGuest ? "ai-chat:turn:guests" : `ai-chat:turn:${userId}`);

    // Limits and the busy check read ReplyLog, which regenerate and delete
    // never remove.
    const now = Date.now();
    const since = new Date(now - RATE_WINDOW_MS).toISOString();
    const mine = (await ctx.db.unsafe.query("ReplyLog", {
      userId,
      createdAt: { $gt: since },
    })) as { createdAt: string; finishedAt?: string | null }[];
    if (mine.length >= replyLimit(ctx.auth.isGuest)) {
      throw ctx.error(
        "RATE_LIMITED",
        ctx.auth.isGuest
          ? "Hourly limit reached. Sign in for a higher limit, or try again later."
          : "Hourly limit reached. Try again later.",
      );
    }
    if (ctx.auth.isGuest) {
      const guests = (await ctx.db.unsafe.query("ReplyLog", {
        isGuest: true,
        createdAt: { $gt: since },
        $limit: siteConfig.chat.repliesPerHour.allGuests,
      })) as unknown[];
      if (guests.length >= siteConfig.chat.repliesPerHour.allGuests) {
        throw ctx.error("RATE_LIMITED", "Guest replies are busy right now. Sign in, or try again later.");
      }
    }
    // One generation per person at a time. A stopped reply still counts until
    // its provider call ends, because the call keeps running on the server.
    if (mine.some((r) => !r.finishedAt && now - Date.parse(r.createdAt) < STALE_STREAM_MS)) {
      throw ctx.error("BUSY", "A reply is already being written. Wait for it to finish.");
    }

    const rows = sortMessages(
      ((await ctx.db.unsafe.query("Message", { conversationId: args.conversationId })) as unknown as MessageRow[]).filter(
        (m) => m.userId === userId,
      ),
    );
    if (rows.some((m) => m.status === "streaming" && !isStaleStream(m, now))) {
      throw ctx.error("BUSY", "A reply is already being written in this conversation.");
    }

    // Message writes are denied to clients by policy. Every write below is
    // scoped to the conversation the caller was just checked to own.
    let transcript = rows;
    if (args.regenerate) {
      const last = rows[rows.length - 1];
      if (last?.role === "assistant") {
        await ctx.db.unsafe.delete("Message", last.id);
        transcript = rows.slice(0, -1);
      }
      if (transcript[transcript.length - 1]?.role !== "user") {
        throw ctx.error("INVALID_ARGS", "There is no message to answer.");
      }
    } else {
      const text = (args.content ?? "").trim();
      if (text.length === 0) throw ctx.error("INVALID_ARGS", "The message is empty.");
      if (text.length > siteConfig.chat.maxInputChars) {
        throw ctx.error("INVALID_ARGS", "The message is too long.");
      }
      const createdAt = new Date(now).toISOString();
      const id = await ctx.db.unsafe.insert("Message", {
        conversationId: args.conversationId,
        userId,
        role: "user",
        content: text,
        status: "done",
        createdAt,
      });
      transcript = [
        ...rows,
        { id, conversationId: args.conversationId, userId, role: "user", content: text, status: "done", createdAt },
      ];
    }

    // One millisecond after the user's message, so the pair sorts in order.
    const messageId = await ctx.db.unsafe.insert("Message", {
      conversationId: args.conversationId,
      userId,
      role: "assistant",
      content: "",
      status: "streaming",
      streamId: args.streamId ?? null,
      model: args.model,
      createdAt: new Date(now + 1).toISOString(),
    });
    await ctx.db.unsafe.insert("ReplyLog", {
      userId,
      messageId,
      isGuest: ctx.auth.isGuest,
      model: args.model,
      createdAt: new Date(now).toISOString(),
    });
    await ctx.db.unsafe.update("Conversation", args.conversationId, {
      updatedAt: new Date(now).toISOString(),
    });

    return { messageId, history: buildHistory(transcript) };
  },
});
