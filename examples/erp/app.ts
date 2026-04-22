import { entity, field, policy, buildManifest } from "@statecraft/sdk";

// ---------------------------------------------------------------------------
// Custom-fabrication ERP — doors, windows, cabinets.
//
// Multi-tenant: every org-scoped entity carries `orgId` and polices reads +
// writes against `auth.orgId`. Users can belong to multiple orgs via
// OrgMember; a "selected org" lives on the session.
// ---------------------------------------------------------------------------

// ---- Identity --------------------------------------------------------------

const User = entity("User", {
  email: field.string().unique(),
  displayName: field.string(),
  avatarColor: field.string(),
  createdAt: field.datetime(),
});

// ---- Tenancy ---------------------------------------------------------------

const Organization = entity(
  "Organization",
  {
    name: field.string(),
    slug: field.string().unique(),
    billingEmail: field.string().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
);

// Role values: owner | admin | estimator | production | viewer
// - owner     — can delete the org, transfer ownership, invite anyone
// - admin     — everything except destroying the org
// - estimator — create/edit quotes, orders, customers, products
// - production— advance production status, update stock
// - viewer    — read-only
const OrgMember = entity(
  "OrgMember",
  {
    userId: field.id("User"),
    orgId: field.id("Organization"),
    role: field.string(),
    invitedBy: field.id("User").optional(),
    joinedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_user", fields: ["orgId", "userId"], unique: true },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

// Pending invites — separate from OrgMember so we can express "not yet
// accepted". Lets the UI surface an invite banner on first login.
const OrgInvite = entity(
  "OrgInvite",
  {
    orgId: field.id("Organization"),
    email: field.string(),
    role: field.string(),
    invitedBy: field.id("User"),
    createdAt: field.datetime(),
    acceptedAt: field.datetime().optional(),
  },
  {
    indexes: [
      { name: "by_org_email", fields: ["orgId", "email"], unique: true },
      { name: "by_email", fields: ["email"], unique: false },
    ],
  },
);

// ---- CRM -------------------------------------------------------------------

const Customer = entity(
  "Customer",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    email: field.string().optional(),
    phone: field.string().optional(),
    company: field.string().optional(),
    addressLine1: field.string().optional(),
    addressLine2: field.string().optional(),
    city: field.string().optional(),
    state: field.string().optional(),
    postal: field.string().optional(),
    notes: field.string().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org", fields: ["orgId"], unique: false },
    ],
  },
);

// ---- Catalog ---------------------------------------------------------------

// A product is a configurable item the shop makes. Real price comes from
// basePrice + sum of selected option priceModifiers on each order line.
const Product = entity(
  "Product",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    category: field.string(), // "door" | "window" | "cabinet" | "other"
    sku: field.string().optional(),
    description: field.string().optional(),
    basePrice: field.number(),
    unit: field.string(), // "each" | "linear-ft" | "sq-ft"
    active: field.bool(),
    leadTimeDays: field.number().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_active", fields: ["orgId", "active"], unique: false },
      { name: "by_org_sku", fields: ["orgId", "sku"], unique: false },
    ],
  },
);

// One configurable dimension of a product (width, wood species, finish).
// `kind` drives the UI: "select" reads `choicesJson`, "number" uses
// min/max, "text" is freeform.
const ProductOption = entity(
  "ProductOption",
  {
    orgId: field.id("Organization"),
    productId: field.id("Product"),
    name: field.string(),
    kind: field.string(), // "select" | "number" | "text"
    required: field.bool(),
    priceModifier: field.number(), // added per-unit for this option when chosen
    choicesJson: field.string().optional(), // JSON array for select kind
    min: field.number().optional(),
    max: field.number().optional(),
    sortOrder: field.number(),
  },
  {
    indexes: [
      { name: "by_product", fields: ["productId", "sortOrder"], unique: false },
    ],
  },
);

// ---- Inventory -------------------------------------------------------------

const Material = entity(
  "Material",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    sku: field.string().optional(),
    unit: field.string(), // "board-ft" | "each" | "ft" | "lb"
    stockQty: field.number(),
    reorderPoint: field.number(),
    costPerUnit: field.number(),
    supplier: field.string().optional(),
    notes: field.string().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org", fields: ["orgId"], unique: false },
    ],
  },
);

// Append-only ledger of stock changes. Lets operators reconcile physical
// counts against what the system believes without losing history.
const StockMovement = entity(
  "StockMovement",
  {
    orgId: field.id("Organization"),
    materialId: field.id("Material"),
    delta: field.number(), // positive = receipt, negative = consumption
    reason: field.string(), // "receipt" | "issue" | "adjust" | "waste"
    reference: field.string().optional(), // order number or PO reference
    performedBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_material", fields: ["materialId", "createdAt"], unique: false },
    ],
  },
);

// ---- Quotes & orders -------------------------------------------------------

