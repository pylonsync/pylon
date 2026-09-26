import { mutation, v } from "@pylonsync/functions";
import { normalizeHandle } from "../lib/handles";
import { getOrCreateContact, getOrCreateConversation } from "../lib/store";

// Add a contact or update one: name and allowlist status. Owner only.
export default mutation({
  auth: "admin",
  args: {
    handle: v.string(),
    displayName: v.optional(v.string()),
    status: v.union(v.literal("allowed"), v.literal("blocked"), v.literal("unknown")),
  },
  async handler(ctx, args) {
    const normalized = normalizeHandle(args.handle);
    if (!normalized) {
      throw ctx.error("INVALID_HANDLE", "Enter a phone number like +1 512 555 0100 or an email address");
    }
    const name = (args.displayName ?? "").replace(/\s+/g, " ").trim().slice(0, 80);
    const contact = await getOrCreateContact(ctx, normalized.handle, { status: args.status });
    await ctx.db.update("Contact", contact.id, {
      status: args.status,
      ...(args.displayName !== undefined ? { displayName: name || null } : {}),
    });
    const conversation = await getOrCreateConversation(ctx, contact);
    return { contactId: contact.id, conversationId: conversation.id, handle: normalized.handle };
  },
});
