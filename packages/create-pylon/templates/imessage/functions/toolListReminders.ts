import { query, v } from "@pylonsync/functions";

// list_reminders. Internal: the contact id comes from resolveTurn.
export default query({
  internal: true,
  auth: "admin",
  args: { contactId: v.string() },
  async handler(ctx, args) {
    const rows = await ctx.db.query("Reminder", {
      contactId: args.contactId,
      status: "scheduled",
      $order: { dueAt: "asc" },
      $limit: 50,
    });
    return rows.map((r) => ({ id: String(r.id), dueAt: String(r.dueAt), text: String(r.text) }));
  },
});
