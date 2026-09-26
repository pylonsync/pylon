import { mutation } from "@pylonsync/functions";
import { FILL_SLOTS, MARKET_KEY } from "../lib/market";

// Fake symbol set. Expand it to 500+ to measure fan-out as rows grow.
const SYMBOLS = [
  ["PYLO", "Pylonsync Inc", "Tech", 142.50],
  ["CVXD", "Convex Data", "Tech", 89.20],
  ["TMPR", "Temporal Labs", "Tech", 312.75],
  ["VCEL", "Vercel Hosting", "Tech", 64.10],
  ["LNRA", "Linear Software", "Tech", 178.40],
  ["STRI", "Stripe Financial", "Fintech", 420.00],
  ["SUPA", "Supabase Data", "Tech", 56.75],
  ["PLTR", "Planetscale", "Tech", 102.30],
  ["NEON", "Neon Postgres", "Tech", 48.90],
  ["REDS", "Redis Labs", "Tech", 215.60],
  ["DOCK", "Docker Hub", "DevOps", 78.20],
  ["HSCR", "HashiCorp", "DevOps", 188.45],
  ["GHUB", "GitHub", "DevOps", 340.00],
  ["CLFR", "Cloudflare", "Infra", 98.40],
  ["AWSX", "Amazon Cloud", "Infra", 174.20],
  ["FLYI", "Fly.io", "Infra", 24.80],
  ["RLWY", "Railway Apps", "Infra", 18.40],
  ["RNDR", "Render Services", "Infra", 42.60],
  ["DBTM", "Datadog Metrics", "Infra", 125.30],
  ["GRFN", "Grafana Labs", "Infra", 88.70],
];

/**
 * Populate the Ticker table, the Market counters row and the Fill ring.
 * Idempotent: on re-run it sets each openPrice to the current price
 * (a new market open) and leaves the counters and the tape alone.
 */
export default mutation({
  // Public demo: anyone with a guest session (POST /api/auth/guest) can seed
  // the market. Without this the function defaults to auth: "user" and rejects
  // guests.
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const now = new Date().toISOString();
    let inserted = 0;
    for (const [symbol, name, sector, seed] of SYMBOLS) {
      const existing = await ctx.db.query("Ticker", { symbol });
      if (existing.length > 0) {
        const row = existing[0];
        await ctx.db.update("Ticker", row.id as string, {
          openPrice: row.price,
          updatedAt: now,
        });
      } else {
        const price = seed as number;
        await ctx.db.insert("Ticker", {
          symbol, name, sector,
          price, openPrice: price,
          dayHigh: price, dayLow: price,
          volume: 0,
          updatedAt: now,
        });
        inserted++;
      }
    }

    const markets = await ctx.db.query("Market", { key: MARKET_KEY });
    if (markets.length === 0) {
      await ctx.db.insert("Market", {
        key: MARKET_KEY,
        seq: 0,
        trades: 0,
        volume: 0,
        notional: 0,
        updatedAt: now,
      });
    }

    const fills = await ctx.db.query("Fill", { $limit: FILL_SLOTS });
    const taken = new Set(fills.map((f) => f.slot as number));
    for (let slot = 0; slot < FILL_SLOTS; slot++) {
      if (taken.has(slot)) continue;
      // seq 0 marks an empty slot; the tape hides it.
      await ctx.db.insert("Fill", {
        slot,
        seq: 0,
        symbol: "",
        price: 0,
        qty: 0,
        side: "buy",
        at: now,
      });
    }

    return { inserted, total: SYMBOLS.length };
  },
});
