/**
 * Log provider — dev-only fallback that prints emails to console
 * instead of dispatching them. Set `provider: "log"` (or rely on
 * the env-driven fallback when no real provider config is present)
 * to avoid silently dropping emails during local development.
 *
 * Behavior intentionally mirrors the framework's old `EmailPlugin`
 * dev mode so existing dev flows don't change shape when apps swap
 * in this package.
 */

import type {
	EmailProvider,
	EmailSendResult,
	ProviderSendArgs,
} from "../types";

export const logProvider: EmailProvider = {
	name: "log",

	async send(args: ProviderSendArgs): Promise<EmailSendResult> {
		// eslint-disable-next-line no-console
		console.log(
			JSON.stringify(
				{
					$: "@pylonsync/email log provider",
					from: args.from,
					to: args.to,
					subject: args.subject,
					text: args.text?.slice(0, 500),
					html: args.html?.slice(0, 500),
					tags: args.tags,
				},
				null,
				2,
			),
		);
		// Synthetic message id so onSent hooks still get a unique
		// reference even though no real provider id exists.
		return {
			messageId: `log_${Date.now().toString(36)}_${Math.random()
				.toString(36)
				.slice(2, 8)}`,
		};
	},
};
