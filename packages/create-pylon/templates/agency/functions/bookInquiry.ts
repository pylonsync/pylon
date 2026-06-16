import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// bookInquiry — owner-only. Marks a lead "booked" AND consumes one project slot:
// it decrements Capacity.openSlots, which syncs to every open landing page, so
// the hero's "N slots open" counter ticks down live for everyone. Taking a
// per-row advisory lock on the capacity row makes the read-then-decrement
// race-safe. Idempotent: booking an already-booked lead doesn't double-count.
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

    // Only decrement when this lead is newly booked — re-booking is a no-op.
    if (inquiry.status !== "booked" && cap) {
      openSlots = Math.max(0, cap.openSlots - 1);
      await ctx.db.unsafe.update("Capacity", cap.id, {
        openSlots,
        updatedAt: new Date().toISOString(),
      });
    }
    await ctx.db.unsafe.update("Inquiry", args.inquiryId, { status: "booked" });

    return { ok: true, openSlots };
  },
});
