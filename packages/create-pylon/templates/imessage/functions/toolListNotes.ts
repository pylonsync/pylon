import { query, v } from "@pylonsync/functions";

// recall_notes. Internal: the contact id comes from resolveTurn, never the model.
export default query({
  internal: true,
  auth: "admin",
  args: { contactId: v.string() },
  async handler(ctx, args) {
    const notes = await ctx.db.query("Note", {
      contactId: args.contactId,
      $order: { createdAt: "desc" },
      $limit: 50,
    });
    return notes.map((n) => ({ id: String(n.id), text: String(n.text), savedAt: String(n.createdAt) }));
  },
});
