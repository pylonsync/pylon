import { mutation, v } from "@pylonsync/functions";

// Fake symbol set — feel free to expand to 500+ for realism.
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
 * Populate the Ticker table. Idempotent — on re-run it just bumps the
 * openPrice to current price (simulating a market open).
 */
export default mutation({
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
    return { inserted, total: SYMBOLS.length };
  },
});
