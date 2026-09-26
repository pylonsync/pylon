import { mutation, v } from "@pylonsync/functions";
import { queueOutbound, toContact } from "../lib/store";

// The owner types a message into a thread and it goes out through the
// active transport. Owner only. Refused for contacts who texted STOP.
export default mutation({
  auth: "admin",
  args: { conversationId: v.string(), text: v.string() },
  async handler(ctx, args) {
    const text = args.text.trim();
    if (text === "") throw ctx.error("INVALID_ARGS", "Type a message first");
    if (text.length > 4000) throw ctx.error("INVALID_ARGS", "Keep it under 4,000 characters");
    const conversation = await ctx.db.get("Conversation", args.conversationId);
    if (!conversation) throw ctx.error("NOT_FOUND", "No such conversation");
    const contactRow = await ctx.db.get("Contact", String(conversation.contactId));
    if (!contactRow) throw ctx.error("NOT_FOUND", "No such contact");
    const contact = toContact(contactRow);
    if (contact.optedOut) throw ctx.error("OPTED_OUT", "This person texted STOP. They have to text START first.");
    const id = await queueOutbound(ctx, {
      conversationId: String(conversation.id),
      contactId: contact.id,
      handle: contact.handle,
      text,
      kind: "owner",
    });
    return { messageId: id };
  },
});
