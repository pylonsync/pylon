/**
 * Pylon Trade: a live market dashboard fed by a client-driven market maker.
 *
 * One tab runs the ticker and calls `recordTrades` with batches of fills
 * (about 2,000 per second). Every other tab reads the results through
 * live queries:
 *   - Ticker: one row per symbol with last price, day range, volume and
 *     the open one-second candle
 *   - Bar: closed one-second OHLCV candles per symbol
 *   - Fill: a fixed ring of the most recent fills for the trade tape
 *   - Market: exchange-wide counters and the fill sequence number
 *
 * The raw Trade log is append-only and grows by thousands of rows per
 * second, so it stays out of the client replica (`sync: false`). The
 * charts read the one-second rollups instead: the open candle on each
 * Ticker plus closed Bar rows, scoped to a rolling window so every
 * replica stays bounded. Each batch changes about 27 rows however many
 * trades it carries, plus one Bar insert per symbol per second.
 */
import {
  entity,
  field,
  policy,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

const Ticker = entity(
  "Ticker",
  {
    symbol: field.string().unique(),
    name: field.string(),
    sector: field.string(),
    price: field.float(),
    openPrice: field.float(),
    dayHigh: field.float(),
    dayLow: field.float(),
    volume: field.int(),
    updatedAt: field.datetime(),
    // The in-progress one-second candle. `recordTrades` moves it into a
    // Bar row when the next second's first trade arrives.
    barT: field.datetime().optional(),
    barOpen: field.float().optional(),
    barHigh: field.float().optional(),
    barLow: field.float().optional(),
    barVolume: field.int().optional(),
    barTrades: field.int().optional(),
  },
  {
    indexes: [
      { name: "by_sector", fields: ["sector"], unique: false },
    ],
  },
);

// Every fill. Append-only; read server-side through the (symbol, at) index.
const Trade = entity(
  "Trade",
  {
    symbol: field.string(),
    price: field.float(),
    qty: field.int(),
    at: field.datetime(),
  },
  {
    indexes: [
      { name: "by_symbol_at", fields: ["symbol", "at"], unique: false },
      { name: "by_at", fields: ["at"], unique: false },
    ],
    sync: false,
  },
);

// Closed one-second OHLCV candle per symbol. Each row is inserted once,
// when its second is over; the open second lives on the Ticker row. The
// replica holds the last two minutes.
const Bar = entity(
  "Bar",
  {
    symbol: field.string(),
    t: field.datetime(),
    open: field.float(),
    high: field.float(),
    low: field.float(),
    close: field.float(),
    volume: field.int(),
    trades: field.int(),
  },
  {
    indexes: [
      { name: "by_symbol_t", fields: ["symbol", "t"], unique: true },
      { name: "by_t", fields: ["t"], unique: false },
    ],
    sync: { where: 'data.t >= ago("2m")', limit: 20_000 },
  },
);

// The trade tape: a ring of FILL_SLOTS rows. Each batch overwrites the
// oldest slots with its latest fills, so the table never grows.
const Fill = entity(
  "Fill",
  {
    slot: field.int().unique(),
    seq: field.int(),
    symbol: field.string(),
    price: field.float(),
    qty: field.int(),
    side: field.string(),
    at: field.datetime(),
  },
  {
    indexes: [{ name: "by_seq", fields: ["seq"], unique: false }],
  },
);

// Singleton row with exchange-wide counters.
const Market = entity("Market", {
  key: field.string().unique(),
  seq: field.int(),
  trades: field.int(),
  volume: field.int(),
  notional: field.float(),
  updatedAt: field.datetime(),
});

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

// The raw trade log is server-side only: `recordTrades` writes it and no
// client reads it. Denying client reads also keeps its inserts off every
// subscriber's socket.
const tradePolicy = policy({
  name: "trade_server_only",
  entity: "Trade",
  allowRead: "false",
});

const barPolicy = policy({
  name: "bar_public",
  entity: "Bar",
  allowRead: "true",
});

const fillPolicy = policy({
  name: "fill_public",
  entity: "Fill",
  allowRead: "true",
});

const marketPolicy = policy({
  name: "market_public",
  entity: "Market",
  allowRead: "true",
});

const watchPolicy = policy({
  name: "watch_ownership",
  entity: "Watch",
  // Per-user watchlist — every verb scopes to the owner.
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

const manifest = buildManifest({
  name: "trade",
  version: "0.1.0",
  entities: [Ticker, Trade, Bar, Fill, Market, Watch],
  queries: [],
  actions: [],
  policies: [tickerPolicy, tradePolicy, barPolicy, fillPolicy, marketPolicy, watchPolicy],
  // File-based SSR routing: app/page.tsx → "/". The single binary serves the
  // frontend and the API on one port — no separate Next.js app.
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));
