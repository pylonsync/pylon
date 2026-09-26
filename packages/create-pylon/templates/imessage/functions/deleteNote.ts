import { mutation, v } from "@pylonsync/functions";

// Remove a saved fact from the dashboard. Owner only.
export default mutation({
  auth: "admin",
  args: { noteId: v.string() },
  async handler(ctx, args) {
    const deleted = await ctx.db.delete("Note", args.noteId);
    return { deleted };
  },
});
