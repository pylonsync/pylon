import { mutation, v } from "@pylonsync/functions";

/**
 * Paint a texture layer onto the splatmap.
 *
 * Layer index 0..3 maps to grass / dirt / rock / snow by convention.
 * The brush adds `strength * falloff` to the target layer's weight and
 * subtracts proportionally from the others so the 4 weights always sum
 * close to 1 (we normalize at the end to avoid drift).
 */
export default mutation({
  auth: "guest",
  args: {
    roomId: v.string(),
    cx: v.number(),
    cz: v.number(),
    radius: v.number(),
    strength: v.number(),
    layer: v.int(), // 0..3
  },
  async handler(ctx, args) {
    const a = args as {
      roomId: string;
      cx: number;
      cz: number;
      radius: number;
      strength: number;
      layer: number;
    };
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (a.layer < 0 || a.layer > 3) {
      throw ctx.error("INVALID_ARGS", "layer must be 0..3");
    }

    const rows = await ctx.db.query("Terrain", { roomId: a.roomId });
    if (rows.length === 0) throw ctx.error("NOT_FOUND", "terrain not initialized");
    const terrain = rows[0];
    const size = terrain.size as number;
    const layers: number[][][] = JSON.parse(terrain.layers as string);

    const r = Math.max(1, a.radius);
    const xMin = Math.max(0, Math.floor(a.cx - r));
    const xMax = Math.min(size - 1, Math.ceil(a.cx + r));
    const zMin = Math.max(0, Math.floor(a.cz - r));
    const zMax = Math.min(size - 1, Math.ceil(a.cz + r));

    for (let z = zMin; z <= zMax; z++) {
      for (let x = xMin; x <= xMax; x++) {
        const dx = x - a.cx;
        const dz = z - a.cz;
        const d = Math.hypot(dx, dz);
        if (d > r) continue;
        const falloff = 0.5 * (1 + Math.cos((Math.PI * d) / r));
        const add = a.strength * falloff;

        const cell = layers[z][x];
        cell[a.layer] = Math.min(1, cell[a.layer] + add);

        // Normalize so weights sum to 1.
        const sum = cell[0] + cell[1] + cell[2] + cell[3];
        if (sum > 0) {
          cell[0] /= sum;
          cell[1] /= sum;
          cell[2] /= sum;
          cell[3] /= sum;
        }
      }
    }

    await ctx.db.update("Terrain", terrain.id as string, {
      layers: JSON.stringify(layers),
      updatedAt: new Date().toISOString(),
    });

    return { ok: true };
  },
});
