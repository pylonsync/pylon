import { mutation, v } from "@pylonsync/functions";

/**
 * Populate the caller's active org with a realistic Dallas Door Designs demo
 * dataset — customers, a door catalog, shop materials, and a handful of orders
 * across statuses and dates — so a fresh workspace shows real numbers on the
 * dashboard and Analytics instead of zeros. Idempotent: returns early if the
 * org already has customers, so it's safe to call on every workspace entry.
 */
export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const orgId = ctx.auth.tenantId;
    if (!orgId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const existing = await ctx.db.query("Customer", { orgId });
    if (existing.length > 0) return { seeded: false, reason: "already has data" };

    const now = Date.now();
    const iso = (offsetDays: number) =>
      new Date(now + offsetDays * 86_400_000).toISOString();
    const userId = ctx.auth.userId;

    // --- Customers -------------------------------------------------------
    const customers = [
      { name: "Highland Park Residence", company: "Bradford Custom Homes", city: "Dallas", state: "TX", email: "build@bradfordhomes.example" },
      { name: "Preston Hollow Estate", company: "Vantage Builders", city: "Dallas", state: "TX", email: "ops@vantagebuilders.example" },
      { name: "Lakewood Renovation", company: null, city: "Dallas", state: "TX", email: "owner@lakewood.example" },
      { name: "Southlake Town Square", company: "Cypress Commercial", city: "Southlake", state: "TX", email: "pm@cypresscommercial.example" },
    ];
    const customerIds: string[] = [];
    for (const c of customers) {
      const id = await ctx.db.insert("Customer", {
        orgId,
        name: c.name,
        email: c.email,
        phone: null,
        company: c.company,
        addressLine1: null,
        addressLine2: null,
        city: c.city,
        state: c.state,
        postal: null,
        notes: null,
        createdBy: userId,
        createdAt: iso(-30),
      });
      customerIds.push(id);
    }

    // --- Product catalog -------------------------------------------------
    const products = [
      { name: "Forged Iron Double Entry", category: "iron", basePrice: 8400, unit: "each", leadTimeDays: 56 },
      { name: "Knotty Alder Arch-Top", category: "wood", basePrice: 3650, unit: "each", leadTimeDays: 35 },
      { name: "Steel Pivot 96\"", category: "pivot", basePrice: 12500, unit: "each", leadTimeDays: 70 },
      { name: "Sliding Barn Door — Reclaimed Oak", category: "barn", basePrice: 1850, unit: "each", leadTimeDays: 21 },
      { name: "Fiberglass Patio Slider", category: "patio", basePrice: 2950, unit: "each", leadTimeDays: 28 },
      { name: "On-site Installation", category: "service", basePrice: 950, unit: "day", leadTimeDays: 0 },
    ];
    const productIds: string[] = [];
    for (const p of products) {
      const id = await ctx.db.insert("Product", {
        orgId,
        name: p.name,
        category: p.category,
        sku: null,
        description: null,
        basePrice: p.basePrice,
        unit: p.unit,
        active: true,
        leadTimeDays: p.leadTimeDays,
        createdAt: iso(-30),
      });
      productIds.push(id);
    }

    // --- Materials (one intentionally below reorder point → "low stock") -
    const materials = [
      { name: "Wrought Iron Bar Stock (1\")", unit: "ft", costPerUnit: 6.5, reorderPoint: 200, initialStock: 540, supplier: "Texas Metal Supply" },
      { name: "Knotty Alder Slab", unit: "board", costPerUnit: 145, reorderPoint: 20, initialStock: 8, supplier: "Hill Country Lumber" },
      { name: "Tempered Glass Panel (3'x7')", unit: "each", costPerUnit: 220, reorderPoint: 12, initialStock: 30, supplier: "Lone Star Glass" },
      { name: "Powder Coat — Matte Black", unit: "gal", costPerUnit: 78, reorderPoint: 6, initialStock: 14, supplier: "FinishPro" },
    ];
    for (const m of materials) {
      const materialId = await ctx.db.insert("Material", {
        orgId,
        name: m.name,
        sku: null,
        unit: m.unit,
        stockQty: m.initialStock,
        reorderPoint: m.reorderPoint,
        costPerUnit: m.costPerUnit,
        supplier: m.supplier,
        notes: null,
        createdAt: iso(-30),
      });
      await ctx.db.insert("StockMovement", {
        orgId,
        materialId,
        delta: m.initialStock,
        reason: "receipt",
        reference: "initial stock",
        performedBy: userId,
        createdAt: iso(-30),
      });
    }

    // --- Orders across statuses + dates ----------------------------------
    // Each entry: [customerIdx, status, createdOffsetDays, lines[]].
    // Statuses drive the dashboard's open-order + revenue KPIs; recent
    // createdAt makes "revenue this month" non-zero.
    const orderSpecs: Array<{
      customer: number;
      status: string;
      created: number;
      lines: Array<{ product: number; qty: number; unitPrice: number; description: string }>;
    }> = [
      { customer: 0, status: "confirmed", created: -3, lines: [{ product: 0, qty: 1, unitPrice: 8400, description: "Forged iron double entry, oil-rubbed bronze" }, { product: 5, qty: 2, unitPrice: 950, description: "Installation — 2 days" }] },
      { customer: 1, status: "in_production", created: -8, lines: [{ product: 2, qty: 1, unitPrice: 12500, description: "Steel pivot 96in, matte black" }] },
      { customer: 2, status: "confirmed", created: -1, lines: [{ product: 1, qty: 2, unitPrice: 3650, description: "Knotty alder arch-top pair" }, { product: 3, qty: 1, unitPrice: 1850, description: "Barn door — reclaimed oak" }] },
      { customer: 3, status: "delivered", created: -20, lines: [{ product: 4, qty: 6, unitPrice: 2950, description: "Fiberglass patio sliders (6 units)" }] },
      { customer: 0, status: "confirmed", created: -5, lines: [{ product: 3, qty: 3, unitPrice: 1850, description: "Barn doors — reclaimed oak (3)" }] },
    ];

    let orderNo = 0;
    const year = new Date(now).getFullYear();
    for (const spec of orderSpecs) {
      orderNo += 1;
      let subtotal = 0;
      for (const l of spec.lines) subtotal += l.qty * l.unitPrice;
      const taxRate = 0.0825;
      const tax = Math.round(subtotal * taxRate * 100) / 100;
      const total = Math.round((subtotal + tax) * 100) / 100;
      const createdAt = iso(spec.created);
      const orderId = await ctx.db.insert("Order", {
        orgId,
        customerId: customerIds[spec.customer],
        quoteId: null,
        number: `SO-${year}-${String(orderNo).padStart(4, "0")}`,
        status: spec.status,
        subtotal: Math.round(subtotal * 100) / 100,
        tax,
        total,
        notes: null,
        dueDate: iso(spec.created + 45),
        shippedAt: spec.status === "delivered" ? iso(spec.created + 14) : null,
        deliveredAt: spec.status === "delivered" ? iso(spec.created + 16) : null,
        cancelledAt: null,
        createdBy: userId,
        createdAt,
      });
      for (let i = 0; i < spec.lines.length; i++) {
        const l = spec.lines[i];
        await ctx.db.insert("OrderLine", {
          orgId,
          orderId,
          productId: productIds[l.product],
          description: l.description,
          configJson: null,
          qty: l.qty,
          unitPrice: l.unitPrice,
          lineTotal: Math.round(l.qty * l.unitPrice * 100) / 100,
          productionStatus: spec.status === "delivered" ? "done" : "queued",
          sortOrder: i,
        });
      }
    }

    return {
      seeded: true,
      customers: customerIds.length,
      products: productIds.length,
      materials: materials.length,
      orders: orderSpecs.length,
    };
  },
});
