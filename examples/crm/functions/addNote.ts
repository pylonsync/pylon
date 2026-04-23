import { mutation, v } from "@pylonsync/functions";

/** Attach a freeform note to a Company / Person / Deal. Polymorphic via
 *  (targetType, targetId). Also writes an Activity row. */
export default mutation({
  args: {
    targetType: v.string(),
    targetId: v.string(),
    body: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");
    const body = args.body.trim();
    if (body.length === 0) throw ctx.error("EMPTY_NOTE", "note body required");
    if (body.length > 4000)
      throw ctx.error("NOTE_TOO_LONG", "notes capped at 4000 chars");

    const validTypes = ["Company", "Person", "Deal"];
    if (!validTypes.includes(args.targetType))
      throw ctx.error("INVALID_TYPE", `targetType ∈ ${validTypes.join(", ")}`);

    // Verify the target belongs to this org — prevents a client from
    // sneaking a note onto a record they can't see.
    const row = await ctx.db.get(args.targetType, args.targetId);
    if (!row || row.orgId !== ctx.auth.tenantId)
      throw ctx.error("TARGET_NOT_FOUND", "record not in this org");

    const now = new Date().toISOString();
    const id = await ctx.db.insert("Note", {
      orgId: ctx.auth.tenantId,
      targetType: args.targetType,
      targetId: args.targetId,
      body,
      authorId: ctx.auth.userId,
      createdAt: now,
    });
    await ctx.db.insert("Activity", {
      orgId: ctx.auth.tenantId,
      targetType: args.targetType,
      targetId: args.targetId,
      kind: "note_added",
      metaJson: JSON.stringify({ noteId: id, preview: body.slice(0, 80) }),
      actorId: ctx.auth.userId,
      createdAt: now,
    });
    return { noteId: id };
  },
});
