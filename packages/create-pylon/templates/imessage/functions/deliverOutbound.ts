import { action, v } from "@pylonsync/functions";
import { sendblueConfig, sendblueTransport } from "../lib/sendblue";
import { isDryRun } from "../lib/transport";

interface Claimed {
  leaseToken: string | null;
  messages: { id: string; handle: string; text: string }[];
}

// Send a conversation's queued Sendblue messages, oldest first, one at a time
// so bubbles arrive in order. claimOutbound leases the batch in a transaction
// under a token only this job holds, so two concurrent jobs for the same
// conversation never send a message twice.
// Scheduled by queueOutbound (lib/store.ts); internal.
export default action({
  internal: true,
  auth: "admin",
  timeout: 120,
  args: { conversationId: v.string() },
  async handler(ctx, args) {
    const claim = await ctx.runMutation<Claimed>("claimOutbound", { conversationId: args.conversationId });
    const batch = claim.messages;
    if (batch.length === 0 || !claim.leaseToken) return { sent: 0 };
    let failed = false;
    const transport = sendblueTransport(sendblueConfig(ctx.env, isDryRun(ctx.env)));
    let sent = 0;
    for (const message of batch) {
      const result = await transport.send({ messageId: message.id, to: message.handle, text: message.text });
      const settled = await ctx.runMutation<{ ok: boolean }>("settleOutbound", {
        messageId: message.id,
        leaseToken: claim.leaseToken,
        status: result.status === "sent" || result.status === "queued" ? "sent" : result.status,
        externalId: result.status === "sent" ? result.externalId : null,
        error: result.status === "failed" ? result.error : null,
      });
      // The lease was lost (expired and taken over): stop, the new holder owns the rest.
      if (!settled.ok) break;
      if (result.status === "failed") {
        failed = true;
        // Release the rest so a later attempt keeps the order intact.
        await ctx.runMutation("releaseOutbound", {
          leaseToken: claim.leaseToken,
          messageIds: batch.slice(batch.indexOf(message) + 1).map((m) => m.id),
        });
        break;
      }
      sent += 1;
    }
    // More may be waiting behind this batch.
    if (!failed && batch.length === 5) {
      await ctx.scheduler.runAfter(0, "deliverOutbound", { conversationId: args.conversationId });
    }
    return { sent };
  },
});
