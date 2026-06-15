/**
 * Pylon Sim — a co-op SimCity-style city builder.
 *
 * The lobby lists every city; you start a new one or join an existing one.
 * Within a city, every browser tab is a co-mayor: players paint roads and
 * zones, and the simulation grows buildings, population and tax income on
 * its own. All of it syncs — a road you lay or a tower that rises shows up
 * for everyone in that city, live.
 *
 * Two entities carry the whole game:
 *   - Tile  — one row per painted grid cell (road or R/C/I zone), scoped to
 *             a city by `cityId`. A zone tile's `level` is its building's
 *             growth stage, advanced server-side by the simulation tick.
 *   - City  — one row per city: a name, funds, population, jobs, the
 *             economic clock. Server-authoritative.
 */
import { buildManifest, discoverAppRoutes, entity, field, policy } from "@pylonsync/sdk";

const Tile = entity(
  "Tile",
  {
    // The city this tile belongs to (City.key). Every read/write is scoped
    // to one city, so multiple cities share the Tile table without bleeding.
    cityId: field.string(),
    gx: field.int(), // grid column
    gz: field.int(), // grid row
    // "road" | "res" | "com" | "ind"
    kind: field.string(),
    // Building growth stage for zone tiles (0 = empty lot, up to 3).
    // Roads stay 0. Written by placeTiles (0) and the sim tick.
    level: field.float(),
    // Who painted it (kept for stats / future per-player coloring).
    userId: field.string(),
    updatedAt: field.datetime(),
  },
  {
    // One tile per cell PER CITY — the unique index keeps placement
    // idempotent within a city and lets the planner upsert by (city,gx,gz).
    indexes: [{ name: "by_cell", fields: ["cityId", "gx", "gz"], unique: true }],
  },
);

const City = entity(
  "City",
  {
    key: field.string(), // unique city id / slug (one row per city)
    name: field.string(), // display name shown in the lobby
    funds: field.float(),
    population: field.float(),
    jobs: field.float(),
    happiness: field.float(),
    tick: field.float(), // monotonic sim-step counter
    nextTickAt: field.datetime(), // heartbeat for the self-perpetuating sim chain
    updatedAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_key", fields: ["key"], unique: true }],
  },
);

// Cities are co-op by design — every player reads and edits the shared maps.
// The client scopes its live query to one city (where: { cityId }); the read
// policy just gates on a session (guests get a userId from /api/auth/guest),
// so anonymous scrapers can't pull the maps.
const tilePolicy = policy({
  name: "tile_shared",
  entity: "Tile",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  // `level` is advanced only by the simulation tick (ctx.db.unsafe).
  allowUpdate: "false",
  allowDelete: "auth.userId != null",
});

// City funds/population are server-authoritative: only functions touch
// them (via ctx.db.unsafe), so a client can't mint money.
const cityPolicy = policy({
  name: "city_server_only",
  entity: "City",
  allowRead: "auth.userId != null",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "sim",
  version: "0.1.0",
  entities: [Tile, City],
  queries: [],
  actions: [],
  policies: [tilePolicy, cityPolicy],
  routes: await discoverAppRoutes(),
});

// Emit canonical manifest JSON to stdout for pylon codegen.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
