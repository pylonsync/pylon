/**
 * Pylon Trade — mock market ticker at scale.
 *
 * 500 symbols getting price updates driven by a client-side "market
 * maker" tab. Every subscriber sees their watchlist update in real
 * time. Demonstrates query fan-out, large-index performance, and the
 * ergonomics of building a trading dashboard without a separate
 * realtime layer.
 *
 * What you'll see:
 *   - Top movers (filtered query, live-ordered)
 *   - Per-symbol sparkline computed client-side from Trade rows
 *   - Throughput counter showing ticks/sec across the whole market
 *
 * The scaling story this demo tells:
 *   - ~500 rows in Ticker, 50k+ rows in Trade after a minute
 *   - Indexed query on (symbol, at) serves sparkline lookups at O(log n)
 *   - Live subscription to a watchlist of 10 symbols stays <1ms even
 *     while the market-maker tab is writing 200 trades/sec
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

const Ticker = entity(
  "Ticker",
  {
    symbol: field.string().unique(),
    name: field.string(),
    sector: field.string(),
    price: field.number(),
    openPrice: field.number(),
    dayHigh: field.number(),
    dayLow: field.number(),
    volume: field.number(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_sector", fields: ["sector"], unique: false },
    ],
  },
);

// Individual trade ticks. Written frequently by the market-maker, read
// by sparkline views. Indexed on (symbol, at) for fast per-symbol
// range scans.
const Trade = entity(
  "Trade",
  {
    symbol: field.string(),
    price: field.number(),
    qty: field.number(),
    at: field.datetime(),
  },
  {
    indexes: [
      { name: "by_symbol_at", fields: ["symbol", "at"], unique: false },
      { name: "by_at", fields: ["at"], unique: false },
    ],
  },
);

// Per-user watchlist — rows the user has pinned to their dashboard.
const Watch = entity(
  "Watch",
  {
    userId: field.string(),
    symbol: field.string(),
    addedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_user_symbol", fields: ["userId", "symbol"], unique: true },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

const tickerPolicy = policy({
  name: "ticker_public",
  entity: "Ticker",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
});

const tradePolicy = policy({
  name: "trade_public",
  entity: "Trade",
  allowRead: "true",
  allowInsert: "auth.userId != null",
});

const watchPolicy = policy({
  name: "watch_ownership",
  entity: "Watch",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

const manifest = buildManifest({
  name: "trade",
  version: "0.1.0",
  entities: [Ticker, Trade, Watch],
  queries: [],
  actions: [],
  policies: [tickerPolicy, tradePolicy, watchPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
