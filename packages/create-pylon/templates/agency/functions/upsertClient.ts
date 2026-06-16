import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { ClientRow } from "../lib/agency";

// upsertClient — owner-only. Create a CRM contact or edit one (pass `id`). When
// a client's name or company changes, any invoice that referenced it keeps its
// denormalized `clientName` from when it was issued — invoices are a financial
// record and shouldn't silently rewrite themselves; the link by `clientId`
// still resolves for navigation.
type Args = {
  id?: string;
  name: string;
  company?: string;
  email?: string;
  phone?: string;
  status?: string;
  notes?: string;
};

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const STATUSES = new Set(["prospect", "active", "past"]);
const clip = (s: string | undefined, max: number): string | null =>
  s != null && s.trim().length > 0 ? s.trim().slice(0, max) : null;

export default mutation<Args, { ok: boolean; id: string }>({
  auth: "user",
  args: {
    id: v.optional(v.string()),
    name: v.string(),
    company: v.optional(v.string()),
    email: v.optional(v.string()),
    phone: v.optional(v.string()),
    status: v.optional(v.string()),
    notes: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage clients.");
    }
    const name = args.name.trim();
    if (name.length < 1 || name.length > 120) {
      throw ctx.error("INVALID_ARGS", "A client name is required (up to 120 chars).");
    }
    const email = clip(args.email, 254);
    if (email && !EMAIL_RE.test(email)) {
      throw ctx.error("INVALID_ARGS", "Enter a valid email address.");
    }
    const status = STATUSES.has((args.status ?? "").trim()) ? args.status!.trim() : "prospect";

    const patch = {
      name,
      company: clip(args.company, 160),
      email,
      phone: clip(args.phone, 40),
      status,
      notes: clip(args.notes, 2000),
    };

    if (args.id) {
      const existing = (await ctx.db.get("Client", args.id)) as ClientRow | null;
      if (!existing) throw ctx.error("NOT_FOUND", "Client not found.");
      await ctx.db.unsafe.update("Client", args.id, patch);
      return { ok: true, id: args.id };
    }

    const id = await ctx.db.unsafe.insert("Client", {
      ...patch,
      createdAt: new Date().toISOString(),
    });
    return { ok: true, id };
  },
});
