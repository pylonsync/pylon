/**
 * Pylon Auction House — timed + live auctions with many lots each.
 *
 * Two flavors:
 *   - **Timed auction**: every lot has its own `endsAt`; bidders place
 *     bids any time before the deadline. A scheduled `closeExpiredLots`
 *     pass finalizes winners.
 *   - **Live auction**: an auctioneer opens lots one at a time. Each
 *     open lot has a sliding deadline that resets a few seconds after
 *     every fresh bid (the "going once… going twice…" antishill timer).
 *     When the timer expires, the lot is sold and the auctioneer can
 *     advance to the next.
 *
 * Demonstrates Pylon strengths:
 *   - Live `db.useQuery` for bid feeds → no app-specific WebSocket code
 *   - Server-side mutations for atomic bid validation
 *   - Scheduled functions for auto-closing
 *   - Per-user auth + balance enforcement
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string(),
    avatarColor: field.string().optional(),
    passwordHash: field.string().optional(),
    balanceCents: field.int().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_email", fields: ["email"], unique: true }],
  },
);

// An Auction groups many Lots. `kind` is "timed" (every lot has its own
// deadline, bidders race against the clock independently) or "live"
// (auctioneer opens one lot at a time; the open lot has an antishill
// timer that bumps on every fresh bid).
const Auction = entity(
  "Auction",
  {
    title: field.string(),
    description: field.string(),
    kind: field.string(), // "timed" | "live"
    status: field.string(), // "draft" | "scheduled" | "running" | "ended"
    creatorId: field.string(),
    startsAt: field.datetime(),
    endsAt: field.datetime(),
    currentLotId: field.string().optional(),
    bannerColor: field.string().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_status", fields: ["status"], unique: false },
      { name: "by_starts_at", fields: ["startsAt"], unique: false },
      { name: "by_creator", fields: ["creatorId"], unique: false },
    ],
    search: {
      text: ["title", "description"],
      facets: ["status", "kind"],
      sortable: ["startsAt", "endsAt", "createdAt"],
    },
  },
);

// A Lot is one item within an Auction. `endsAt` is per-lot for timed
// auctions; for live auctions the auctioneer sets it dynamically when
// opening the lot.
const Lot = entity(
  "Lot",
  {
    auctionId: field.string(),
    position: field.int(),
    title: field.string(),
    description: field.string(),
    imageColor: field.string().optional(),
    startingCents: field.int(),
    currentCents: field.int(),
    minIncrementCents: field.int(),
    bidCount: field.int(),
    status: field.string(), // "pending" | "running" | "sold" | "passed"
    endsAt: field.datetime().optional(),
    winningBidId: field.string().optional(),
    winnerId: field.string().optional(),
    soldAt: field.datetime().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_auction", fields: ["auctionId"], unique: false },
      { name: "by_status", fields: ["status"], unique: false },
      { name: "by_ends_at", fields: ["endsAt"], unique: false },
    ],
  },
);

const Bid = entity(
  "Bid",
  {
    auctionId: field.string(),
    lotId: field.string(),
    bidderId: field.string(),
    bidderName: field.string(),
    amountCents: field.int(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_lot", fields: ["lotId"], unique: false },
      { name: "by_bidder", fields: ["bidderId"], unique: false },
    ],
  },
);

// Watchlist — per-user pinned lots so a bidder can see their watched
// items light up when bid activity arrives.
const Watch = entity(
  "Watch",
  {
    userId: field.string(),
    lotId: field.string(),
    addedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_lot", fields: ["lotId"], unique: false },
    ],
  },
);

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

// Users read their own row.
const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId != null",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Auctions + lots are public. Writes go through server functions.
const auctionPolicy = policy({
  name: "auction_public",
  entity: "Auction",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const lotPolicy = policy({
  name: "lot_public",
  entity: "Lot",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Bids are public so spectators can watch the bidding war. Writes
// always flow through `placeBid` for atomic price/balance checks.
const bidPolicy = policy({
  name: "bid_public",
  entity: "Bid",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Watchlist is per-user.
const watchPolicy = policy({
  name: "watch_owner",
  entity: "Watch",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId == data.userId",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "auction-house",
  version: "0.1.0",
  entities: [User, Auction, Lot, Bid, Watch],
  queries: [],
  actions: [],
  policies: [userPolicy, auctionPolicy, lotPolicy, bidPolicy, watchPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
