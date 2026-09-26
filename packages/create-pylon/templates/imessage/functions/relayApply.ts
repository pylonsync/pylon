import { mutation, v } from "@pylonsync/functions";
import { MAX_OUTBOUND_PER_SYNC, MAX_RELAY_ATTEMPTS, OUTBOUND_LEASE_MS } from "../lib/relay-protocol";
import { ingestInbound, touchTransport } from "../lib/store";

// One relay sync, in one transaction: record the heartbeat, settle the acks
// for replies the relay sent, store new inbound messages, and lease the next
// batch of queued replies. Internal: reachable only from relaySync after its
// token check.
export default mutation({
  internal: true,
  auth: "admin",
  args: {
    relayVersion: v.string(),
    host: v.string(),
    messages: v.array(
      v.object({
        externalId: v.string(),
        handle: v.string(),
        text: v.string(),
        sentAt: v.string(),
        service: v.string(),
      }),
    ),
    skipped: v.array(v.string()),
    acks: v.array(
      v.object({ id: v.string(), ok: v.boolean(), error: v.optional(v.string()), dryRun: v.optional(v.boolean()) }),
    ),
  },
  async handler(ctx, args) {
    const now = new Date();
    const nowIso = now.toISOString();
    const statePatch: Record<string, unknown> = {
      relayLastSeenAt: nowIso,
      relayVersion: args.relayVersion,
      relayHost: args.host,
    };

    for (const ack of args.acks) {
      const row = await ctx.db.get("Message", ack.id);
      // Only a leased relay reply can be settled; anything else is stale.
      if (!row || row.direction !== "out" || row.transport !== "relay" || row.status !== "leased") continue;
      if (ack.ok && ack.dryRun) {
        await ctx.db.update("Message", ack.id, {
          status: "dry_run",
          sentAt: nowIso,
          leaseUntil: null,
          detail: "RELAY_DRY_RUN is on; the relay did not send it.",
        });
      } else if (ack.ok) {
        await ctx.db.update("Message", ack.id, { status: "sent", sentAt: nowIso, leaseUntil: null, detail: null });
        statePatch.lastOutboundAt = nowIso;
      } else {
        const error = ack.error || "The relay could not send this message";
        await ctx.db.update("Message", ack.id, { status: "failed", detail: error, leaseUntil: null });
        statePatch.lastError = error;
        statePatch.lastErrorAt = nowIso;
      }
    }

    const accepted: string[] = [...args.skipped];
    for (const message of args.messages) {
      await ingestInbound(ctx, message, "relay");
      // "dropped" is accepted too: the server decided not to store it, and
      // resending the same row would get the same answer.
      accepted.push(message.externalId.replace(/^relay:/, ""));
    }

    // Queued replies plus leases the relay let expire, oldest first.
    const queued = await ctx.db.query("Message", {
      status: { $in: ["queued", "leased"] },
      $order: { createdAt: "asc" },
      $limit: 200,
    });
    const outbound: { id: string; handle: string; text: string }[] = [];
    const leaseUntil = new Date(now.getTime() + OUTBOUND_LEASE_MS).toISOString();
    for (const row of queued) {
      if (outbound.length >= MAX_OUTBOUND_PER_SYNC) break;
      if (row.transport !== "relay" || row.direction !== "out") continue;
      if (row.status === "leased" && String(row.leaseUntil ?? "") > nowIso) continue;
      const attempts = Number(row.attempts) || 0;
      if (attempts >= MAX_RELAY_ATTEMPTS) {
        await ctx.db.update("Message", String(row.id), {
          status: "failed",
          leaseUntil: null,
          detail: `The relay did not confirm delivery after ${attempts} attempts`,
        });
        continue;
      }
      const conversation = await ctx.db.get("Conversation", String(row.conversationId));
      if (!conversation) continue;
      await ctx.db.update("Message", String(row.id), {
        status: "leased",
        leaseUntil,
        attempts: attempts + 1,
      });
      outbound.push({ id: String(row.id), handle: String(conversation.handle), text: String(row.text) });
    }

    await touchTransport(ctx, "relay", statePatch);
    return { accepted, outbound };
  },
});