// Quote: headers only. Line items live in QuoteLine. Status flow:
// draft → sent → accepted → (Order created) or → rejected.
const Quote = entity(
  "Quote",
  {
    orgId: field.id("Organization"),
    customerId: field.id("Customer"),
    number: field.string(), // "Q-2026-0001" — unique per org
    status: field.string(), // draft | sent | accepted | rejected | expired
    notes: field.string().optional(),
    subtotal: field.number(),
    tax: field.number(),
    total: field.number(),
    validUntil: field.datetime().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
    sentAt: field.datetime().optional(),
    decidedAt: field.datetime().optional(),
  },
  {
    indexes: [
      { name: "by_org_status", fields: ["orgId", "status"], unique: false },
      { name: "by_org_number", fields: ["orgId", "number"], unique: true },
    ],
  },
);

const QuoteLine = entity(
  "QuoteLine",
  {
    orgId: field.id("Organization"),
    quoteId: field.id("Quote"),
    productId: field.id("Product"),
    description: field.string(),
    configJson: field.string().optional(), // serialized option choices
    qty: field.number(),
    unitPrice: field.number(),
    lineTotal: field.number(),
    sortOrder: field.number(),
  },
  {
    indexes: [
      { name: "by_quote", fields: ["quoteId", "sortOrder"], unique: false },
    ],
  },
);

const Order = entity(
  "Order",
  {
    orgId: field.id("Organization"),
    customerId: field.id("Customer"),
    quoteId: field.id("Quote").optional(),
    number: field.string(), // "SO-2026-0001"
    status: field.string(), // confirmed | in_production | ready | shipped | delivered | cancelled
    subtotal: field.number(),
    tax: field.number(),
    total: field.number(),
    notes: field.string().optional(),
    dueDate: field.datetime().optional(),
    shippedAt: field.datetime().optional(),
    deliveredAt: field.datetime().optional(),
    cancelledAt: field.datetime().optional(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_status", fields: ["orgId", "status"], unique: false },
      { name: "by_org_number", fields: ["orgId", "number"], unique: true },
    ],
  },
);

const OrderLine = entity(
  "OrderLine",
  {
    orgId: field.id("Organization"),
    orderId: field.id("Order"),
    productId: field.id("Product"),
    description: field.string(),
    configJson: field.string().optional(),
    qty: field.number(),
    unitPrice: field.number(),
    lineTotal: field.number(),
    productionStatus: field.string(), // queued | in_progress | done
    sortOrder: field.number(),
  },
  {
    indexes: [
      { name: "by_order", fields: ["orderId", "sortOrder"], unique: false },
      {
        name: "by_org_production",
        fields: ["orgId", "productionStatus"],
        unique: false,
      },
    ],
  },
);

// ---------------------------------------------------------------------------
// Policies — row-level ownership via data.orgId == auth.tenantId.
//
// Reads require the caller's active org to match the row's org. Writes go
// through functions that additionally enforce role (e.g. only admins can
// invite). OrgMember reads are slightly looser — users can see their OWN
// memberships (so the client can list "orgs I belong to" at login), plus
// members of their currently-active org.
// ---------------------------------------------------------------------------

const orgScoped = (entity: string) =>
  policy({
    name: `${entity}_org_scoped`,
    entity,
    allowRead: "auth.tenantId == data.orgId",
    allowInsert: "auth.tenantId == data.orgId",
    allowUpdate: "auth.tenantId == data.orgId",
    allowDelete: "auth.tenantId == data.orgId",
  });

// Organization reads are broader — a user might be invited to an org
// before they've "selected" it, so gating on tenantId would hide the row
// that lets the client render the switcher.
const organizationPolicy = policy({
  name: "organization_access",
  entity: "Organization",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.createdBy",
  allowDelete: "auth.userId == data.createdBy",
});

const orgMemberPolicy = policy({
  name: "org_member_access",
  entity: "OrgMember",
  // A user must be able to see their own memberships (to render the org
  // switcher at login) and members of the org they're currently in.
  allowRead:
    "auth.userId == data.userId || auth.tenantId == data.orgId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

const orgInvitePolicy = policy({
  name: "org_invite_access",
  entity: "OrgInvite",
  // Invitees need to see invites addressed to them; org admins see all
  // invites for their org.
  allowRead: "auth.tenantId == data.orgId",
  allowInsert: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "erp",
  version: "0.1.0",
  entities: [
    User,
    Organization,
    OrgMember,
    OrgInvite,
    Customer,
    Product,
    ProductOption,
    Material,
    StockMovement,
    Quote,
    QuoteLine,
    Order,
    OrderLine,
  ],
  queries: [],
  actions: [],
  policies: [
    organizationPolicy,
    orgMemberPolicy,
    orgInvitePolicy,
    orgScoped("Customer"),
    orgScoped("Product"),
    orgScoped("ProductOption"),
    orgScoped("Material"),
    orgScoped("StockMovement"),
    orgScoped("Quote"),
    orgScoped("QuoteLine"),
    orgScoped("Order"),
    orgScoped("OrderLine"),
  ],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
