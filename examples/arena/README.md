# Arena — mass-multiplayer dot world

Every connected browser becomes a dot on a shared 2D plane. Click to set
a target; everyone watches every dot move in realtime. Open tabs or push
the built-in bot spawner to watch latency stay flat as N grows.

**What this example demonstrates:**

- **Live query fan-out.** A single `db.useQuery("Dot")` subscription
  feeds every client; every `moveDot` mutation broadcasts to all peers
  through the change log — no sidecar, no Redis, no separate realtime
  layer.
- **End-to-end latency HUD.** Every outgoing mutation is timed from
  send → observing our own row update bounce back. p50 / p95 shown
  live in the top-left.
- **Throughput.** Mutations/sec counter shows the actual write rate
  driven by this browser + bots.
- **No game-loop primitive needed.** The server stores positions; the
  client interpolates between updates. Works fine at 20 Hz update rate
  for hundreds of concurrent dots.

## Run

```bash
cd examples/arena
bun install
bun run dev          # starts the Pylon server on :4321

# in a second terminal
cd web
bun install
bun run dev          # serves the UI on :5174
```

Open <http://localhost:5174>, click to move, open more tabs, spawn bots.

## Stress knobs

- **+10 / +100 / +500 bots** — inserts bot rows that pick new random
  targets every ~1.2s. Each tab that has the UI open contributes bot
  moves; one tab can drive ~1k bots comfortably.
- **Clear bots** — sweep all `isBot: true` dots.
- **Show target line** — toggle the dashed trail for your own dot.

## Files

- `app.ts` — `Dot` + `ArenaStats` entities, ownership policy
- `functions/spawnDot.ts` — idempotent per-user dot creation
- `functions/moveDot.ts` — target update, clamped to `[0,1]`
- `functions/removeBots.ts` — bulk delete of bot rows
- `client/ArenaApp.tsx` — canvas, interpolation, HUD, bot driver
- `web/` — Vite UI mounting `ArenaApp`

## What to look for

The latency HUD turns red when p95 exceeds 100ms — that's where the canvas starts to feel laggy. Open multiple tabs and push bot counts (+10, +100, +500) to watch p50/p95 evolve as concurrency grows. The interesting thing is how flat latency stays as N grows — the live-query fan-out is the same code path regardless of subscriber count.

For canonical throughput numbers across hardware tiers, see [Sizing](https://docs.pylonsync.com/operations/sizing). Run the [`bench`](../bench) example to measure your own box.
