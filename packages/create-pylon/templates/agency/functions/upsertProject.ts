import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import { slugify, type ProjectRow } from "../lib/agency";

// upsertProject — owner-only. Create a new case study or edit an existing one
// (pass `id` to update). Projects are public-read, so the change shows up on the
// dashboard's live `db.useQuery("Project")` immediately, and on the marketing
// site on its next render. The slug is derived from the title (or an explicit
// slug) and made unique — a unique index backs it, so we resolve collisions
// here rather than letting the insert fail.
type Args = {
  id?: string;
  title: string;
  slug?: string;
  client?: string;
  summary?: string;
  year?: string;
  tags?: string;
  selected?: boolean;
  published?: boolean;
  order?: number;
  challenge?: string;
  approach?: string;
  outcome?: string;
  liveUrl?: string;
};

const clip = (s: string | undefined, max: number): string | null =>
  s != null && s.trim().length > 0 ? s.trim().slice(0, max) : null;

export default mutation<Args, { ok: boolean; id: string; slug: string }>({
  auth: "user",
  args: {
    id: v.optional(v.string()),
    title: v.string(),
    slug: v.optional(v.string()),
    client: v.optional(v.string()),
    summary: v.optional(v.string()),
    year: v.optional(v.string()),
    tags: v.optional(v.string()),
    selected: v.optional(v.boolean()),
    published: v.optional(v.boolean()),
    order: v.optional(v.int()),
    challenge: v.optional(v.string()),
    approach: v.optional(v.string()),
    outcome: v.optional(v.string()),
    liveUrl: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage projects.");
    }
    const title = args.title.trim();
    if (title.length < 1 || title.length > 120) {
      throw ctx.error("INVALID_ARGS", "A project title is required (up to 120 chars).");
    }

    await ctx.db.advisoryLock("agency_projects");
    const all = (await ctx.db.unsafe.list("Project")) as unknown as ProjectRow[];

    // Resolve a unique slug (skip the row being edited).
    const base = slugify(args.slug || title) || "project";
    let slug = base;
    let n = 2;
    while (all.some((p) => p.slug === slug && p.id !== args.id)) slug = `${base}-${n++}`;

    const patch = {
      title,
      slug,
      client: clip(args.client, 120) ?? "",
      summary: clip(args.summary, 300) ?? "",
      year: clip(args.year, 12),
      tags: clip(args.tags, 200),
      selected: args.selected === true,
      published: args.published !== false, // default published
      order: Math.trunc(args.order ?? 0),
      challenge: clip(args.challenge, 2000),
      approach: clip(args.approach, 2000),
      outcome: clip(args.outcome, 2000),
      liveUrl: clip(args.liveUrl, 400),
    };

    if (args.id) {
      const existing = all.find((p) => p.id === args.id);
      if (!existing) throw ctx.error("NOT_FOUND", "Project not found.");
      await ctx.db.unsafe.update("Project", args.id, patch);
      return { ok: true, id: args.id, slug };
    }

    const id = await ctx.db.unsafe.insert("Project", {
      ...patch,
      createdAt: new Date().toISOString(),
    });
    return { ok: true, id, slug };
  },
});
