import { mutation, v } from "@pylonsync/functions";

// Cancel an upcoming reminder from the dashboard. Owner only.
export default mutation({
  auth: "admin",
  args: { reminderId: v.string() },
  async handler(ctx, args) {
    const reminder = await ctx.db.get("Reminder", args.reminderId);
    if (!reminder || reminder.status !== "scheduled") throw ctx.error("NOT_FOUND", "No upcoming reminder with that id");
    await ctx.db.update("Reminder", args.reminderId, { status: "cancelled", detail: "Cancelled by the owner" });
    return { ok: true };
  },
});
