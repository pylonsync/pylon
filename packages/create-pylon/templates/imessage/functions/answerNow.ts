import { mutation, v } from "@pylonsync/functions";
import { toContact } from "../lib/store";

// Put a conversation's unanswered texts (waiting on setup, failed, or rate
// limited) back in the queue and start a turn. Owner only. The contact must
// be on the allowlist.
export default mutation({
  auth: "admin",
  args: { conversationId: v.string() },
  async handler(ctx, args) {
    const conversation = await ctx.db.get("Conversation", args.conversationId);
    if (!conversation) throw ctx.error("NOT_FOUND", "No such conversation");
    const contactRow = await ctx.db.get("Contact", String(conversation.contactId));
    const contact = contactRow ? toContact(contactRow) : null;
    if (!contact || contact.status !== "allowed") {
      throw ctx.error("NOT_ALLOWED", "Add this person to the allowlist first");
    }
    if (contact.optedOut) throw ctx.error("OPTED_OUT", "This person texted STOP");
    const rows = await ctx.db.query("Message", {
      conversationId: args.conversationId,
      status: { $in: ["awaiting_setup", "failed", "rate_limited"] },
      $order: { createdAt: "desc" },
      $limit: 20,
    });
    let requeued = 0;
    for (const row of rows) {
      if (row.direction !== "in") continue;
      await ctx.db.update("Message", String(row.id), { status: "received", detail: null, turnId: null });
      requeued += 1;
    }
    if (requeued > 0) await ctx.scheduler.runAfter(0, "processTurn", { conversationId: args.conversationId });
    return { requeued };
  },
});
