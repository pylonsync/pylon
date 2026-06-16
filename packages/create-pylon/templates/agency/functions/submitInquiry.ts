import { mutation, v } from "@pylonsync/functions";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

// submitInquiry — a prospect's "start a project" lead. A `mutation` (it writes
// the Inquiry row transactionally). `auth: "public"` — a prospect has no
// account.
//
// PRIVACY: it returns only `{ ok }`, never an Inquiry row or anyone's details.
// Submitting does NOT consume a capacity slot — a lead isn't a booking. The
// owner books it from the dashboard, which is what decrements the live counter.
export default mutation<
  {
    name: string;
    email: string;
    company?: string;
    projectType?: string;
    budget?: string;
    message?: string;
  },
  { ok: boolean }
>({
  auth: "public",
  args: {
    name: v.string(),
    email: v.string(),
    company: v.optional(v.string()),
    projectType: v.optional(v.string()),
    budget: v.optional(v.string()),
    message: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const name = args.name.trim();
    const email = args.email.trim().toLowerCase();
    if (name.length < 1 || name.length > 120) {
      throw ctx.error("INVALID_ARGS", "Please enter your name.");
    }
    if (!EMAIL_RE.test(email) || email.length > 254) {
      throw ctx.error("INVALID_ARGS", "Please enter a valid email address.");
    }
    const clip = (s: string | undefined, max: number) => (s ? s.trim().slice(0, max) : null);

    await ctx.db.unsafe.insert("Inquiry", {
      name,
      email,
      company: clip(args.company, 160),
      projectType: clip(args.projectType, 80),
      budget: clip(args.budget, 80),
      message: clip(args.message, 4000),
      status: "new",
      createdAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
