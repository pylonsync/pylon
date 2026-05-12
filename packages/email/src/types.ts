/**
 * `@pylonsync/email` — declarative transactional email for Pylon
 * apps. Replaces the dead `EmailPlugin` Rust shell with a TS layer
 * over Resend / Postmark / SES, plus a typed template registry,
 * suppression list, and signed webhook ingest for delivery events.
 *
 * Why TS instead of Rust:
 *   - Templates are React or simple HTML strings — apps already have
 *     a JSX renderer (next, vite, plain React-Email) in their TS
 *     tree, so passing rendered HTML is natural.
 *   - Providers ship official Node/Deno SDKs; reimplementing their
 *     HTTP APIs in Rust would be churn for little benefit.
 *   - Pylon's runtime exposes `ctx.email` already (a thin adapter
 *     hook), so wiring this layer to that hook means apps drop in
 *     a provider without forking the runtime.
 */

export type EmailProviderName = "resend" | "postmark" | "ses" | "smtp" | "log";

export interface EmailConfig {
	/**
	 * Provider selection. `log` prints the email to console.log
	 * (dev fallback when no real provider is configured — better
	 * than silently dropping the email).
	 */
	provider: EmailProviderName;
	/**
	 * Default `from` address. Apps that need per-template overrides
	 * set `from` on the template registration; this is just the
	 * fallback.
	 */
	from: string;
	/** Default `reply_to` if templates don't override. */
	replyTo?: string;
	/**
	 * Resolved at runtime from `ctx.env.<provider>_API_KEY`. Apps
	 * with multi-tenant providers (one Resend account per customer
	 * org, etc.) override this hook.
	 */
	getApiKey?: (ctx: EmailCtx) => string;
	/**
	 * SMTP-specific config when `provider: "smtp"`. Resolved from
	 * env in the default case.
	 */
	smtp?: SmtpConfig;
	/** Template registry — required keys app code references. */
	templates: Record<string, EmailTemplate<EmailVars>>;
	/**
	 * Suppression check. Called before every send. Return true to
	 * skip delivery (recipient is on the bounce/complain list).
	 * Default implementation queries an internal `EmailSuppression`
	 * entity if the manifest contributes one.
	 */
	isSuppressed?: (
		ctx: EmailCtx,
		args: { email: string; reason?: string },
	) => Promise<boolean> | boolean;
	/**
	 * Per-account rate limit. The default rate-limit plugin handles
	 * route-level rate limiting; this is a per-recipient cap so a
	 * runaway loop can't email-bomb one address.
	 */
	perRecipientRateLimit?: { count: number; windowSecs: number };
	/**
	 * Webhook secret for delivery events (Resend signs with Svix-
	 * style headers; Postmark with `X-Postmark-Signature`; SES via
	 * SNS). Set per-provider; the `emailWebhook` handler picks the
	 * right verifier based on `cfg.provider`.
	 */
	webhookSecret?: string;
	/** Lifecycle hooks fired on delivery events. */
	hooks?: EmailHooks;
}

export interface SmtpConfig {
	host: string;
	port: number;
	username?: string;
	password?: string;
	secure?: boolean;
}

/**
 * One template — apps register a static catalog at config time, then
 * call `email.send({template: 'invite', to, vars: {...}})`.
 */
export interface EmailTemplate<V extends EmailVars = EmailVars> {
	subject: string | ((vars: V) => string);
	/**
	 * Body. Either:
	 *   - `text`: plain-text only (most reliable, no rendering deps)
	 *   - `html`: pre-rendered HTML string (caller renders React/JSX)
	 *   - `render(vars)`: callback that returns `{ html, text }`
	 *     given the variables. Use this for templated content; the
	 *     plugin captures the rendered output once per send.
	 */
	text?: string | ((vars: V) => string);
	html?: string | ((vars: V) => string);
	render?: (vars: V) => Promise<{ html?: string; text?: string }> | { html?: string; text?: string };
	from?: string;
	replyTo?: string;
	/** Optional per-template tags (for analytics/segmentation). */
	tags?: Record<string, string>;
}

export type EmailVars = Record<string, string | number | boolean | null | undefined>;

