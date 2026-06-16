import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { ClientRow, ProjectRow } from "../lib/agency";

// upsertInvoice — owner-only. Create a bill or edit one (pass `id`). The client
// is referenced by id and must exist; we denormalize its name onto the invoice
// (`clientName`) so the list renders without a join, and so the bill keeps the
// name it was issued under even if the contact is renamed later. An optional
// project link works the same way (`projectTitle`). Amount is integer cents.
type Args = {
  id?: string;
  number: string;
  clientId: string;
  projectId?: string;
  amountCents: number;
  status?: string;
  issuedAt?: string;
  dueAt?: string;
  notes?: string;
};

const STATUSES = new Set(["draft", "sent", "paid", "overdue"]);
const clip = (s: string | undefined, max: number): string | null =>
  s != null && s.trim().length > 0 ? s.trim().slice(0, max) : null;

export default mutation<Args, { ok: boolean; id: string }>({
  auth: "user",
  args: {
    id: v.optional(v.string()),
    number: v.string(),
    clientId: v.string(),
    projectId: v.optional(v.string()),
    amountCents: v.int(),
    status: v.optional(v.string()),
    issuedAt: v.optional(v.string()),
    dueAt: v.optional(v.string()),
    notes: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage invoices.");
    }
    const number = args.number.trim();
    if (number.length < 1 || number.length > 40) {
      throw ctx.error("INVALID_ARGS", "An invoice number is required.");
    }

    const client = (await ctx.db.get("Client", args.clientId)) as ClientRow | null;
    if (!client) throw ctx.error("INVALID_ARGS", "Pick a client for this invoice.");

    let projectId: string | null = null;
    let projectTitle: string | null = null;
    if (args.projectId) {
      const project = (await ctx.db.get("Project", args.projectId)) as ProjectRow | null;
      if (project) {
        projectId = project.id;
        projectTitle = project.title;
      }
    }

    const amountCents = Math.max(0, Math.trunc(args.amountCents || 0));
    const status = STATUSES.has((args.status ?? "").trim()) ? args.status!.trim() : "draft";

    const patch = {
      number,
      clientId: client.id,
      clientName: client.name,
      projectId,
      projectTitle,
      amountCents,
      status,
      issuedAt: clip(args.issuedAt, 20),
      dueAt: clip(args.dueAt, 20),
      notes: clip(args.notes, 2000),
    };

    if (args.id) {
      const existing = await ctx.db.get("Invoice", args.id);
      if (!existing) throw ctx.error("NOT_FOUND", "Invoice not found.");
      await ctx.db.unsafe.update("Invoice", args.id, patch);
      return { ok: true, id: args.id };
    }

    const id = await ctx.db.unsafe.insert("Invoice", {
      ...patch,
      createdAt: new Date().toISOString(),
    });
    return { ok: true, id };
  },
});
