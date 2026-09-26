import { mutation, v } from "@pylonsync/functions";
import { FILL_SLOTS, MARKET_KEY } from "../lib/market";

// Fills from each batch that go onto the tape. The rest of the batch is
// still recorded in Trade, Bar and Ticker.
const TAPE_PER_BATCH = 6;

/**
 * Record a batch of fills in one transaction. For each fill: append a
 * Trade row. For each symbol in the batch: update the Ticker's price, day
 * range, volume and open one-second candle, closing the candle into a Bar
 * row when the second has changed. Then advance the Market counters and
 * overwrite the oldest tape slots with the batch's latest fills.
 *
 * The market-maker tab calls this in a loop. A production feed would run
 * the same writes from a scheduled job or an ingest endpoint.
 */
export default mutation({
  // Public demo: any guest session can drive the market maker.
  auth: "guest",
  args: {
    trades: v.array(
      v.object({
        symbol: v.string(),
        price: v.number(),
        qty: v.int(),
      }),
    ),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (args.trades.length === 0) return { recorded: 0 };
    if (args.trades.length > 1000) {
      throw ctx.error("BATCH_TOO_LARGE", "send at most 1000 trades per call");
    }

    const nowMs = Date.now();
    const now = new Date(nowMs).toISOString();
    const second = new Date(Math.floor(nowMs / 1000) * 1000).toISOString();

    const bySymbol = new Map<string, Array<{ price: number; qty: number }>>();
    for (const t of args.trades) {
      const price = Number(t.price);
      const qty = Math.floor(Number(t.qty));
      if (!(price > 0) || !(qty > 0)) {
        throw ctx.error("INVALID_TRADE", `bad price or qty for ${t.symbol}`);
      }
      const list = bySymbol.get(t.symbol) ?? [];
      list.push({ price, qty });
      bySymbol.set(t.symbol, list);
    }

    // Side of each fill: "buy" on an uptick or flat, "sell" on a downtick.
    const sides: string[] = new Array(args.trades.length);
    const lastSeen = new Map<string, number>();
    let volume = 0;
    let notional = 0;

    for (const [symbol, fills] of bySymbol) {
      const rows = await ctx.db.query("Ticker", { symbol });
      if (rows.length === 0) throw ctx.error("NOT_FOUND", `ticker ${symbol}`);
      const ticker = rows[0];
      lastSeen.set(symbol, ticker.price as number);

      let high = ticker.dayHigh as number;
      let low = ticker.dayLow as number;
      let batchHigh = -Infinity;
      let batchLow = Infinity;
      let qtySum = 0;
      for (const f of fills) {
        batchHigh = Math.max(batchHigh, f.price);
        batchLow = Math.min(batchLow, f.price);
        qtySum += f.qty;
        notional += f.price * f.qty;
      }
      high = Math.max(high, batchHigh);
      low = Math.min(low, batchLow);
      volume += qtySum;
      const last = fills[fills.length - 1].price;

      // The open candle continues in the same second. In a new second the
      // previous candle is final: store it as a Bar and start a new one.
      const barT = (ticker.barT as string | null | undefined) ?? null;
      let bar: Record<string, unknown>;
      if (barT === second) {
        bar = {
          barHigh: Math.max(ticker.barHigh as number, batchHigh),
          barLow: Math.min(ticker.barLow as number, batchLow),
          barVolume: (ticker.barVolume as number) + qtySum,
          barTrades: (ticker.barTrades as number) + fills.length,
        };
      } else {
        if (barT !== null && ((ticker.barTrades as number | null) ?? 0) > 0) {
          await ctx.db.insert("Bar", {
            symbol,
            t: barT,
            open: ticker.barOpen as number,
            high: ticker.barHigh as number,
            low: ticker.barLow as number,
            close: ticker.price as number,
            volume: ticker.barVolume as number,
            trades: ticker.barTrades as number,
          });
        }
        bar = {
          barT: second,
          barOpen: fills[0].price,
          barHigh: batchHigh,
          barLow: batchLow,
          barVolume: qtySum,
          barTrades: fills.length,
        };
      }

      await ctx.db.update("Ticker", ticker.id as string, {
        price: last,
        volume: (ticker.volume as number) + qtySum,
        dayHigh: high,
        dayLow: low,
        updatedAt: now,
        ...bar,
      });
    }

    for (let i = 0; i < args.trades.length; i++) {
      const t = args.trades[i];
      const prev = lastSeen.get(t.symbol) ?? t.price;
      sides[i] = t.price >= prev ? "buy" : "sell";
      lastSeen.set(t.symbol, t.price);
      await ctx.db.insert("Trade", {
        symbol: t.symbol,
        price: Number(t.price),
        qty: Math.floor(Number(t.qty)),
        at: now,
      });
    }

    const markets = await ctx.db.query("Market", { key: MARKET_KEY });
    if (markets.length === 0) throw ctx.error("NOT_SEEDED", "run seedMarket first");
    const market = markets[0];
    const seqStart = market.seq as number;
    await ctx.db.update("Market", market.id as string, {
      seq: seqStart + args.trades.length,
      trades: (market.trades as number) + args.trades.length,
      volume: (market.volume as number) + volume,
      notional: (market.notional as number) + notional,
      updatedAt: now,
    });

    const onTape = Math.min(TAPE_PER_BATCH, args.trades.length, FILL_SLOTS);
    const slots = await ctx.db.query("Fill", {
      $order: { seq: "asc" },
      $limit: onTape,
    });
    const first = args.trades.length - onTape;
    for (let i = 0; i < slots.length; i++) {
      const idx = first + i;
      const t = args.trades[idx];
      await ctx.db.update("Fill", slots[i].id as string, {
        seq: seqStart + idx + 1,
        symbol: t.symbol,
        price: Number(t.price),
        qty: Math.floor(Number(t.qty)),
        side: sides[idx],
        at: now,
      });
    }

    return { recorded: args.trades.length };
  },
});
