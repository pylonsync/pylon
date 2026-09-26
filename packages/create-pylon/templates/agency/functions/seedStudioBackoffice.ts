import { mutation } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import { siteConfig } from "../lib/site.config";
import { lineItemsTotal, slugify, type ProjectRow } from "../lib/agency";
import { DEMO_INQUIRIES, daysAgoDate, demoDataEnabled } from "../lib/demo";

// seedStudioBackoffice — owner-only, development-only, idempotent. Fills the
// dashboard's pipeline, CRM, and billing with demo leads (lib/demo.ts) and the
// demo clients/invoices from config, so the tabs are not empty on first
// sign-in.
//
// It writes nothing unless demo data is on (lib/demo.ts: PYLON_DEMO_DATA, else
// a `pylon dev` process), so `pylon start`, Docker, and Pylon Cloud deploys do
// not get demo rows. Each table is seeded only while it is empty, so deleting
// every lead in development seeds them again on the next load. Private data
// (PII + money) is only ever seeded for the signed-in owner, never by an
// anonymous visitor, which is why this is owner-gated, unlike the public
// seedProjects.
//
// Invoices link to clients by name and to projects by slug; we resolve those to
// ids here. Projects may already be seeded (the public site calls
// seedProjects); if a project isn't present yet, the invoice still carries its
// title and just doesn't deep-link.
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  async handler(ctx) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can seed the back-office.");
    }

    if (!demoDataEnabled(ctx.env)) return { seeded: false };

    await ctx.db.advisoryLock("agency_seed_backoffice");
    const nowMs = Date.now();

    const existingInquiries = await ctx.db.unsafe.list("Inquiry");
    if (existingInquiries.length === 0) {
      for (const q of DEMO_INQUIRIES) {
        await ctx.db.unsafe.insert("Inquiry", {
          name: q.name,
          email: q.email,
          company: q.company,
          projectType: q.projectType,
          budget: q.budget,
          message: q.message,
          status: q.status,
          createdAt: new Date(nowMs - q.hoursAgo * 3_600_000).toISOString(),
        });
      }
    }

    const existingClients = await ctx.db.unsafe.list("Client");
    if (existingClients.length > 0) return { seeded: existingInquiries.length === 0 };

    const now = new Date().toISOString();

    // Clients first — keep a name → id map for the invoices.
    const clientIdByName = new Map<string, string>();
    for (const c of siteConfig.backoffice.clients) {
      const id = await ctx.db.unsafe.insert("Client", {
        name: c.name,
        company: c.company ?? null,
        email: c.email ?? null,
        phone: c.phone ?? null,
        status: c.status ?? "prospect",
        notes: c.notes ?? null,
        createdAt: now,
      });
      clientIdByName.set(c.name, id);
    }

    // Resolve project links by slug (config slug, or derived from the title).
    const projects = (await ctx.db.unsafe.list("Project")) as unknown as ProjectRow[];
    const projectBySlug = new Map(projects.map((p) => [p.slug, p]));
    const titleBySlug = new Map(
      siteConfig.work.items.map((p) => [p.slug || slugify(p.title), p.title]),
    );

    for (const inv of siteConfig.backoffice.invoices) {
      const clientId = clientIdByName.get(inv.client);
      if (!clientId) continue; // skip an invoice whose client wasn't seeded
      const project = inv.projectSlug ? projectBySlug.get(inv.projectSlug) : undefined;
      await ctx.db.unsafe.insert("Invoice", {
        number: inv.number,
        clientId,
        clientName: inv.client,
        projectId: project?.id ?? null,
        projectTitle: project?.title ?? (inv.projectSlug ? titleBySlug.get(inv.projectSlug) ?? null : null),
        lineItems: JSON.stringify(inv.lineItems),
        amountCents: lineItemsTotal(inv.lineItems),
        status: inv.status ?? "draft",
        issuedAt: inv.issuedDaysAgo !== undefined ? daysAgoDate(inv.issuedDaysAgo, nowMs) : null,
        dueAt:
          inv.issuedDaysAgo !== undefined
            ? daysAgoDate(inv.issuedDaysAgo - (inv.termsDays ?? 30), nowMs)
            : null,
        notes: null,
        createdAt: now,
      });
    }

    return { seeded: true };
  },
});
