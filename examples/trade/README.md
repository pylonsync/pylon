# Trade: live market dashboard

A dark, data-dense equities dashboard over a seeded set of 20 symbols. One
tab clicks "Start ticker" and runs a client-side market maker that writes
about 2,000 trades per second. Every other tab updates through live
queries.

The dashboard shows:

- A market table with last price, change from open, a 60-second sparkline
  and volume per symbol
- One-second candles with volume for the selected symbol (90 seconds)
- Fills per second across the market
- A sector treemap sized by dollar volume and colored by change from open
- Market breadth: advancers and decliners, up and down volume, sector change
- Top gainers and losers
- A trade tape of the latest fills
- Write latency (p50, p99) in the ticker tab and sync lag in every tab

**What this example demonstrates:**

- **Query fan-out under write load.** The ticker tab sends batches of 500
  trades to `recordTrades` four times per second. Each batch changes about
  27 synced rows (one Ticker per symbol, six tape slots, the Market row),
  plus one Bar insert per symbol per second. Every subscribed tab gets
  those changes over its socket and stays caught up.
- **Rollups instead of raw rows.** The raw `Trade` log grows by 2,000 rows
  per second. It is `sync: false` and its read policy denies clients, so it
  never reaches a browser. Charts read one-second candles instead: the open
  candle on each Ticker row and closed `Bar` rows.
- **Rolling sync windows.** `Bar` uses `sync: { where: 'data.t >= ago("2m")' }`,
  so each replica holds two minutes of candles however long the ticker runs.
- **A bounded tape.** `Fill` is a ring of 48 rows. Each batch overwrites the
  oldest slots, so the table never grows.
- **Per-user state.** The watchlist (`Watch` rows) uses the ownership
  policy to scope reads and writes to the calling user. Watched symbols
  sort to the top of the market table.

## Run

```bash
cd examples/trade
bun install
bun run dev          # starts Pylon on :4321, serving both UI and API
```

Open <http://localhost:4321>. Click **Start ticker** in one tab. Open
a second tab — you'll see prices updating live without touching the
ticker button.

## Stress knobs

In `client/marketMaker.ts`:

- `TARGET_RATE`: trades per second (default 2,000)
- `BATCH_SIZE`: trades per `recordTrades` call (default 500). Smaller
  batches mean more synced row changes per second for every tab.
- `MAX_IN_FLIGHT`: concurrent calls (default 2)

In `functions/seedMarket.ts`, expand `SYMBOLS` to 100, 500 or 5,000 to
measure fan-out cost as the row count grows.

## Files

- `app.ts`: `Ticker`, `Trade`, `Bar`, `Fill`, `Market`, `Watch` entities and policies
- `lib/market.ts`: constants shared by the functions
- `functions/seedMarket.ts`: idempotent setup of symbols, the Market row and the tape ring
- `functions/recordTrades.ts`: records a batch of trades (Trade log, Ticker,
  closed Bars, Market counters, tape)
- `functions/toggleWatch.ts`: watchlist toggle (uses `ctx.auth.userId`,
  never a caller-supplied user ID)
- `app/TradeIsland.tsx`: SSR shell that creates the guest session before
  mounting the client island
- `client/TradeApp.tsx`: dashboard layout and panels
- `client/charts.tsx`: sparkline, candle chart, throughput chart, sector treemap
- `client/marketMaker.ts`: the client-side market maker

## What to look for

Open two tabs side by side and start the ticker in one. The other tab's charts, tape and counters move with it through live queries, with no app-specific socket. Compare "Sync lag p50" in both tabs.

Try these load dimensions:

- **Batch size**: lower `BATCH_SIZE` to raise the synced change rate at the same trade rate
- **Symbol count**: expand `SYMBOLS` in `functions/seedMarket.ts` (20 → 100 → 500 → 5000) to see how live-query fan-out cost scales with subscribed-row count
- **Subscriber count**: open more reader tabs (or use the bot driver) to put fan-out load on the server

For canonical throughput numbers across hardware tiers, see [Sizing](https://docs.pylonsync.com/operations/sizing). Run the [`bench`](../bench) example to measure your own box.
