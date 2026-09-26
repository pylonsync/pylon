import { mutation, v } from "@pylonsync/functions";
import {
  checkReminderTime,
  cleanStoredText,
  MAX_OPEN_REMINDERS_PER_CONTACT,
  MAX_REMINDER_CHARS,
  MAX_REMINDERS_PER_CONTACT_PER_DAY,
} from "../lib/reminders";

// set_reminder. Internal: contact and conversation come from resolveTurn. The
// reminder always goes to the contact being answered.
export default mutation({
  internal: true,
  auth: "admin",
  args: { contactId: v.string(), conversationId: v.string(), when: v.string(), text: v.string() },
  async handler(ctx, args) {
    const time = checkReminderTime(args.when, new Date());
    if (!time.ok) return { scheduled: false, error: time.error };
    const text = cleanStoredText(args.text, MAX_REMINDER_CHARS);
    if (text === "") return { scheduled: false, error: "The reminder text is empty." };
    const conversation = await ctx.db.get("Conversation", args.conversationId);
    if (!conversation || conversation.contactId !== args.contactId) {
      return { scheduled: false, error: "No conversation for this person." };
    }
    const open = await ctx.db.query("Reminder", {
      contactId: args.contactId,
      status: "scheduled",
      $limit: MAX_OPEN_REMINDERS_PER_CONTACT + 1,
    });
    if (open.length >= MAX_OPEN_REMINDERS_PER_CONTACT) {
      return { scheduled: false, error: "Too many upcoming reminders for this person." };
    }
    // A daily cap on created reminders, whatever their state, so fired
    // reminders do not free slots for an unbounded stream of texts.
    const since = new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString();
    const today = await ctx.db.query("Reminder", {
      contactId: args.contactId,
      createdAt: { $gte: since },
      $limit: MAX_REMINDERS_PER_CONTACT_PER_DAY + 1,
    });
    if (today.filter((r) => !r.demo).length >= MAX_REMINDERS_PER_CONTACT_PER_DAY) {
      return { scheduled: false, error: "This person has reached today's reminder limit." };
    }
    const id = await ctx.db.insert("Reminder", {
      contactId: args.contactId,
      conversationId: args.conversationId,
      text,
      dueAt: time.dueAt,
      status: "scheduled",
      createdAt: new Date().toISOString(),
    });
    await ctx.scheduler.runAt(time.dueMs, "fireReminder", { reminderId: id });
    return { scheduled: true, id, dueAt: time.dueAt };
  },
});
