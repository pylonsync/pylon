/**
 * The runtime send path. Apps call this directly from action
 * handlers (`await sendEmail(ctx, cfg, {template, to, vars})`) or
 * via the public `email.send` action factory.
 *
 * The send path does:
 *   1. Resolve the template (string template + render callback)
 *   2. Resolve the API key for the configured provider
 *   3. Suppression check (calls cfg.isSuppressed; default queries
 *      the EmailSuppression entity if one's been declared)
 *   4. beforeSend hook (apps can mutate or throw)
 *   5. Provider .send() with idempotency key + tags
 *   6. onSent hook fires async (don't await — the response is back
 *      to the caller before the hook resolves)
 */

import { getProvider } from "./providers";
import type {
	EmailConfig,
	EmailCtx,
	EmailSendArgs,
	EmailSendResult,
	EmailTemplate,
	EmailVars,
	ProviderSendArgs,
} from "./types";

export async function sendEmail(
	ctx: EmailCtx,
	cfg: EmailConfig,
	args: EmailSendArgs,
): Promise<EmailSendResult> {
	const tpl = cfg.templates[args.template] as EmailTemplate | undefined;
	if (!tpl) {
		throw ctx.error(
			"UNKNOWN_TEMPLATE",
			`template "${args.template}" not registered`,
		);
	}
	const recipients = Array.isArray(args.to) ? args.to : [args.to];
	if (recipients.length === 0) {
		throw ctx.error("NO_RECIPIENTS", "to: must include at least one address");
	}

	if (cfg.isSuppressed) {
		// Single-recipient sends short-circuit at the first hit;
		// multi-recipient sends filter the suppression list and
		// continue if any survive.
		const survivors: string[] = [];
		for (const r of recipients) {
			const suppressed = await cfg.isSuppressed(ctx, { email: r });
			if (!suppressed) survivors.push(r);
		}
		if (survivors.length === 0) return { messageId: "", suppressed: true };
		recipients.splice(0, recipients.length, ...survivors);
	}

	const vars = (args.vars ?? {}) as EmailVars;
	const rendered = await renderTemplate(tpl, vars);

	if (cfg.hooks?.beforeSend) {
		await cfg.hooks.beforeSend(ctx, args);
	}

	const apiKey = resolveApiKey(ctx, cfg);
	const provider = getProvider(cfg.provider);
	const providerArgs: ProviderSendArgs = {
		apiKey,
		from: tpl.from ?? cfg.from,
		replyTo: args.replyTo ?? tpl.replyTo ?? cfg.replyTo,
		to: recipients,
		cc: normalize(args.cc),
		bcc: normalize(args.bcc),
		subject: rendered.subject,
		html: rendered.html,
		text: rendered.text,
		tags: tpl.tags,
		idempotencyKey: args.idempotencyKey ?? deriveIdemKey(args),
		scheduledAt: args.scheduledAt,
		extra: args.extra,
		smtp: cfg.smtp,
	};
	const result = await provider.send(providerArgs);

	if (cfg.hooks?.onSent) {
		// Fire-and-forget so the hook can do slow side effects (write
		// to audit_log, etc) without blocking the action response.
		void Promise.resolve(
			cfg.hooks.onSent(ctx, { messageId: result.messageId, send: args }),
		).catch(() => {});
	}

	return result;
}

async function renderTemplate(
	tpl: EmailTemplate,
	vars: EmailVars,
): Promise<{ subject: string; html?: string; text?: string }> {
	const subject =
		typeof tpl.subject === "function" ? tpl.subject(vars) : tpl.subject;
	if (tpl.render) {
		const r = await tpl.render(vars);
		return { subject, html: r.html, text: r.text };
	}
	const html =
		typeof tpl.html === "function" ? tpl.html(vars) : tpl.html;
	const text =
		typeof tpl.text === "function" ? tpl.text(vars) : tpl.text;
	if (!html && !text) {
		throw new Error(
			`template "${tpl.subject.toString()}" has neither html, text, nor render() — must provide at least one`,
		);
	}
	return { subject, html, text };
}

function normalize(v: string | string[] | undefined): string[] | undefined {
	if (!v) return undefined;
	return Array.isArray(v) ? v : [v];
}

function deriveIdemKey(args: EmailSendArgs): string {
	const to = Array.isArray(args.to) ? args.to.join(",") : args.to;
	const varsHash = args.vars ? hashCheap(JSON.stringify(args.vars)) : "0";
	return `${args.template}:${to}:${varsHash}`;
}

function hashCheap(s: string): string {
	// Cheap non-cryptographic hash — only used as idempotency-key
	// suffix to make retries dedup. Good enough; collisions across
	// templates+recipients are negligible.
	let h = 5381;
	for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
	return (h >>> 0).toString(36);
}

function resolveApiKey(ctx: EmailCtx, cfg: EmailConfig): string {
	if (cfg.getApiKey) return cfg.getApiKey(ctx);
	const envKey = `${cfg.provider.toUpperCase()}_API_KEY`;
	const v = ctx.env[envKey];
	if (!v && cfg.provider !== "log") {
		throw ctx.error("MISSING_ENV", `${envKey} not set`);
	}
	return v ?? "";
}
