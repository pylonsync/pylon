import { mutation, type DbWriter } from "@pylonsync/functions";
import { guestHue, guestName } from "../lib/chat";
import { includeDemoConversation, seedChannels, seedMessages, seedPeople } from "../lib/seed";

/**
 * Called by the client once per page load, after the guest session exists.
 *
 * 1. On the first call against an empty database, creates the channels from
 *    lib/seed.ts and, when `includeDemoConversation` is on, the four demo
 *    people and their conversation.
 * 2. Creates the caller's Member row (a generated name like "Amber Otter" and
 *    an avatar hue) if it does not exist yet.
 * 3. Returns the server time, which the client uses as the starting "read up
 *    to" mark for channels it has not opened yet.
 *
 * Both steps are idempotent. The advisory lock keeps two first visitors from
 * seeding twice on Postgres; SQLite already serializes writers.
 *
 * Channel and Member deny client writes, so these writes use `ctx.db.unsafe`,
 * which keeps them working under PYLON_STRICT_FN_POLICIES. Every value comes
 * from lib/seed.ts or the caller's session, never from arguments.
 */
export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx): Promise<{ memberId: string; seeded: boolean; now: string }> {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "Start a guest session first.");
    await ctx.db.advisoryLock("chat_join");

    let seeded = false;
    const channels = await ctx.db.query("Channel", { $limit: 1 });
    if (channels.length === 0) {
      const channelIds = await seedChannelRows(ctx.db);
      if (includeDemoConversation) {
        // Demo messages carry the demo authors' ids, and field.owner() lets
        // only an admin context write an owner id that isn't the caller's.
        await ctx.auth.elevate({ admin: true, reason: "seed demo chat conversation" });
        await seedConversation(ctx.db, channelIds);
      }
      seeded = true;
    }

    const existing = await ctx.db.lookup("Member", "userId", userId);
    const now = new Date().toISOString();
    if (existing) return { memberId: existing.id as string, seeded, now };

    const memberId = await ctx.db.unsafe.insert("Member", {
      userId,
      name: guestName(userId),
      hue: guestHue(userId),
    });
    return { memberId, seeded, now };
  },
});

async function seedChannelRows(db: DbWriter): Promise<Map<string, string>> {
  const channelIds = new Map<string, string>();
  for (const [position, c] of seedChannels.entries()) {
    const id = await db.unsafe.insert("Channel", { ...c, position });
    channelIds.set(c.slug, id);
  }
  return channelIds;
}

async function seedConversation(db: DbWriter, channelIds: Map<string, string>) {
  for (const p of seedPeople) {
    await db.unsafe.insert("Member", { userId: p.userId, name: p.name, hue: p.hue });
  }
  const now = Date.now();
  for (const m of seedMessages) {
    const channelId = channelIds.get(m.channel);
    if (!channelId) continue;
    // Demo authors are not session users, so these rows set authorId and
    // createdAt directly.
    await db.insert("Message", {
      channelId,
      authorId: m.author,
      text: m.text,
      createdAt: new Date(now - m.minutesAgo * 60_000).toISOString(),
    });
  }
}
