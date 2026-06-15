/**
 * Shared starter-neighbourhood generator for new / reset cities.
 *
 * Returns grid cells WITHOUT a cityId — the caller scopes the inserts to its
 * own city. A grid of streets with zones lining them and block interiors left
 * open for trees, RANDOMISED per call (street spacing, district size, mix,
 * density) so every new city is a genuinely different layout. Lives under
 * `functions/lib/` so the function loader (which globs `functions/*.ts`) does
 * not mistake it for an endpoint.
 *
 * Constants mirror game/config.ts (functions can't import it).
 */
export type SeedCell = { gx: number; gz: number; kind: string; level: number };

export function seedTiles(): SeedCell[] {
  const seed = ((Math.random() * 0xffffffff) >>> 0) || 1;
  const rnd = (a: number, b: number): number => {
    let h = (Math.imul(a | 0, 73856093) ^ Math.imul(b | 0, 19349663) ^ seed) >>> 0;
    h = Math.imul(h ^ (h >>> 16), 2246822507) >>> 0;
    h = Math.imul(h ^ (h >>> 13), 3266489909) >>> 0;
    return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
  };
  const cells = new Map<string, SeedCell>();
  const set = (gx: number, gz: number, kind: string, level: number) => {
    const k = gx + "," + gz;
    if (!cells.has(k)) cells.set(k, { gx, gz, kind, level });
  };
  const C = 64;
  const R = 10 + Math.floor(rnd(7, 7) * 3); // 10..12
  const spacing = 3 + Math.floor(rnd(3, 9) * 3); // streets every 3..5 cells
  const density = 0.82 + rnd(5, 5) * 0.13; // fraction of road-served lots built
  const isRoad = (gx: number, gz: number) =>
    (gx - C) % spacing === 0 || (gz - C) % spacing === 0;
  // Every third grid line is a major arterial (avenue); the rest are streets,
  // so boulevards stay the minority — a clear hierarchy.
  const av = spacing * 3;
  const isAvenue = (gx: number, gz: number) =>
    (gx - C) % av === 0 || (gz - C) % av === 0;
  for (let gx = C - R; gx <= C + R; gx++) {
    for (let gz = C - R; gz <= C + R; gz++) {
      if (isRoad(gx, gz)) {
        set(gx, gz, isAvenue(gx, gz) ? "avenue" : "road", 0);
        continue;
      }
      const adj =
        isRoad(gx + 1, gz) || isRoad(gx - 1, gz) || isRoad(gx, gz + 1) || isRoad(gx, gz - 1);
      if (!adj) continue; // interior → trees
      const h = rnd(gx, gz);
      if (h > density) {
        // Open lot: a pocket park, a small parking lot, or left for trees.
        const open = rnd(gx * 3 + 1, gz * 3 + 1);
        if (open < 0.3) set(gx, gz, "park", 1);
        else if (open < 0.5) set(gx, gz, "carpark", 1);
        continue;
      }
      // A rare central civic landmark (city hall / clocktower).
      if (Math.hypot(gx - C, gz - C) < 6 && rnd(gx * 5 + 3, gz * 5 + 3) < 0.1) {
        set(gx, gz, "civic", 1);
        continue;
      }
      const k = rnd(gx * 7 + 1, gz * 13 + 1);
      const kind = k < 0.74 ? "res" : k < 0.9 ? "com" : "ind";
      set(gx, gz, kind, rnd(gx + 5, gz + 5) < 0.72 ? 1 : 2);
    }
  }
  return [...cells.values()];
}
