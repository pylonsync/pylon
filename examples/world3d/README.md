# World3D — multiplayer procedural-island FPS

A fully procedural tropical island rendered in Three.js and served by
Pylon's native SSR from one binary and one port. Every browser tab is a
player; buildings you demolish crumble for everyone in realtime.

## What this example demonstrates

- **Pylon SSR as a game shell.** `app/page.tsx` server-renders an
  instant HUD shell, then dynamic-imports the engine — three.js ships
  as its own async chunk and never loads during SSR.
- **A realtime shard for movement and combat.** The `island` shard
  (`shards/island`, Rust compiled to WebAssembly and run by the Pylon
  server) holds every player's pose and health. `game/net.ts` uses
  `connectShardGame` from `@pylonsync/realtime`:
  - your moves go up at 20 Hz as the distance moved since the last one;
    the shard applies them within a speed budget and the island bounds,
    and prediction puts you back where the shard has you when it clamps
    one;
  - other players are drawn 100 ms behind the shard, interpolated between
    its frames, with no React render per frame;
  - hits are range-checked and damage is capped in the shard, which also
    owns respawns.
  Names and colors stay in the `Avatar` table (`db.useQuery("Avatar")`).
- **Deterministic worldgen as a sync strategy.** The entire island —
  terrain, water, sky, vegetation, buildings — generates from one
  seed, so multiplayer only syncs *destroyed block keys* (a
  `Destruction` row each). Structural collapse is a pure function of
  that set: every client derives identical rubble.
- **A small game-engine core** (`game/engine.ts`): ordered systems, a
  typed event bus, and object pools, all on a fixed frame clock.

## Run

```bash
cd examples/world3d
bun install
rustup target add wasm32-unknown-unknown   # once, for the shard
bun run dev          # pylon dev — everything on :4321; builds the shard
```

Open <http://localhost:4321>, click to deploy, and open more tabs for
real multiplayer. WASD + mouse, shift to sprint, space to jump/swim,
G or right-click for grenades, R to reload. "rebuild island" restores
every demolished building (deletes all Destruction rows).

## The world

- `game/terrain.ts`: 257² heightfield: radial island falloff, fbm
  hills, a ridged mountain spine, beach flattening. Doubles as the
  collision query (`heightAt`) and placement oracle for everything else.
- `game/water.ts`: single-quad ocean shader: depth-ramped tropical
  color from the heightmap, scrolling-noise detail normals, sun
  glints, animated breaker + foam bands at the shoreline.
- `game/sky.ts`: gradient dome with analytic sun + forward-scatter
  haze, drifting billboard clouds, and a shadow frustum that follows
  the player snapped to texel steps.
- `game/vegetation.ts`: instanced palms (procedural curved trunks +
  alpha-cutout fronds), EZ Tree species (ash/oak/pine via the MIT
  `@dgreenheck/ez-tree` generator), ferns, broadleaf plants, foliage-
  card bushes, normal-mapped rocks, and a **streamed grass field**:
  blade tufts are precomputed per 16 m cell and only the cells around
  the player occupy the instance buffer — near-field density at a
  fixed GPU cost, one draw call.
- `game/buildings.ts`: block compounds in one InstancedMesh.
  Shooting removes blocks; a flood fill from the ground layer detaches
  anything unsupported into physics debris.
- `game/textures.ts`: every texture is generated on a canvas at boot
  (detail speckle, bark fiber, leaflets, foliage clusters, rock +
  derived Sobel normal maps). No downloads, no asset folder.

## Performance notes

- Vegetation, buildings, debris, and particles are all instanced or
  pooled. The whole world renders in about 50 draw calls.
- Grass streams around the player instead of existing island-wide.
- One shadow map, tight frustum, snapped to texels (no shimmer).
- HUD stats flow through a 4 Hz callback so React renders stay off the
  frame loop. The minimap base image renders once from the heightfield.

## Files

- `app.ts`: entities (`Avatar`, `Destruction`), policies, SSR routes,
  the `island` shard kind
- `shards/island`: the shard: joins, moves, hits, respawns (`cargo test`
  there runs its tests)
- `functions/`: `spawnAvatar` (idempotent, prunes stale rows),
  `joinIsland` (starts the shard, mints a ticket for your avatar),
  `destroyBlocks` (idempotent batch), `resetIsland`
- `app/page.tsx`: SSR shell, HUD, minimap, `<SyncBridge/>` feeding
  live queries into the engine
- `game/`: the engine: one system per file, composed in `game.ts`
