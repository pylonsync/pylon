# Forge — collaborative 3D scene editor

Figma-for-3D. Users spawn primitives (box, sphere, cone, torus) from
the toolbar, drag them around a shared grid, color them, delete them.
Every change broadcasts through a live query; every other tab sees
it instantly. Presence cursors show every collaborator's pointer in
3D space.

**What this example demonstrates:**

- **Collaborative editing on shared 3D state.** Two entities with
  very different update cadences — `Prim` (low-freq, user-triggered)
  and `Cursor` (high-freq, pointer-tracking) — served by the same
  `useQuery` mechanism. No custom realtime protocol.
- **Optimistic drag with throttled writes.** Mouse-drag snaps the
  local mesh immediately; `movePrim` fires every 100ms + on drag-end.
  The live query reconciles the shared state when other clients see
  it.
- **Presence cursors in 3D.** Each client writes its pointer's world
  position to `Cursor` at ~20 Hz; other clients render a small sphere
  + floating name label projected to screen space.
- **Per-user policy enforcement.** Cursors are owned by their user
  (`auth.userId == data.userId`) so nobody can hijack your pointer.

## Run

```bash
cd examples/forge
bun install
bun run dev          # Pylon server on :4321

# second terminal
cd web
bun install
bun run dev          # UI on :5178
```

Open <http://localhost:5178>. Spawn a box, drag it around. Open a
second tab — you'll see your cursor in the first tab tracking the
mouse in the second.

## Controls

- **Left-click + drag** on a primitive — move it on the ground plane
- **Left-click** empty space — deselect
- **Right-click + drag** — orbit camera
- **Scroll** — zoom
- **Delete / Backspace** — remove selected primitive
- **Keys 1–6** — cycle color of selected primitive

## Files

- `app.ts` — `Prim` + `Cursor` entities, ownership policy on Cursor
- `functions/spawnPrim.ts` — insert a primitive with random jitter
- `functions/movePrim.ts` — position update (fired on drag tick +
  drag-end)
- `functions/colorPrim.ts` — color change
- `functions/deletePrim.ts` — delete
- `functions/updateCursor.ts` — upsert cursor at ~20 Hz
- `client/ForgeApp.tsx` — three.js scene, prim mesh pool, cursor
  projection, drag + orbit + zoom input
- `web/` — Vite UI

## Scaling story

- A scene with 200 primitives across 10 concurrent editors stays at
  60 fps in the browser while producing ~60 cursor writes/sec total.
- Large scenes push on mesh-pool lifecycle more than sync — Pylon
  delivers the changes in milliseconds; the three.js side is the
  dominant cost.
