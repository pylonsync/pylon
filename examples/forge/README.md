# Forge — collaborative 3D scene editor

Figma-for-3D. Users spawn primitives (box, sphere, cone, torus) from
the toolbar, drag them around a shared heightmap terrain, color them,
delete them. Every change broadcasts through a live query; every other
tab sees it instantly. Presence cursors show every collaborator's
pointer in 3D space.

**What this example demonstrates:**

- **Collaborative editing on shared 3D state.** Two entities with
  very different update cadences — `Prim` (low-freq, user-triggered)
  and `Cursor` (high-freq, pointer-tracking) — served by the same
  `useQuery` mechanism. No custom realtime protocol.
- **Optimistic drag with throttled writes.** Mouse-drag snaps the
  local mesh immediately; `movePrim` fires every 100ms + on drag-end.
  The live query reconciles the shared state when other clients see it.
- **Heightmap terrain sculpting.** Four brush modes (raise, lower,
  smooth, flatten) + a 4-layer splatmap paint tool. Strokes preview
  locally at 60 fps; the authoritative state writes at 10 Hz.
- **Presence cursors in 3D.** Each client writes its pointer's world
  position to `Cursor` at ~20 Hz; other clients render a small sphere
  + floating name label projected to screen space.
- **Per-user policy enforcement.** Cursors are owned by their user
  (`auth.userId == data.userId`) so nobody can hijack your pointer.
- **Native SSR.** One binary serves the frontend and API on a single
  port — no separate Vite/Next app needed.

## Run

```bash
cd examples/forge
bun install
bun run dev          # Pylon server on :4321, open http://localhost:4321
```

Open a second tab to see your cursor appear in the first tab.

## Controls

- **Left-click + drag** on a primitive — move it on the ground plane
- **Left-click** empty space — deselect
- **Right-click + drag** — orbit camera
- **Scroll** — zoom
- **Delete / Backspace** — remove selected primitive
- **Keys 1–6** — cycle color of selected primitive

## Terrain tools

- **Move** — default mode; left-click to select/drag primitives
- **Raise / Lower / Smooth / Flatten** — sculpt the heightmap
- **Paint** — blend grass / dirt / rock / snow layers

## Files

- `app.ts` — `Prim`, `Cursor`, `Terrain` entities + policies
- `app/ForgeIsland.tsx` — client bootstrap island (guest auth + SSR shell)
- `client/ForgeApp.tsx` — Three.js scene, prim mesh pool, cursor
  projection, drag + orbit + zoom, terrain rendering, brush palette
- `functions/spawnPrim.ts` — insert a primitive with random jitter
- `functions/movePrim.ts` — position update (fired on drag tick + drag-end)
- `functions/colorPrim.ts` — color change
- `functions/deletePrim.ts` — delete
- `functions/updateCursor.ts` — upsert cursor at ~20 Hz
- `functions/initTerrain.ts` — idempotent terrain seed (first join)
- `functions/sculptTerrain.ts` — apply a heightmap brush stroke
- `functions/paintTerrain.ts` — blend splatmap layer weights

## Scaling story

- A scene with 200 primitives across 10 concurrent editors stays at
  60 fps in the browser while producing ~60 cursor writes/sec total.
- Terrain sculpting at 10 Hz sends ~35 KB per stroke for a 64×64 grid.
  For MMO-scale tooling, chunk terrain into 8×8 tiles so each brush
  stroke only rewrites the affected tiles.
- Large scenes push on mesh-pool lifecycle more than sync — Pylon
  delivers the changes in milliseconds; the Three.js side is the
  dominant cost.
