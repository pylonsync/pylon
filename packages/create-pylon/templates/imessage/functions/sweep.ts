import { mutation } from "@pylonsync/functions";
import { TURN_STALE_MS } from "../lib/store";

// Every five minutes (app.ts crons):
//   - release turns whose worker died so the conversation can answer again,
//     and mark their texts failed;
//   - restart Sendblue delivery for conversations with messages still queued
//     or leased past expiry (the delivery job died). Relay leases need no
//     sweep: the relay's next sync picks expired ones up.
// Internal.
export default mutation({
  internal: true,
  args: {},
  async handler(ctx) {
    await ctx.auth.elevate({ admin: true, reason: "scheduled sweep of stuck agent turns" });
    const cutoff = new Date(Date.now() - TURN_STALE_MS).toISOString();
    const running = await ctx.db.query("Conversation", { turnState: "running", $limit: 100 });
    let released = 0;
    for (const conversation of running) {
      if (String(conversation.turnStartedAt ?? "") > cutoff) continue;
      const stuck = await ctx.db.query("Message", {
        conversationId: String(conversation.id),
        status: "processing",
      });
      for (const m of stuck) {
        await ctx.db.update("Message", String(m.id), {
          status: "failed",
          detail: "The reply took too long and was abandoned",
        });
      }
      await ctx.db.update("Conversation", String(conversation.id), {
        turnState: "idle",
        turnId: null,
        turnStartedAt: null,
        lastError: "A reply timed out",
        lastErrorAt: new Date().toISOString(),
      });
      released += 1;
    }
    const stale = new Date(Date.now() - 60_000).toISOString();
    const pending = await ctx.db.query("Message", {
      status: { $in: ["queued", "leased"] },
      createdAt: { $lt: stale },
      $limit: 500,
    });
    const restart = new Set<string>();
    const nowIso = new Date().toISOString();
    for (const m of pending) {
      if (m.transport !== "sendblue" || m.direction !== "out") continue;
      if (m.status === "leased" && String(m.leaseUntil ?? "") > nowIso) continue;
      restart.add(String(m.conversationId));
    }
    for (const conversationId of restart) {
      await ctx.scheduler.runAfter(0, "deliverOutbound", { conversationId });
    }
    return { released, deliveriesRestarted: restart.size };
  },
});
