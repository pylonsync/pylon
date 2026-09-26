import { action, v } from "@pylonsync/functions";
import { parseSendblueInbound, sendblueConfig, verifySendblueWebhook } from "../lib/sendblue";
import { isDryRun, selectedTransport } from "../lib/transport";

// Sendblue's receive webhook. Point Sendblue at:
//   https://<your app>/api/webhooks/sendblueWebhook
// with the same secret as SENDBLUE_WEBHOOK_SECRET.
//
// Public by necessity (Sendblue has no session). The handler rejects the
// request unless the `sb-signing-secret` header matches the configured secret,
// and only then elevates to store the message.
export default action({
  auth: "public",
  // /api/webhooks/* passes { rawBody }; the handler always reads the raw
  // request so the body it parses is the one that arrived with the header.
  args: { rawBody: v.optional(v.string()) },
  async handler(ctx) {
    if (selectedTransport(ctx.env) !== "sendblue") {
      throw ctx.error("TRANSPORT_DISABLED", "The Sendblue transport is not enabled on this server");
    }
    const request = ctx.request;
    if (!request) throw ctx.error("BAD_REQUEST", "This endpoint only accepts HTTP webhook deliveries");

    const config = sendblueConfig(ctx.env, isDryRun(ctx.env));
    const check = verifySendblueWebhook(request.headers, config.webhookSecret);
    if (!check.ok) {
      if (check.reason === "not_configured") {
        throw ctx.error("NOT_CONFIGURED", "SENDBLUE_WEBHOOK_SECRET is not set");
      }
      throw ctx.error("UNAUTHORIZED", "Webhook secret missing or invalid");
    }

    let body: unknown;
    try {
      body = JSON.parse(request.rawBody);
    } catch {
      throw ctx.error("INVALID_JSON", "Webhook body is not JSON");
    }
    const parsed = parseSendblueInbound(body, config.fromNumber);
    if ("skip" in parsed) return { ok: true, skipped: parsed.skip };

    await ctx.auth.elevate({ admin: true, reason: "sendblue webhook secret verified" });
    const result = await ctx.runMutation<{ status: string }>("ingestInbound", {
      transport: "sendblue",
      message: parsed.message,
    });
    return { ok: true, status: result.status };
  },
});
