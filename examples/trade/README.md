# Trade — live market ticker

A mock equities dashboard with a seeded set of symbols and a client-
driven ticker that writes trades in a tight loop. Open two tabs — one
clicks "Start ticker", the rest watch prices move in realtime through
subscribed live queries.

**What this example demonstrates:**

- **Query fan-out under write load.** A single tab writing ~160
  trades/sec is feeding every other tab's `Ticker` subscription. The
  server de-duplicates the fanout; clients see only the rows they
  actually read.
- **Indexed range scans.** The per-symbol sparkline reads from `Trade`
  filtered by `symbol` — the `by_symbol_at` index serves it at
  O(log n) even after a minute of continuous writes.
- **Aggregation from raw events.** Ticker rows accumulate `dayHigh` /
  `dayLow` / `volume` via `recordTrade` updates; the dashboard sorts
  by computed `pct` change in the client.
- **Per-user state.** The watchlist (`Watch` rows) uses the ownership
  policy to scope reads/writes to the calling user.

## Run

```bash
cd examples/trade
bun install
bun run dev          # starts Pylon server on :4321

# in a second terminal
cd web
bun install
bun run dev          # serves the UI on :5175
```

Open <http://localhost:5175>. Click **Start ticker** in one tab. Open
a second tab — you'll see prices updating live without touching the
ticker button.

## Stress knobs

- Change the `setTimeout(step, 120)` in `TradeApp.tsx` to 60, 30, 10
  to dial up the write rate.
- Change the inner batch size (currently 20) to 50 to emit more
  writes per tick.
- Expand the `SYMBOLS` list in `seedMarket.ts` to 100, 500, 5000 to
  measure fan-out costs as row count grows.

## Files

- `app.ts` — `Ticker`, `Trade`, `Watch` entities
- `functions/seedMarket.ts` — idempotent symbol setup
- `functions/recordTrade.ts` — single-trade tick write (updates Ticker,
  appends to Trade log)
- `functions/toggleWatch.ts` — watchlist toggle
- `client/TradeApp.tsx` — dashboard UI (movers table, watchlist,
  detail panel with sparkline)
- `web/` — Vite UI
