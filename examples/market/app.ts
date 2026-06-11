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

// Public marketplace: everyone can read listings + offers (so buyers and
// sellers both see the live state). Writes require a session and are
// owner-scoped; the heavy lifting (accept = mark sold + decline siblings)
// runs in functions where it can enforce "only the seller responds".
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
  entities: [Listing, Offer],
  queries: [],
  actions: [],
  policies: [listingPolicy, offerPolicy],
  // File-based SSR routing: app/**/page.tsx. One binary serves the frontend
  // and the API on one port.
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
