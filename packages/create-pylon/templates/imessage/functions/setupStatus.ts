import { query } from "@pylonsync/functions";
import { selfBaseUrl } from "../lib/agent-call";
import { providerStatus } from "../lib/provider";
import { findOwner } from "../lib/store";
import { isDevMode, isDryRun, selectedTransport, transportChecklist } from "../lib/transport";

// What the server is configured with, for the dashboard's setup panel.
// Reports which variables are present, never their values. Owner only.
export default query({
  auth: "admin",
  args: {},
  async handler(ctx) {
    const transport = selectedTransport(ctx.env);
    const provider = providerStatus(ctx.env);
    const owner = await findOwner(ctx);
    const base = selfBaseUrl(ctx.env);
    const devMode = isDevMode(ctx.env);
    return {
      transport,
      dryRun: isDryRun(ctx.env),
      checklist: transport ? transportChecklist(ctx.env, transport) : [],
      provider,
      ownerConfigured: owner !== null,
      adminEmailsSet: (ctx.env.PYLON_ADMIN_EMAILS ?? "").trim() !== "",
      adminTokenSet: (ctx.env.PYLON_ADMIN_TOKEN ?? "").trim() !== "",
      appUrlSet: (ctx.env.APP_URL ?? ctx.env.PYLON_PUBLIC_URL ?? "").trim() !== "",
      devMode,
      baseUrl: base,
      webhookUrl: `${base}/api/webhooks/sendblueWebhook`,
      relayUrl: base,
    };
  },
});
