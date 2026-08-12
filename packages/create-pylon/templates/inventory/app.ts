import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
  font,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// inventory — stock control for a small business: products, an append-only
// ledger of movements, and the levels derived from it.
//
// THE CENTRAL DECISION: there is no `quantity` column. On-hand is the SUM of a
// product's movements. A mutable counter is the classic inventory bug — two
// people receive the same delivery, both read 10, both write 15, and five units
// vanish with nothing to audit. Appending "+5" twice gives 20 and shows exactly
// who did it. It also makes every count explainable from the same rows that
// produced it.
//
//   • Product  — sku, name, cost, price, reorder point.
//   • Movement — a signed delta with a reason. Append-only; never edited.
//   • User     — a team member (email/password).
//
// Money is INTEGER CENTS; quantities are whole units. You cannot hold half a
// physical thing, and allowing it hides unit-of-measure mistakes.
//
// The realtime hook: a movement recorded at the back door updates the level on
// the shop floor's screen immediately — nobody sells what was just damaged.
//
// TRUST MODEL: an internal tool for ONE business. Every entity is readable and
// writable by any SIGNED-IN user and by nobody else.
// ---------------------------------------------------------------------------

const TEAM = "auth.userId != null";

const Product = entity(
  "Product",
  {
    // Unique: two rows sharing a SKU means two different answers to "how many
    // do we have", which is the question this app exists to answer.
    sku: field.string().unique(),
    name: field.string(),
    category: field.string().optional(),
    // What it costs YOU — valuation is at cost, not at retail.
    unitCostCents: field.number().default(0),
    unitPriceCents: field.number().default(0),
    // Order more at or below this. 0 / unset means never flag it.
    reorderPoint: field.number().default(0),
    // Discontinued lines stay for their movement history rather than being
    // deleted — removing the product would orphan the ledger that explains
    // last quarter's numbers.
    archived: field.boolean().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_sku", fields: ["sku"], unique: true },
      { name: "by_category", fields: ["category"], unique: false },
    ],
  },
);

// APPEND-ONLY. Nothing in the app updates or deletes a movement: a correction
// is another movement in the opposite direction, which is what makes the
// history trustworthy. The policy enforces that, not just convention.
const Movement = entity(
  "Movement",
  {
    productId: field.id("Product"),
    // Signed whole units: positive receives, negative issues. Never zero.
    delta: field.number(),
    // "received" | "sold" | "returned" | "damaged" | "count" — see REASONS.
    reason: field.string(),
    note: field.string().optional(),
    actorId: field.id("User").readonly().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_product", fields: ["productId"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
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

const productPolicy = policy({
  name: "product_team",
  entity: "Product",
  allowRead: TEAM,
  allowInsert: TEAM,
  allowUpdate: TEAM,
  allowDelete: TEAM,
});

// The ledger is INSERT-ONLY at the policy level. Editing a movement would
// silently rewrite history and change a past valuation; deleting one would make
// the current level unexplainable. Corrections are new rows.
const movementPolicy = policy({
  name: "movement_append_only",
  entity: "Movement",
  allowRead: TEAM,
  allowInsert: TEAM,
  allowUpdate: "false",
  allowDelete: "false",
});

const userPolicy = policy({
  name: "user_team_read",
  entity: "User",
  allowRead: TEAM,
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Product, Movement, User],
  queries: fns.queries,
  actions: fns.actions,
  policies: [productPolicy, movementPolicy, userPolicy],
  auth: auth(),
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