export interface EmailHooks {
	/** Fires before every send; can throw to abort. */
	beforeSend?: (ctx: EmailCtx, send: EmailSendArgs) => Promise<void> | void;
	/** Fires after a provider returns a successful send response. */
	onSent?: (
		ctx: EmailCtx,
		args: { messageId: string; send: EmailSendArgs },
	) => Promise<void> | void;
	/** Fires when the webhook handler ingests a `delivered` event. */
	onDelivered?: (
		ctx: EmailCtx,
		args: { messageId: string; recipient: string; deliveredAt: string },
	) => Promise<void> | void;
	/** Fires when the webhook ingests a `bounced` event. Default
	 *  behavior also adds the recipient to the suppression list. */
	onBounced?: (
		ctx: EmailCtx,
		args: { messageId: string; recipient: string; reason: string },
	) => Promise<void> | void;
	/** Fires on `complained` (spam complaint). Also suppresses. */
	onComplained?: (
		ctx: EmailCtx,
		args: { messageId: string; recipient: string },
	) => Promise<void> | void;
	/** Fires on `opened` event. Many providers gate this on tracking
	 *  pixels which require explicit opt-in per region (GDPR). */
	onOpened?: (
		ctx: EmailCtx,
		args: { messageId: string; recipient: string },
	) => Promise<void> | void;
	/** Fires on `clicked`. Same tracking-pixel caveat as `opened`. */
	onClicked?: (
		ctx: EmailCtx,
		args: { messageId: string; recipient: string; url: string },
	) => Promise<void> | void;
}

export interface EmailSendArgs {
	template: string;
	to: string | string[];
	vars?: EmailVars;
	cc?: string | string[];
	bcc?: string | string[];
	replyTo?: string;
	/**
	 * Idempotency key — providers dedup on this within 24h. Defaults
	 * to `${template}:${to}:${hash(vars)}` so apps that retry on
	 * transient failure don't send duplicates.
	 */
	idempotencyKey?: string;
	/** Optional scheduled-send timestamp (ISO 8601). */
	scheduledAt?: string;
	/** Provider-specific extras (attachments, custom headers). */
	extra?: Record<string, unknown>;
}

export interface EmailSendResult {
	messageId: string;
	suppressed?: boolean;
}

/**
 * Provider adapter interface. Add new providers by implementing
 * this and registering in `providers/index.ts`. Each implementation
 * is free to use the official SDK or raw fetch — the plugin doesn't
 * care, just needs `send(args)` to resolve with a message id.
 */
export interface EmailProvider {
	name: EmailProviderName;
	send(args: ProviderSendArgs): Promise<EmailSendResult>;
	verifyWebhook?(
		secret: string,
		headers: Record<string, string | undefined>,
		rawBody: string,
	): Promise<true | string>;
	parseWebhookEvent?(rawBody: string): EmailDeliveryEvent | null;
}

export interface ProviderSendArgs {
	apiKey: string;
	from: string;
	to: string[];
	cc?: string[];
	bcc?: string[];
	replyTo?: string;
	subject: string;
	html?: string;
	text?: string;
	tags?: Record<string, string>;
	idempotencyKey?: string;
	scheduledAt?: string;
	extra?: Record<string, unknown>;
	smtp?: SmtpConfig;
}

export interface EmailDeliveryEvent {
	type:
		| "sent"
		| "delivered"
		| "bounced"
		| "complained"
		| "opened"
		| "clicked"
		| "failed"
		| "delayed";
	messageId: string;
	recipient: string;
	occurredAt: string;
	reason?: string;
	url?: string;
}

/**
 * Subset of ActionCtx the email handlers need. Same pattern as
 * @pylonsync/stripe — keeps the package from tightly coupling to
 * the full @pylonsync/functions ctx type.
 */
export interface EmailCtx {
	env: Record<string, string | undefined>;
	auth: { userId?: string | null; tenantId?: string | null; isAdmin?: boolean };
	request?: {
		headers: Record<string, string | undefined>;
		rawBody: string;
	};
	runQuery: <T>(name: string, args: Record<string, unknown>) => Promise<T>;
	runMutation: <T = unknown>(
		name: string,
		args: Record<string, unknown>,
	) => Promise<T>;
	error: (code: string, message: string) => Error;
}
