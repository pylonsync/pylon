import { mutation, v } from "@pylonsync/functions";

/** Longer than a full batch can take: 5 sends × 20 s fetch timeout, plus settling. */
const LEASE_MS = 3 * 60 * 1000;
const BATCH = 5;

// Lease a conversation's queued Sendblue messages for deliverOutbound, oldest
// first. The lease carries a random token; settleOutbound accepts a result
// only with that token, so a job whose lease expired and was taken over
// cannot settle (or be followed by a resend of) a message it no longer holds.
// Internal.
export default mutation({
  internal: true,
  auth: "admin",
  args: { conversationId: v.string() },
  async handler(ctx, args) {
    const conversation = await ctx.db.get("Conversation", args.conversationId);
    if (!conversation) return { leaseToken: null, messages: [] };
    const now = new Date();
    const rows = await ctx.db.query("Message", {
      conversationId: args.conversationId,
      status: { $in: ["queued", "leased"] },
      $order: { createdAt: "asc" },
      $limit: 50,
    });
    const leaseToken = crypto.randomUUID();
    const leaseUntil = new Date(now.getTime() + LEASE_MS).toISOString();
    const messages: { id: string; handle: string; text: string }[] = [];
    for (const row of rows) {
      if (messages.length >= BATCH) break;
      if (row.direction !== "out" || row.transport !== "sendblue") continue;
      // A live lease belongs to another delivery job; stop so order holds.
      if (row.status === "leased" && String(row.leaseUntil ?? "") > now.toISOString()) break;
      await ctx.db.update("Message", String(row.id), {
        status: "leased",
        leaseUntil,
        leaseToken,
        attempts: (Number(row.attempts) || 0) + 1,
      });
      messages.push({ id: String(row.id), handle: String(conversation.handle), text: String(row.text) });
    }
    return { leaseToken: messages.length > 0 ? leaseToken : null, messages };
  },
});
