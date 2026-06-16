import { mutation, v } from "@pylonsync/functions";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const URL_RE = /^https?:\/\/.+/i;

// submitListing — a visitor proposes a new directory entry. A `mutation` (it
// writes the Submission transactionally). `auth: "public"` — submitters have no
// account.
//
// PRIVACY: returns only `{ ok }`, never a Submission row. The entry is NOT
// public yet — it's written as a pending Submission (deny-all) for the owner to
// review; approveSubmission is what turns it into a public Listing.
export default mutation<
  {
    submitterName: string;
    submitterEmail: string;
    name: string;
    tagline: string;
    url: string;
    category: string;
    tags?: string;
    description?: string;
  },
  { ok: boolean }
>({
  auth: "public",
  args: {
    submitterName: v.string(),
    submitterEmail: v.string(),
    name: v.string(),
    tagline: v.string(),
    url: v.string(),
    category: v.string(),
    tags: v.optional(v.string()),
    description: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const submitterName = args.submitterName.trim();
    const submitterEmail = args.submitterEmail.trim().toLowerCase();
    const name = args.name.trim();
    const url = args.url.trim();
    if (!submitterName) throw ctx.error("INVALID_ARGS", "Your name is required.");
    if (!EMAIL_RE.test(submitterEmail)) throw ctx.error("INVALID_ARGS", "Enter a valid email.");
    if (!name) throw ctx.error("INVALID_ARGS", "The listing name is required.");
    if (!URL_RE.test(url)) throw ctx.error("INVALID_ARGS", "Enter a valid URL (https://…).");

    const clip = (s: string | undefined, n: number) => (s ? s.trim().slice(0, n) : null);

    await ctx.db.unsafe.insert("Submission", {
      submitterName: submitterName.slice(0, 120),
      submitterEmail: submitterEmail.slice(0, 254),
      name: name.slice(0, 120),
      tagline: clip(args.tagline, 160) ?? "",
      url: url.slice(0, 500),
      category: clip(args.category, 60) ?? "",
      tags: clip(args.tags, 160),
      description: clip(args.description, 2000),
      status: "new",
      createdAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
