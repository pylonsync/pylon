import { mutation, v } from "@pylonsync/functions";

/**
 * Idempotent terrain seed. Called on first room join — if a Terrain row
 * already exists for the room, returns its id; otherwise creates a flat
 * heightmap with layer 0 (grass) at weight 1 everywhere else zero.
 *
 * Default size = 64 cells per edge. Bumping this to 128 quadruples the
 * payload; for the demo 64 strikes a balance between looking like real
 * terrain and staying under ~35 KB per update.
 */
export default mutation({
  args: {
    roomId: v.string(),
    size: v.optional(v.int()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const existing = await ctx.db.query("Terrain", { roomId: args.roomId });
    if (existing.length > 0) return { id: existing[0].id as string, created: false };

    const size = args.size ?? 64;

    // Flat heightmap
    const heights: number[][] = Array.from({ length: size }, () =>
      Array.from({ length: size }, () => 0),
    );

    // 4-layer splatmap: layer 0 = full (grass), rest zero.
    // Stored as [size][size][4] so the client can sample per-vertex.
    const layers: number[][][] = Array.from({ length: size }, () =>
      Array.from({ length: size }, () => [1, 0, 0, 0]),
    );

    const id = await ctx.db.insert("Terrain", {
      roomId: args.roomId,
      size,
      heights: JSON.stringify(heights),
      layers: JSON.stringify(layers),
      updatedAt: new Date().toISOString(),
    });

    return { id, created: true };
  },
});
