import { mutation } from "@pylonsync/functions";
import { exampleConversations } from "../lib/examples";
import { providerStatus } from "../lib/provider";

// Write the example conversations from lib/examples.ts into the caller's
// history. Runs only while no provider key is configured and only for a
// caller with no conversations, so it never adds to a real history. Each row
// is marked `example: true`; the thread labels it and keeps it read-only.
export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    const userId = ctx.auth.userId;
    if (!userId || providerStatus(ctx.env).configured) return { seeded: 0 };

    await ctx.db.advisoryLock(`ai-chat:seed:${userId}`);
    const existing = (await ctx.db.unsafe.query("Conversation", { userId, $limit: 1 })) as unknown[];
    if (existing.length > 0) return { seeded: 0 };

    const now = Date.now();
    // Conversation and Message inserts are denied to clients by policy; these
    // rows belong to the caller.
    for (const ex of exampleConversations) {
      const start = now - ex.hoursAgo * 60 * 60 * 1000;
      const last = new Date(start + (ex.messages.length - 1) * 60_000).toISOString();
      const conversationId = await ctx.db.unsafe.insert("Conversation", {
        userId,
        title: ex.title,
        example: true,
        createdAt: new Date(start).toISOString(),
        updatedAt: last,
      });
      for (const [i, m] of ex.messages.entries()) {
        await ctx.db.unsafe.insert("Message", {
          conversationId,
          userId,
          role: m.role,
          content: m.content,
          status: "done",
          createdAt: new Date(start + i * 60_000).toISOString(),
        });
      }
    }
    return { seeded: exampleConversations.length };
  },
});
