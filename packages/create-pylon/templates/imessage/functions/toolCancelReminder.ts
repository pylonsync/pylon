import { mutation, v } from "@pylonsync/functions";

// cancel_reminder. Internal. The reminder must belong to the current contact.
// The scheduled job still fires and finds it cancelled.
export default mutation({
  internal: true,
  auth: "admin",
  args: { contactId: v.string(), reminderId: v.string() },
  async handler(ctx, args) {
    const reminder = await ctx.db.get("Reminder", args.reminderId);
    if (!reminder || reminder.contactId !== args.contactId || reminder.status !== "scheduled") {
      return { cancelled: false, error: "No such upcoming reminder." };
    }
    await ctx.db.update("Reminder", args.reminderId, { status: "cancelled", detail: "Cancelled in conversation" });
    return { cancelled: true };
  },
});
