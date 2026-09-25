# Shard Arena

A realtime shard whose game logic is Rust compiled to WebAssembly. Each
player is a dot that moves to where they click. The stock `pylon` binary runs
the module; there is no custom server build.

- `shards/arena/src/lib.rs` is the simulation (`pylon-shard-guest`).
- `app.ts` declares the `arena` shard kind.
- `functions/joinArena.ts` starts the shard and mints a ticket.
- `app/ArenaIsland.tsx` joins with `useShard` and draws the snapshot.
- `shards/zone/src/lib.rs` is a second kind, `zone`: players with hit
  points, buffs, and cooldowns. `functions/joinZone.ts` starts a zone and
  `functions/moveZone.ts` moves the caller to another zone with its state
  (`ctx.shards.transfer`). `tools/smoke-shard-cluster.sh` moves a player
  between zones on two machines.

## Run

```bash
rustup target add wasm32-unknown-unknown   # once
bun install
pylon dev
```

Open http://localhost:4321 in two windows. `pylon dev` builds
`shards/arena.wasm` at start and again when the Rust changes.

`shards/arena.wasm` and `shards/zone.wasm` are committed, so `pylon deploy` and a GitHub deploy ship
it without a Rust toolchain on the builder. Run `pylon shards build` after a
Rust change you want to deploy.

## Load test

`frontier` is the kind for load tests at MMO scale: a 2000 × 2000 zone with
interest management (view radius 250), entity replication, and a 900 B/tick
byte budget. `size` in `--join-args` makes a small zone, where every player
is in view of the others:

```bash
pylon bench shard --join joinFrontier --bots 300 \
  --input '"join"' --input '{"move_to":{"x":"$rand:0:2000","y":"$rand:0:2000"}}' --input '"hit"'
pylon bench shard --join joinFrontier --join-args '{"frontier":"crowd","size":400}' --bots 300 \
  --input '"join"' --input '{"move_to":{"x":"$rand:0:400","y":"$rand:0:400"}}' --input '"hit"'
```

`arena` sends every player the full snapshot. With `pylon dev` running, put
300 bots in the arena:

```bash
pylon bench shard --join joinArena --bots 300 --input '"join"' \
  --input '{"move_to":{"x":"$rand:0:800","y":"$rand:0:500"}}'
```

See [Realtime shards](https://pylonsync.com/docs/concepts/shards).
