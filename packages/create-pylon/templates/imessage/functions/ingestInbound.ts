import { mutation, v } from "@pylonsync/functions";
import { ingestInbound } from "../lib/store";

// Store one verified inbound message and act on it (lib/store.ts ingestInbound).
// Internal: reachable only from sendblueWebhook after its secret check.
export default mutation({
  internal: true,
  auth: "admin",
  args: {
    transport: v.union(v.literal("sendblue"), v.literal("relay")),
    message: v.object({
      externalId: v.string(),
      handle: v.string(),
      text: v.string(),
      sentAt: v.string(),
      service: v.string(),
    }),
  },
  async handler(ctx, args) {
    return ingestInbound(ctx, args.message, args.transport);
  },
});
