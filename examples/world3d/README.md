# World3D — 3D multiplayer avatar world

Every connected browser becomes an avatar cube in a shared three.js
scene. WASD to move, click + drag for mouse-look, open tabs to spawn
more players. A bot spawner drops autonomous avatars so you can
stress-test a crowded world from a single machine.

**What this example demonstrates:**

- **3D realtime sync with zero special primitives.** Avatar positions
  live in a single `Avatar` table. `db.useQuery("Avatar")` powers
  both your own state and every other player's. No game-server
  framework, no separate netcode.
- **Client-side interpolation** absorbs the ~10 Hz server update rate
  so motion stays smooth at 60 fps even with 200+ avatars.
- **End-to-end latency HUD** — every outgoing `moveAvatar` mutation
  is timed from send → own-row-bounces-back in the live query.
  p50 / p95 shown live in the top-left.
- **Screen-space name labels** — each avatar has a DOM label projected
  from 3D world position to 2D screen every frame.

## Run

```bash
cd examples/world3d
bun install
bun run dev          # Pylon server on :4321

# second terminal
cd web
bun install
bun run dev          # UI on :5177
```

Open <http://localhost:5177>. Click the scene to engage pointer lock
(WASD + mouse). Open more tabs for real multiplayer. Hit **+200** to
stress-test with bots.

## Stress knobs

- **+10 / +50 / +200 bots** — creates bot avatars that wander randomly.
  Each tab that has the UI open drives its bots, so one tab with 200
  bots running is writing ~200 × ~0.5 rps = 100 mut/sec broadcasts.
- **Clear bots** — wipes all `isBot: true` rows.

## Files

- `app.ts` — `Avatar` entity with position + heading + bot flag
- `functions/spawnAvatar.ts` — idempotent per-user avatar creation
- `functions/moveAvatar.ts` — pose update, clamped to the 40×40 plane
- `functions/clearBots.ts` — bulk delete for bot cleanup
- `client/WorldApp.tsx` — three.js scene setup, input, interpolation,
  HUD. One effect owns the scene; avatars are added/removed from
  meshes Map as the live query changes.
- `web/` — Vite UI mounting `WorldApp`

## Dependencies

- `three` + `@types/three` pinned in `web/package.json`. No orbit
  controls / pointer-lock-controls helpers — we do the math inline
  to keep the bundle small and the code readable.
