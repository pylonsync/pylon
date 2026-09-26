import { mutation, v } from "@pylonsync/functions";
import { cleanStoredText, MAX_NOTE_CHARS, MAX_NOTES_PER_CONTACT } from "../lib/reminders";

// remember_note. Internal: the contact id comes from resolveTurn, never the model.
export default mutation({
  internal: true,
  auth: "admin",
  args: { contactId: v.string(), text: v.string() },
  async handler(ctx, args) {
    const text = cleanStoredText(args.text, MAX_NOTE_CHARS);
    if (text === "") return { saved: false, error: "The note is empty." };
    const existing = await ctx.db.query("Note", { contactId: args.contactId, $limit: MAX_NOTES_PER_CONTACT + 1 });
    if (existing.length >= MAX_NOTES_PER_CONTACT) {
      return { saved: false, error: "Too many notes for this person. Forget an old one first." };
    }
    if (existing.some((n) => String(n.text).toLowerCase() === text.toLowerCase())) {
      return { saved: true, duplicate: true };
    }
    const id = await ctx.db.insert("Note", { contactId: args.contactId, text, createdAt: new Date().toISOString() });
    return { saved: true, id };
  },
});
