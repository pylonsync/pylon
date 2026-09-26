import { mutation, v } from "@pylonsync/functions";
import { loadSettings, queueOutbound, toContact } from "../lib/store";

// Send a due reminder. Scheduled with ctx.scheduler.runAt by toolSetReminder;
// internal. A cancelled or already-sent reminder is a no-op, so a job retry
// never texts twice.
export default mutation({
  internal: true,
  auth: "admin",
  args: { reminderId: v.string() },
  async handler(ctx, args) {
    const reminder = await ctx.db.get("Reminder", args.reminderId);
    if (!reminder || reminder.status !== "scheduled") return { sent: false };
    const contactRow = await ctx.db.get("Contact", String(reminder.contactId));
    const conversation = await ctx.db.get("Conversation", String(reminder.conversationId));
    const contact = contactRow ? toContact(contactRow) : null;
    if ((await loadSettings(ctx)).paused) {
      await ctx.db.update("Reminder", args.reminderId, { status: "cancelled", detail: "The assistant was paused" });
      return { sent: false };
    }
    if (!contact || !conversation || contact.status !== "allowed" || contact.optedOut) {
      await ctx.db.update("Reminder", args.reminderId, {
        status: "cancelled",
        detail: "The contact is no longer on the allowlist",
      });
      return { sent: false };
    }
    const messageId = await queueOutbound(ctx, {
      conversationId: String(conversation.id),
      contactId: contact.id,
      handle: contact.handle,
      text: String(reminder.text),
      kind: "reminder",
    });
    await ctx.db.update("Reminder", args.reminderId, {
      status: "sent",
      sentAt: new Date().toISOString(),
      detail: null,
    });
    return { sent: true, messageId };
  },
});
