import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import { lineItemsTotal, type ClientRow, type InvoiceLineItem, type ProjectRow } from "../lib/agency";

// upsertInvoice — owner-only. Create a bill or edit one (pass `id`). The client
// is referenced by id and must exist; we denormalize its name onto the invoice
// (`clientName`) so the list renders without a join, and so the bill keeps the
// name it was issued under even if the contact is renamed later. An optional
// project link works the same way (`projectTitle`).
//
// `lineItems` is the source of truth for the amount: when present we store it
// (JSON) and compute `amountCents = Σ quantity × unitCents` server-side, so the
// total can never disagree with the breakdown the client sent. When omitted we
// fall back to the explicit `amountCents` (a one-line bill). Amounts are cents.
type Args = {
  id?: string;
  number: string;
  clientId: string;
  projectId?: string;
  lineItems?: InvoiceLineItem[];
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
    lineItems: v.optional(
      v.array(
        v.object({ description: v.string(), quantity: v.number(), unitCents: v.int() }),
      ),
    ),
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

    // Normalize line items; when present they define the total.
    const items: InvoiceLineItem[] = (args.lineItems ?? [])
      .map((it) => ({
        description: (it.description ?? "").trim().slice(0, 300),
        quantity: Math.max(0, Number(it.quantity) || 0),
        unitCents: Math.max(0, Math.trunc(Number(it.unitCents) || 0)),
      }))
      .filter((it) => it.description.length > 0 || it.unitCents > 0);

    const amountCents =
      items.length > 0 ? lineItemsTotal(items) : Math.max(0, Math.trunc(args.amountCents || 0));
    const status = STATUSES.has((args.status ?? "").trim()) ? args.status!.trim() : "draft";

    const patch = {
      number,
      clientId: client.id,
      clientName: client.name,
      projectId,
      projectTitle,
      lineItems: items.length > 0 ? JSON.stringify(items) : null,
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
