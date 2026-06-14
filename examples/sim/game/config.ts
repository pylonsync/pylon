/**
 * Shared tunables for the city builder. Anything the server and client
 * must agree on (grid size, tile cost, building capacities) lives here
 * AND is mirrored in functions/ — functions/ files are standalone
 * endpoints and can't import this module, so keep the two in sync.
 */
import type { TileKind } from "./engine";

/** Grid is GRID×GRID cells, centred on the origin. */
export const GRID = 128;
/** World size of one cell, in metres. */
export const TILE = 4;
/** Half the world extent (for camera clamps). */
export const WORLD_HALF = (GRID * TILE) / 2;

/** Sea level. Terrain below this is water (rivers, lakes, ocean). */
export const WATER_LEVEL = 0;
/** Seed for the deterministic heightfield. */
export const TERRAIN_SEED = 0x5eed;

/** Max building growth stage for a zone tile. */
export const MAX_LEVEL = 3;

/** Per-tile placement cost (charged from the shared treasury). */
export const COST: Record<TileKind, number> = {
  road: 10,
  res: 20,
  com: 30,
  ind: 25,
  park: 0, // seeded greenspace, not player-placed
  carpark: 0, // seeded parking lot, not player-placed
};
/** Bulldoze refunds a fraction of the original cost. */
export const BULLDOZE_REFUND = 0.3;

export interface ZoneStyle {
  /** Lot tint shown before a building rises. */
  lot: number;
  /** Building body palette (picked per cell). */
  body: number[];
  /** UI accent + RCI bar colour. */
  accent: string;
  label: string;
}

/** Visual identity per zone. */
export const ZONE: Record<"res" | "com" | "ind", ZoneStyle> = {
  res: {
    lot: 0x3f6f3a,
    body: [0xd9b48a, 0xc97b5a, 0xe0c89a, 0xb5825f],
    accent: "#5ad05a",
    label: "Residential",
  },
  com: {
    lot: 0x3a5a6f,
    body: [0x6fa8d6, 0x86c2e6, 0x5a8fc0, 0x9ad0e0],
    accent: "#54b8e0",
    label: "Commercial",
  },
  ind: {
    lot: 0x6f6535,
    body: [0xd0b048, 0xc0a030, 0xe0c050, 0xb09028],
    accent: "#e0b73c",
    label: "Industrial",
  },
};

/** Population added per residential building at each level (index = level). */
export const RES_CAPACITY = [0, 8, 28, 70];
/** Jobs added per commercial / industrial building at each level. */
export const JOB_CAPACITY = [0, 6, 20, 52];

/** Sim tick cadence (ms) — the self-perpetuating cityTick chain. */
export const TICK_MS = 6000;
