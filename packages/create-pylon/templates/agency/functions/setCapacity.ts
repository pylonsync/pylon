import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// setCapacity — owner-only. Set the booking window label and how many project
// slots are open (e.g. at the start of a new quarter). The update syncs to
// every open landing page, so the hero counter reflects it live. Creates the
// row if it doesn't exist yet.
export default mutation<
  { label: string; openSlots: number },
  { ok: boolean; openSlots: number }
>({
  auth: "user",
  args: { label: v.string(), openSlots: v.int() },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage capacity.");
    }
    const label = args.label.trim().slice(0, 60);
    const openSlots = Math.max(0, Math.trunc(args.openSlots));

    await ctx.db.advisoryLock("agency_capacity");
    const cap = ((await ctx.db.unsafe.list("Capacity")) as unknown as { id: string }[])[0];
    const now = new Date().toISOString();
    if (cap) {
      await ctx.db.unsafe.update("Capacity", cap.id, { label, openSlots, updatedAt: now });
    } else {
      await ctx.db.unsafe.insert("Capacity", { label, openSlots, updatedAt: now });
    }
    return { ok: true, openSlots };
  },
});
