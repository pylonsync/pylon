import { action, v } from "@pylonsync/functions";
import {
  MAX_INBOUND_PER_SYNC,
  parseRelayInbound,
  type RelaySyncResponse,
} from "../lib/relay-protocol";
import { bearerToken, checkSecret, headerValue } from "../lib/secrets";
import { selectedTransport, type InboundMessage } from "../lib/transport";

// The Mac relay's single endpoint (see lib/relay-protocol.ts for the contract).
// Public by necessity; the handler rejects the call unless the bearer token
// matches RELAY_TOKEN (constant-time), and only then elevates to write.
export default action({
  auth: "public",
  args: {
    relayVersion: v.string(),
    host: v.string(),
    inbound: v.array(v.any()),
    acks: v.array(
      v.object({
        id: v.string(),
        ok: v.boolean(),
        error: v.optional(v.string()),
        dryRun: v.optional(v.boolean()),
      }),
    ),
  },
  async handler(ctx, args): Promise<RelaySyncResponse> {
    if (selectedTransport(ctx.env) !== "relay") {
      throw ctx.error("TRANSPORT_DISABLED", "The relay transport is not enabled on this server");
    }
    const presented = bearerToken(headerValue(ctx.request?.headers, "authorization"));
    const check = checkSecret(presented, ctx.env.RELAY_TOKEN);
    if (!check.ok) {
      if (check.reason === "not_configured") throw ctx.error("NOT_CONFIGURED", "RELAY_TOKEN is not set");
      throw ctx.error("UNAUTHORIZED", "Relay token missing or invalid");
    }
    if (args.inbound.length > MAX_INBOUND_PER_SYNC || args.acks.length > 100) {
      throw ctx.error("TOO_LARGE", `Send at most ${MAX_INBOUND_PER_SYNC} messages per sync`);
    }

    const messages: InboundMessage[] = [];
    // Items the server deliberately skips (group chats, attachments without
    // text) still count as accepted, so the relay's watermark moves past them.
    const skipped: string[] = [];
    for (const item of args.inbound) {
      const parsed = parseRelayInbound(item);
      if ("message" in parsed) messages.push(parsed.message);
      else if (typeof (item as { guid?: unknown })?.guid === "string") {
        skipped.push(String((item as { guid: string }).guid).slice(0, 128));
      }
    }

    await ctx.auth.elevate({ admin: true, reason: "relay token verified" });
    return ctx.runMutation<RelaySyncResponse>("relayApply", {
      relayVersion: args.relayVersion.slice(0, 32),
      host: args.host.slice(0, 64),
      messages,
      skipped,
      acks: args.acks.map((a) => ({ id: a.id, ok: a.ok, error: a.error?.slice(0, 300), dryRun: a.dryRun === true })),
    });
  },
});
