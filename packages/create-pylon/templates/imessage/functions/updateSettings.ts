import { mutation, v } from "@pylonsync/functions";
import { ensureSettings } from "../lib/store";

function validTimezone(tz: string): boolean {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: tz });
    return true;
  } catch {
    return false;
  }
}

// Owner settings. Owner only.
export default mutation({
  auth: "admin",
  args: {
    ownerName: v.optional(v.string()),
    timezone: v.optional(v.string()),
    unknownSenderPolicy: v.optional(v.union(v.literal("ignore"), v.literal("reply_once"))),
    unknownSenderReply: v.optional(v.string()),
    rateLimitPerHour: v.optional(v.int()),
    paused: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    const settings = await ensureSettings(ctx);
    const patch: Record<string, unknown> = { updatedAt: new Date().toISOString() };
    if (args.ownerName !== undefined) patch.ownerName = args.ownerName.trim().slice(0, 80);
    if (args.timezone !== undefined) {
      if (!validTimezone(args.timezone)) throw ctx.error("INVALID_ARGS", "Unknown timezone");
      patch.timezone = args.timezone;
    }
    if (args.unknownSenderPolicy !== undefined) patch.unknownSenderPolicy = args.unknownSenderPolicy;
    if (args.unknownSenderReply !== undefined) {
      const reply = args.unknownSenderReply.trim().slice(0, 320);
      if (reply === "") throw ctx.error("INVALID_ARGS", "The reply to unknown senders cannot be empty");
      patch.unknownSenderReply = reply;
    }
    if (args.rateLimitPerHour !== undefined) {
      if (args.rateLimitPerHour < 1 || args.rateLimitPerHour > 500) {
        throw ctx.error("INVALID_ARGS", "The hourly limit must be between 1 and 500");
      }
      patch.rateLimitPerHour = args.rateLimitPerHour;
    }
    if (args.paused !== undefined) patch.paused = args.paused;
    await ctx.db.update("Settings", String(settings.id), patch);
    return { ok: true };
  },
});
