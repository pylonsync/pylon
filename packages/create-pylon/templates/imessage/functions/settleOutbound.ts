import { mutation, v } from "@pylonsync/functions";
import { touchTransport } from "../lib/store";

// Record the result of one Sendblue send. Internal: deliverOutbound only.
export default mutation({
  internal: true,
  auth: "admin",
  args: {
    messageId: v.string(),
    leaseToken: v.string(),
    status: v.union(v.literal("sent"), v.literal("dry_run"), v.literal("failed")),
    externalId: v.optional(v.string()),
    error: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const row = await ctx.db.get("Message", args.messageId);
    if (!row || row.status !== "leased" || row.leaseToken !== args.leaseToken) return { ok: false };
    const now = new Date().toISOString();
    await ctx.db.update("Message", args.messageId, {
      status: args.status,
      leaseUntil: null,
      leaseToken: null,
      sentAt: args.status === "failed" ? null : now,
      detail: args.error ?? null,
      // The provider's id for the sent message. Unique like inbound ids.
      ...(args.externalId ? { externalId: args.externalId } : {}),
    });
    await touchTransport(
      ctx,
      "sendblue",
      args.status === "failed" ? { lastError: args.error ?? "Send failed", lastErrorAt: now } : { lastOutboundAt: now },
    );
    return { ok: true };
  },
});
