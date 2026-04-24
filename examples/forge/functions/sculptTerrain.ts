import { mutation, v } from "@pylonsync/functions";

/**
 * Apply a brush stroke to the heightmap.
 *
 * Modes:
 *   raise    — add `strength * falloff` to heights under brush
 *   lower    — subtract same
 *   smooth   — nudge each cell toward the brush-region average
 *   flatten  — pull heights toward a target Y (default 0)
 *
 * `cx`/`cz` are cell coordinates (0..size-1), `radius` is in cells.
 * The falloff is a smooth cosine from brush center to edge so strokes
 * don't leave hard rings.
 *
 * Client throttles strokes to 10 Hz; each call reads + rewrites the whole
 * heights JSON. For a 64×64 grid that's ~35 KB of payload per stroke.
 */
export default mutation({
  args: {
    roomId: v.string(),
    cx: v.float(),
    cz: v.float(),
    radius: v.float(),
    strength: v.float(),
    mode: v.string(), // "raise" | "lower" | "smooth" | "flatten"
    targetY: v.optional(v.float()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const rows = await ctx.db.query("Terrain", { roomId: args.roomId });
    if (rows.length === 0) throw ctx.error("NOT_FOUND", "terrain not initialized");
    const terrain = rows[0];
    const size = terrain.size as number;
    const heights: number[][] = JSON.parse(terrain.heights as string);

    const r = Math.max(1, args.radius);
    const xMin = Math.max(0, Math.floor(args.cx - r));
    const xMax = Math.min(size - 1, Math.ceil(args.cx + r));
    const zMin = Math.max(0, Math.floor(args.cz - r));
    const zMax = Math.min(size - 1, Math.ceil(args.cz + r));

    // Compute brush-area average for smooth mode in one pass.
    let avg = 0;
    let count = 0;
    if (args.mode === "smooth") {
      for (let z = zMin; z <= zMax; z++) {
        for (let x = xMin; x <= xMax; x++) {
          const dx = x - args.cx;
          const dz = z - args.cz;
          if (Math.hypot(dx, dz) <= r) {
            avg += heights[z][x];
            count++;
          }
        }
      }
      if (count > 0) avg /= count;
    }

    const target = args.targetY ?? 0;

    for (let z = zMin; z <= zMax; z++) {
      for (let x = xMin; x <= xMax; x++) {
        const dx = x - args.cx;
        const dz = z - args.cz;
        const d = Math.hypot(dx, dz);
        if (d > r) continue;
        const falloff = 0.5 * (1 + Math.cos((Math.PI * d) / r));
        const s = args.strength * falloff;

        switch (args.mode) {
          case "raise":
            heights[z][x] += s;
            break;
          case "lower":
            heights[z][x] -= s;
            break;
          case "smooth":
            heights[z][x] += (avg - heights[z][x]) * Math.min(1, s);
            break;
          case "flatten":
            heights[z][x] += (target - heights[z][x]) * Math.min(1, s);
            break;
        }
      }
    }

    await ctx.db.update("Terrain", terrain.id as string, {
      heights: JSON.stringify(heights),
      updatedAt: new Date().toISOString(),
    });

    return { ok: true };
  },
});
