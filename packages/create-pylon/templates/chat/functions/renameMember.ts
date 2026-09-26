import { mutation, v } from "@pylonsync/functions";
import { cleanName, MAX_NAME_LENGTH } from "../lib/chat";
import { seedPeople } from "../lib/seed";

/** Changes the caller's display name. Other people see it on every message at once. */
export default mutation({
  auth: "guest",
  args: { name: v.string() },
  async handler(ctx, { name }) {
    const clean = cleanName(name);
    if (!clean) {
      throw ctx.error("INVALID_NAME", `Use 1 to ${MAX_NAME_LENGTH} characters.`);
    }
    const taken = seedPeople.some((p) => p.name.toLowerCase() === clean.toLowerCase());
    if (taken) throw ctx.error("NAME_TAKEN", "Someone in this workspace already uses that name.");
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "Start a guest session first.");
    const member = await ctx.db.lookup("Member", "userId", userId);
    if (!member) throw ctx.error("NOT_JOINED", "Call joinChat first.");
    // Member denies client writes; `unsafe` keeps this working under
    // PYLON_STRICT_FN_POLICIES. The row is the caller's own, found by session id.
    await ctx.db.unsafe.update("Member", member.id as string, { name: clean });
    return { name: clean };
  },
});
