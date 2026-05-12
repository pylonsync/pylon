/**
 * `@pylonsync/email` — declarative transactional email for Pylon
 * apps. Provider-agnostic (Resend, Postmark, SES, SMTP, log),
 * declarative template registry, suppression list, signed-webhook
 * delivery-event ingest.
 *
 * Usage:
 *
 * ```ts
 * import { email } from "@pylonsync/email";
 *
 * export const mail = email({
 *   provider: "resend",
 *   from: "Acme <hello@acme.com>",
 *   templates: {
 *     welcome: {
 *       subject: (v) => `Welcome, ${v.name}!`,
 *       render: (v) => ({
 *         html: `<h1>Hi ${v.name}</h1>`,
 *         text: `Hi ${v.name}`,
 *       }),
 *     },
 *   },
 *   hooks: {
 *     onBounced: async (ctx, { recipient }) => {
 *       // suppression already auto-added; emit audit event
 *     },
 *   },
 * });
 *
 * // In an action:
 * import { sendEmail } from "@pylonsync/email";
 * await sendEmail(ctx, mail.config, {
 *   template: "welcome",
 *   to: user.email,
 *   vars: { name: user.name },
 * });
 * ```
 */

import { sendEmail } from "./send";
import type { EmailConfig } from "./types";

export type {
	EmailConfig,
	EmailProviderName,
	EmailProvider,
	EmailTemplate,
	EmailVars,
	EmailSendArgs,
	EmailSendResult,
	EmailDeliveryEvent,
	EmailHooks,
	EmailCtx,
	SmtpConfig,
	ProviderSendArgs,
} from "./types";
export { sendEmail } from "./send";
export { getProvider } from "./providers";

export interface EmailPlugin {
	config: EmailConfig;
	send: typeof sendEmail;
}

export function email(cfg: EmailConfig): EmailPlugin {
	return {
		config: cfg,
		send: sendEmail,
	};
}
