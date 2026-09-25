# Shard Arena

A realtime shard whose game logic is Rust compiled to WebAssembly. Each
player is a dot that moves to where they click. The stock `pylon` binary runs
the module; there is no custom server build.

- `shards/arena/src/lib.rs` is the simulation (`pylon-shard-guest`).
- `app.ts` declares the `arena` shard kind.
- `functions/joinArena.ts` starts the shard and mints a ticket.
- `app/ArenaIsland.tsx` joins with `useShard` and draws the snapshot.

## Run

```bash
rustup target add wasm32-unknown-unknown   # once
bun install
pylon dev
```

Open http://localhost:4321 in two windows. `pylon dev` builds
`shards/arena.wasm` at start and again when the Rust changes.

`shards/arena.wasm` is committed, so `pylon deploy` and a GitHub deploy ship
it without a Rust toolchain on the builder. Run `pylon shards build` after a
Rust change you want to deploy.

## Load test

With `pylon dev` running, put 300 bots in the arena:

```bash
pylon bench shard --join joinArena --bots 300 --input '"join"' \
  --input '{"move_to":{"x":"$rand:0:800","y":"$rand:0:500"}}'
```

See [Realtime shards](https://pylonsync.com/docs/concepts/shards).
