import { mutation, v } from "@pylonsync/functions";

// forget_note. Internal. The note must belong to the current contact, so a
// note id guessed or copied from elsewhere does nothing.
export default mutation({
  internal: true,
  auth: "admin",
  args: { contactId: v.string(), noteId: v.string() },
  async handler(ctx, args) {
    const note = await ctx.db.get("Note", args.noteId);
    if (!note || note.contactId !== args.contactId) return { deleted: false, error: "No such note." };
    await ctx.db.delete("Note", args.noteId);
    return { deleted: true };
  },
});
