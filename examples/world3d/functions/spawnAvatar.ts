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
 * Idempotent per-user avatar creation: the name and color other players
 * see. Poses and health live in the island shard (see joinIsland).
 * Existing rows are reused so refreshing the page keeps your identity.
 */
export default mutation({
  // Players are anonymous guest sessions — no signup screen in a demo.
  auth: "guest",
  args: {
    userId: v.string(),
    name: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (args.userId !== ctx.auth.userId) {
      throw ctx.error("FORBIDDEN", "can only spawn your own avatar");
    }

    // Prune avatars nobody has joined with for a day (closed tabs, old
    // guest sessions) so the table doesn't accumulate ghosts. joinIsland
    // sets lastSeenAt on every join.
    const cutoff = Date.now() - 24 * 60 * 60 * 1000;
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

    const existing = await ctx.db.query("Avatar", { userId: ctx.auth.userId });
    if (existing.length > 0) return { id: existing[0].id as string };

    const color = PALETTE[Math.floor(Math.random() * PALETTE.length)];
    const name = (args.name as string | undefined) ?? randomName();
    const id = await ctx.db.insert("Avatar", {
      userId: ctx.auth.userId,
      name,
      color,
      lastSeenAt: new Date().toISOString(),
    });
    return { id };
  },
});
