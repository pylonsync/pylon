import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// declineInquiry — owner-only. Marks a lead "declined". If it had been "booked",
// it returns the project slot to the pool (Capacity.openSlots += 1), which syncs
// live so the hero counter ticks back up everywhere. Race-safe via the capacity
// advisory lock; idempotent on an already-declined lead.
export default mutation<{ inquiryId: string }, { ok: boolean; openSlots: number }>({
  auth: "user",
  args: { inquiryId: v.id("Inquiry") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage inquiries.");
    }

    const inquiry = (await ctx.db.get("Inquiry", args.inquiryId)) as
      | { status: string }
      | null;
    if (!inquiry) throw ctx.error("NOT_FOUND", "Inquiry not found.");

    await ctx.db.advisoryLock("agency_capacity");
    const cap = ((await ctx.db.unsafe.list("Capacity")) as unknown as {
      id: string;
      openSlots: number;
    }[])[0];
    let openSlots = cap?.openSlots ?? 0;

    // Releasing a previously-booked lead frees its slot again.
    if (inquiry.status === "booked" && cap) {
      openSlots = cap.openSlots + 1;
      await ctx.db.unsafe.update("Capacity", cap.id, {
        openSlots,
        updatedAt: new Date().toISOString(),
      });
    }
    await ctx.db.unsafe.update("Inquiry", args.inquiryId, { status: "declined" });

    return { ok: true, openSlots };
  },
});
