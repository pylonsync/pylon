import { mutation } from "@pylonsync/functions";
import { DEMO_THREADS } from "../lib/demo";
import { ensureSettings, getOrCreateContact, getOrCreateConversation } from "../lib/store";

// Insert the sample threads (lib/demo.ts) once, on the owner's first visit to
// an empty dashboard. Owner only. A no-op after the first run, and a no-op if
// any real conversation exists.
export default mutation({
  auth: "admin",
  args: {},
  async handler(ctx) {
    const settings = await ensureSettings(ctx);
    if (settings.demoSeededAt) return { seeded: false };
    const existing = await ctx.db.query("Conversation", { $limit: 1 });
    await ctx.db.update("Settings", String(settings.id), { demoSeededAt: new Date().toISOString() });
    if (existing.length > 0) return { seeded: false };

    const now = Date.now();
    for (const thread of DEMO_THREADS) {
      const contact = await getOrCreateContact(ctx, thread.handle, {
        status: thread.status,
        displayName: thread.displayName,
        demo: true,
      });
      const conversation = await getOrCreateConversation(ctx, contact, true);
      let last = { at: "", text: "", direction: "in" };
      for (const m of thread.messages) {
        const at = new Date(now - m.ago * 60_000).toISOString();
        const inbound = m.from === "them";
        await ctx.db.insert("Message", {
          conversationId: conversation.id,
          contactId: contact.id,
          direction: inbound ? "in" : "out",
          kind: inbound ? "inbound" : "agent",
          text: m.text,
          status: inbound ? (thread.status === "allowed" ? "answered" : "ignored") : "sent",
          detail: inbound && thread.status !== "allowed" ? "unknown_sender" : null,
          transport: "demo",
          attempts: 0,
          sentAt: at,
          demo: true,
          createdAt: at,
        });
        last = { at, text: m.text, direction: inbound ? "in" : "out" };
      }
      await ctx.db.update("Conversation", conversation.id, {
        lastMessageAt: last.at,
        lastPreview: last.text,
        lastDirection: last.direction,
      });
      await ctx.db.update("Contact", contact.id, { lastMessageAt: last.at });
      for (const text of thread.notes ?? []) {
        await ctx.db.insert("Note", { contactId: contact.id, text, demo: true, createdAt: new Date(now).toISOString() });
      }
      if (thread.reminder) {
        await ctx.db.insert("Reminder", {
          contactId: contact.id,
          conversationId: conversation.id,
          text: thread.reminder.text,
          dueAt: new Date(now + thread.reminder.inMinutes * 60_000).toISOString(),
          // Sample reminders are never scheduled, so they can never send.
          status: "scheduled",
          demo: true,
          createdAt: new Date(now).toISOString(),
        });
      }
    }
    return { seeded: true };
  },
});
