import { mutation, v } from "@pylonsync/functions";

/**
 * Edit a channel. Only the creator can rename / change topic / flip privacy.
 * Name changes go through the same regex validation as createChannel so you
 * can't backdoor an invalid name. Privacy changes are write-only — we don't
 * retroactively hide past messages from non-members when flipping to private
 * (that's a pure read-policy story; membership in the channel is the gate).
 */
export default mutation({
  args: {
    channelId: v.id("Channel"),
    name: v.optional(v.string()),
    topic: v.optional(v.string()),
    isPrivate: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const channel = await ctx.db.get("Channel", args.channelId);
    if (!channel) throw ctx.error("CHANNEL_NOT_FOUND", "channel does not exist");

    // DM channels aren't editable — their name encodes the participant pair
    // and topic isn't surfaced in the DM UI.
    if (typeof channel.name === "string" && channel.name.startsWith("dm:")) {
      throw ctx.error("DM_NOT_EDITABLE", "direct messages can't be edited");
    }

    if (channel.createdBy !== ctx.auth.userId) {
      throw ctx.error(
        "FORBIDDEN",
        "only the channel creator can edit this channel",
      );
    }

    const patch: Record<string, unknown> = {};

    if (args.name !== undefined) {
      const cleaned = args.name.trim().toLowerCase();
      if (!/^[a-z0-9][a-z0-9-]{0,79}$/.test(cleaned)) {
        throw ctx.error(
          "INVALID_NAME",
          "channel name must be lowercase alphanumeric + dashes, max 80 chars",
        );
      }
      if (cleaned !== channel.name) patch.name = cleaned;
    }

    if (args.topic !== undefined) {
      const topic = args.topic.slice(0, 250);
      if (topic !== (channel.topic ?? "")) patch.topic = topic;
    }

    if (args.isPrivate !== undefined && args.isPrivate !== channel.isPrivate) {
      patch.isPrivate = args.isPrivate;
    }

    if (Object.keys(patch).length === 0) {
      return { channelId: args.channelId, changed: false };
    }

    await ctx.db.update("Channel", args.channelId, patch);
    return { channelId: args.channelId, changed: true };
  },
});
