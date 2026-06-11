/**
 * Pylon Market — a live local marketplace.
 *
 * Anyone (a guest session) can list an item for sale; anyone else can make
 * an offer. Sellers watch offers arrive in realtime and accept/decline them.
 *
 * The Pylon story this demo tells:
 *   - The browse grid (`/`) and each listing page (`/listing/:id`) are
 *     SERVER-RENDERED with real rows from the database (good for SEO + LCP) —
 *     view source and the products are in the HTML, not fetched later.
 *   - The interactive, realtime bits — the "just listed" ticker, the live
 *     offers on a listing, your inbox on `/me` — ride the sync engine: a
 *     single `useQuery` fans every write out to every open tab instantly.
 *   - One binary, one port. SSR + REST + WebSockets all from `pylon dev`.
 *     No Next.js app, no separate realtime service.
 */
import {
  entity,
  field,
  policy,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

// Accounts. Email/password auth is built in: registering through
// /api/auth/password/register hashes the password and writes this row.
// `passwordHash` is server-only — never serialized to clients.
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string(),
    // Set by the auth subsystem on register (a default avatar tint).
    avatarColor: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [{ name: "by_email", fields: ["email"], unique: true }],
  },
);

// A thing for sale. `seed` drives a deterministic gradient "photo" so the
// demo needs no image hosting. `status` flips active → sold when an offer
// is accepted.
//
// `sellerId: field.owner()` is what lets SellForm create a listing with a
// plain, optimistic `db.insert` (it shows in the live ticker the instant
// you post — no server round-trip) while the seller id stays unspoofable:
// the framework stamps it from the session and rejects any forged value.
// No createListing function needed. `status` + `createdAt` default
// server-side so the client doesn't have to send them.
const Listing = entity(
  "Listing",
  {
    sellerId: field.string().owner(),
    sellerName: field.string(),
    title: field.string(),
    // Human-readable URL key: "herman-miller-aeron-size-b-a1f3". Unique so it
    // addresses exactly one listing; the detail route resolves by it.
    slug: field.string().unique(),
    description: field.string(),
    price: field.float(),
    category: field.string(),
    condition: field.string(), // new | like-new | good | fair
    status: field.string().default("active"), // active | sold
    seed: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_status", fields: ["status"], unique: false },
      { name: "by_seller", fields: ["sellerId"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
      { name: "by_slug", fields: ["slug"], unique: true },
    ],
  },
);

// A buyer's bid on a listing. The seller responds; accepting marks the
// listing sold and auto-declines the rest.
const Offer = entity(
  "Offer",
  {
    listingId: field.string(),
    listingTitle: field.string(),
    sellerId: field.string(),
    // `buyerId: field.owner()` keeps the bidder unspoofable on the
    // optimistic db.useMutation path too — even though makeOffer also
    // stamps it server-side, the field-level guarantee is defense in
    // depth (and documents intent).
    buyerId: field.string().owner(),
    buyerName: field.string(),
    amount: field.float(),
    message: field.string().optional(),
    status: field.string().default("pending"), // pending | accepted | declined
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_listing", fields: ["listingId"], unique: false },
      { name: "by_buyer", fields: ["buyerId"], unique: false },
      { name: "by_seller", fields: ["sellerId"], unique: false },
    ],
  },
);

// A buyer's saved listing. Private to the watcher (owner-scoped read), so
// your watchlist is yours alone. `listingTitle` is denormalized so the
// "Watching" list renders without a join.
const Watch = entity(
  "Watch",
  {
    userId: field.string().owner(),
    listingId: field.string(),
    listingTitle: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      // One watch per (user, listing) — toggling the heart inserts/deletes
      // this row.
      { name: "by_user_listing", fields: ["userId", "listingId"], unique: true },
    ],
  },
);

// Public marketplace: everyone can read listings + offers (so buyers and
// sellers both see the live state). Writes require a session and are
// owner-scoped; the heavy lifting (accept = mark sold + decline siblings)
// runs in functions where it can enforce "only the seller responds".
// Signed-in users can read profiles (to render seller/buyer names). The
// auth subsystem owns writes — registration/login go through
// /api/auth/password/*, not the entity API, so direct inserts/updates are
// closed off.
const userPolicy = policy({
  name: "user_access",
  entity: "User",
  allowRead: "auth.userId != null",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Watchlists are private: you can only read, add to, or remove from your own.
const watchPolicy = policy({
  name: "watch_access",
  entity: "Watch",
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId != null",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.userId",
});

const listingPolicy = policy({
  name: "listing_access",
  entity: "Listing",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.sellerId",
  allowDelete: "auth.userId == data.sellerId",
});

const offerPolicy = policy({
  name: "offer_access",
  entity: "Offer",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  // Buyers can withdraw their own offer; the seller's accept/decline goes
  // through respondToOffer (which checks ownership of the listing).
  allowUpdate: "auth.userId == data.buyerId || auth.userId == data.sellerId",
  allowDelete: "auth.userId == data.buyerId",
});

const manifest = buildManifest({
  name: "market",
  version: "0.1.0",
  entities: [User, Listing, Offer, Watch],
  queries: [],
  actions: [],
  policies: [userPolicy, listingPolicy, offerPolicy, watchPolicy],
  // File-based SSR routing: app/**/page.tsx. One binary serves the frontend
  // and the API on one port.
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
