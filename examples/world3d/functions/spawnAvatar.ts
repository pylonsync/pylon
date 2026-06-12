import { mutation, v } from "@pylonsync/functions";

const PALETTE = [
  "#8b5cf6", "#f5b946", "#7ab7ff", "#5ee6a6",
  "#ff6b9d", "#ffd166", "#80e0d8", "#c89dff",
];

const NAMES = [
  "nova", "onyx", "echo", "lyra", "atlas", "rhea",
  "orion", "vega", "juno", "mira", "zed", "kai",
];

function randomName() {
  return `${NAMES[Math.floor(Math.random() * NAMES.length)]}_${Math.floor(Math.random() * 900 + 100)}`;
}

// Mirror of the constants in rebuildBuildings.ts (functions/ files are
// standalone endpoints — they can't share a module).
const SWEEP_MS = 10 * 60 * 1000;
const MARKER_KEY = "buildingSweep";

/**
 * Idempotent per-user avatar creation. The client computes the actual
 * spawn position (it knows the terrain heights); the server only
 * validates bounds. Existing rows are reused so refreshing the page
 * keeps your identity.
 */
export default mutation({
  // Players are anonymous guest sessions — no signup screen in a demo.
  auth: "guest",
  args: {
    userId: v.string(),
    name: v.optional(v.string()),
    x: v.optional(v.number()),
    y: v.optional(v.number()),
    z: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    // Prune avatars that haven't moved in 10 minutes (closed tabs,
    // stale rows from older schema versions) so the island doesn't
    // accumulate ghosts.
    const cutoff = Date.now() - 10 * 60 * 1000;
    const all = await ctx.db.query("Avatar", {});
    for (const row of all) {
      const seen = Date.parse((row.lastSeenAt as string) ?? "");
      if (Number.isNaN(seen) || seen < cutoff) {
        // unsafe: pruning OTHER players' stale rows — the Avatar
        // delete policy is owner-only, and this is the sanctioned
        // janitor sweep.
        await ctx.db.unsafe.delete("Avatar", row.id as string);
      }
    }

    // Make sure the building-rebuild chain is alive (see
    // rebuildBuildings.ts). The heartbeat row records when the next
    // sweep is due; missing or >60 s overdue means the chain died
    // (fresh database, wiped job store) — start a new one.
    await ctx.db.advisoryLock("world3d.buildingSweep");
    const marker = await ctx.db.lookup("World", "key", MARKER_KEY);
    const due = marker ? new Date(marker.nextRunAt as string).getTime() : NaN;
    if (!Number.isFinite(due) || due < Date.now() - 60_000) {
      const nextRunAt = new Date(Date.now() + SWEEP_MS).toISOString();
      // unsafe: World is policy-locked to server functions.
      if (marker) {
        await ctx.db.unsafe.update("World", marker.id as string, { nextRunAt });
      } else {
        await ctx.db.unsafe.insert("World", { key: MARKER_KEY, nextRunAt });
      }
      await ctx.scheduler.runAfter(SWEEP_MS, "rebuildBuildings", {});
    }

    const existing = await ctx.db.query("Avatar", { userId: args.userId });
    if (existing.length > 0) return { id: existing[0].id as string };

    const clamp = (n: number) => Math.max(-250, Math.min(250, n));
    const color = PALETTE[Math.floor(Math.random() * PALETTE.length)];
    const name = (args.name as string | undefined) ?? randomName();
    const id = await ctx.db.insert("Avatar", {
      userId: args.userId,
      name,
      color,
      x: clamp((args.x as number | undefined) ?? 0),
      y: (args.y as number | undefined) ?? 0,
      z: clamp((args.z as number | undefined) ?? 0),
      heading: Math.random() * Math.PI * 2,
      pitch: 0,
      health: 100,
      lastSeenAt: new Date().toISOString(),
    });
    return { id };
  },
});
