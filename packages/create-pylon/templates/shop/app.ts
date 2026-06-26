import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  font,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// shop — a small DTC store with LIVE inventory + real Stripe checkout. The
// realtime hook: each product shows its stock ("3 left"), and the moment
// someone buys the last one it flips to "Sold out" for EVERYONE with the page
// open — no refresh. The server holds stock at checkout (under a per-product
// lock) so two shoppers can't both buy the last unit.
//
//   • Product — the catalog row, INCLUDING live stock. No PII, so it's
//               PUBLIC-READ: the grid reads it directly with `db.useQuery`,
//               which is what makes the stock count live. Seeded from config
//               on first visit. Clients can't write it; checkout holds stock
//               and restockProduct / cancelOrder return it, all server-side.
//   • Order   — a placed order line, with the customer's name + email. Holds
//               PII → denies ALL client reads/writes. Lines from one cart
//               share an `orderGroupId` (the Stripe Checkout Session's
//               client_reference_id) so the webhook settles them together.
//   • User    — the owner's account for the dashboard.
//
// Payments: the `checkout` action opens a Stripe Checkout Session when
// STRIPE_SECRET_KEY is set, and the `stripeWebhook` action (signature-verified,
// mounted at /api/webhooks/stripeWebhook) flips orders to "paid" or releases
// held stock on expiry. With NO Stripe key the store still works end-to-end:
// checkout holds the stock and records a "reserved" order the owner follows up
// on — so the template boots and demos live inventory with zero config.
//
// Order status: "pending" (awaiting Stripe payment, stock held) →
// "paid" (Stripe confirmed) | "reserved" (no Stripe; manual follow-up, stock
// held) → "fulfilled" (shipped) | "cancelled" (released, stock returned).
// ---------------------------------------------------------------------------

const Product = entity(
  "Product",
  {
    slug: field.string(),
    name: field.string(),
    priceCents: field.int(),
    description: field.string().optional(),
    image: field.string().optional(), // emoji or image URL
    stock: field.int().default(0),
  },
  { indexes: [{ name: "by_slug", fields: ["slug"], unique: true }] },
);

const Order = entity(
  "Order",
  {
    orderGroupId: field.string(), // one cart → many lines, settled together
    productSlug: field.string(),
    productName: field.string(),
    qty: field.int(),
    unitPriceCents: field.int(),
    customerName: field.string(),
    customerEmail: field.string(),
    // "pending" | "paid" | "reserved" | "fulfilled" | "cancelled"
    status: field.string().default("pending"),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_created", fields: ["createdAt"], unique: false },
      { name: "by_group", fields: ["orderGroupId"], unique: false },
    ],
  },
);

const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// Catalog + stock are public to READ — the whole point is the grid showing live
// stock to everyone. Clients can't WRITE; only seedProducts / checkout /
// restockProduct / cancelOrder maintain it server-side.
const productPolicy = policy({
  name: "product_public_read",
  entity: "Product",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// PRIVACY — Order holds the customer's name + email, so it denies EVERY client
// read and write. The public page never reads an Order; the owner reads them
// only through the owner-gated `ordersForOwner`.
const orderPolicy = policy({
  name: "order_private",
  entity: "Order",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Product, Order, User],
  queries: [],
  actions: [],
  policies: [productPolicy, orderPolicy, userPolicy],
  auth: auth(),
  // Self-hosted Inter (next/font parity): the build fetches the woff2, serves it
  // same-origin (no third-party request, no FOUT), preloads it, and synthesizes a
  // size-adjusted fallback face so there's no layout shift. globals.css reads it
  // via `var(--font-sans, …)`; layout.tsx carries no font <link>.
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
