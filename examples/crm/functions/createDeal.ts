import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    name: v.string(),
    companyId: v.optional(v.id("Company")),
    personId: v.optional(v.id("Person")),
    stage: v.optional(v.string()),
    amount: v.optional(v.number()),
    probability: v.optional(v.number()),
    closeDate: v.optional(v.string()),
    ownerId: v.optional(v.id("User")),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");
    if (!args.name.trim()) throw ctx.error("INVALID_NAME", "deal name required");

    const validStages = [
      "lead", "qualified", "proposal", "negotiation", "won", "lost",
    ];
    const stage = args.stage || "lead";
    if (!validStages.includes(stage))
      throw ctx.error("INVALID_STAGE", `stage ∈ ${validStages.join(", ")}`);

    if (args.companyId) {
      const c = await ctx.db.get("Company", args.companyId);
      if (!c || c.orgId !== ctx.auth.tenantId)
        throw ctx.error("COMPANY_NOT_FOUND", "company not in this org");
    }
    if (args.personId) {
      const p = await ctx.db.get("Person", args.personId);
      if (!p || p.orgId !== ctx.auth.tenantId)
        throw ctx.error("PERSON_NOT_FOUND", "person not in this org");
    }

    const now = new Date().toISOString();
    const id = await ctx.db.insert("Deal", {
      orgId: ctx.auth.tenantId,
      name: args.name.trim(),
      companyId: args.companyId || null,
      personId: args.personId || null,
      stage,
      amount: args.amount ?? 0,
      probability: args.probability ?? defaultProbability(stage),
      closeDate: args.closeDate || null,
      ownerId: args.ownerId || ctx.auth.userId,
      description: null,
      customFieldsJson: null,
      createdBy: ctx.auth.userId,
      createdAt: now,
      updatedAt: now,
      wonAt: stage === "won" ? now : null,
      lostAt: stage === "lost" ? now : null,
    });
    await ctx.db.insert("Activity", {
      orgId: ctx.auth.tenantId,
      targetType: "Deal",
      targetId: id,
      kind: "created",
      metaJson: JSON.stringify({ stage }),
      actorId: ctx.auth.userId,
      createdAt: now,
    });
    return { dealId: id };
  },
});

// Reasonable defaults per stage so users don't have to think about it.
function defaultProbability(stage: string): number {
  switch (stage) {
    case "lead": return 10;
    case "qualified": return 30;
    case "proposal": return 60;
    case "negotiation": return 80;
    case "won": return 100;
    case "lost": return 0;
    default: return 10;
  }
}
