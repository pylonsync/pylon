import { mutation } from "@pylonsync/functions";

// Delete every sample row (demo: true). Owner only.
export default mutation({
  auth: "admin",
  args: {},
  async handler(ctx) {
    let removed = 0;
    for (const entity of ["Message", "Note", "Reminder", "Conversation", "Contact"]) {
      const rows = await ctx.db.query(entity, { demo: true });
      for (const row of rows) {
        await ctx.db.delete(entity, String(row.id));
        removed += 1;
      }
    }
    return { removed };
  },
});
