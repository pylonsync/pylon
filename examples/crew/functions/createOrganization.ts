import { mutation, v } from "@pylonsync/functions";

// Seeds a new workspace with three starter agents and a two-step pipeline so
// the app is usable the moment the workspace is created. Without this, a new
// tenant lands on an empty Agents page and has to fill out forms before any
// part of the UI does anything interesting.
const STARTER_AGENTS = [
  {
    name: "Researcher",
    role: "Gathers background, facts, and links.",
    avatarEmoji: "\u{1F50D}",
    model: "claude-sonnet-4-6",
    systemPrompt:
      "You are a diligent research assistant. Given a topic, produce a concise brief with key facts, open questions, and 3–5 credible sources.",
  },
  {
    name: "Copywriter",
    role: "Turns briefs into sharp prose.",
    avatarEmoji: "\u{270D}\u{FE0F}",
    model: "claude-sonnet-4-6",
    systemPrompt:
      "You are a punchy B2B copywriter. Given a research brief, write an outline and a ~200-word draft in a clear, confident voice.",
  },
  {
    name: "Reviewer",
    role: "Sharpens drafts and flags risks.",
    avatarEmoji: "\u{1F9D0}",
    model: "claude-haiku-4-5",
    systemPrompt:
      "You are a thoughtful editor. Given a draft, critique structure and clarity, suggest concrete edits, and flag anything factually dubious.",
  },
];

export default mutation({
  args: { name: v.string(), slug: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name required");
    const slug = args.slug.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{1,49}$/.test(slug))
      throw ctx.error("INVALID_SLUG", "lowercase letters/numbers/dashes, 2–50");

    const now = new Date().toISOString();
    try {
      const orgId = await ctx.db.insert("Organization", {
        name,
        slug,
        createdBy: ctx.auth.userId,
        createdAt: now,
      });
      await ctx.db.insert("OrgMember", {
        userId: ctx.auth.userId,
        orgId,
        role: "owner",
        joinedAt: now,
      });

      const agentIds: Record<string, string> = {};
      for (const a of STARTER_AGENTS) {
        agentIds[a.name] = await ctx.db.insert("Agent", {
          orgId,
          name: a.name,
          role: a.role,
          systemPrompt: a.systemPrompt,
          model: a.model,
          avatarEmoji: a.avatarEmoji,
          tools: null,
          createdBy: ctx.auth.userId,
          createdAt: now,
        });
      }

      // Seed a two-step pipeline: Researcher → Copywriter. The Reviewer is
      // left as a standalone agent so users can see both run shapes.
      const pipelineId = await ctx.db.insert("Pipeline", {
        orgId,
        name: "Brief & Draft",
        description: "Research a topic, then write a short draft.",
        createdBy: ctx.auth.userId,
        createdAt: now,
      });
      await ctx.db.insert("PipelineStep", {
        orgId,
        pipelineId,
        position: 0,
        agentId: agentIds["Researcher"],
        instruction: "Research the following and produce a brief: {{input}}",
      });
      await ctx.db.insert("PipelineStep", {
        orgId,
        pipelineId,
        position: 1,
        agentId: agentIds["Copywriter"],
        instruction:
          "Using this brief, write a short draft:\n\n{{previous}}\n\nOriginal request: {{input}}",
      });

      return { orgId, pipelineId };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg))
        throw ctx.error("SLUG_TAKEN", `slug "${slug}" already taken`);
      throw e;
    }
  },
});
