import { action, mutation, query, v } from "@pylonsync/functions";

import { signWebhook } from "./signature";
import { postWebhookSafely } from "./safe-request";
import type { WebhookConfig } from "./types";

/**
 * Plugin-internal handlers. Apps wrap these as one-line files in
 * `functions/` (same pattern as @pylonsync/stripe).
 *
 * Public surface:
 *   - none (dispatch is called from app code; delivery is server-
 *     internal). Apps add their own typed `createWebhookEndpoint`
 *     mutation that reads/writes the entity directly with whatever
 *     authorization rules they want.
 *
 * Internal:
 *   - _pylonWebhookListEndpoints — used by dispatch()
 *   - _pylonWebhookDeliver       — the worker job
 *   - _pylonWebhookRecordAttempt — write the audit row
 */
export function internalHandlers(
	cfg: WebhookConfig,
): Record<string, unknown> {
	const endpointEnt = cfg.entities?.endpoint ?? "WebhookEndpoint";
	const attemptEnt = cfg.entities?.attempt ?? "WebhookAttempt";
	const retrySchedule = cfg.retrySchedule ?? [];

	return {
		_pylonWebhookListEndpoints: query({
			args: { applicationId: v.string() },
			internal: true,
			async handler(ctx, args: { applicationId: string }) {
				return ctx.db.query(endpointEnt, {
					applicationId: args.applicationId,
				});
			},
		}),

		_pylonWebhookRecordAttempt: mutation({
			args: {
				applicationId: v.string(),
				endpointId: v.string(),
				eventId: v.string(),
				eventType: v.string(),
				url: v.string(),
				attempt: v.number(),
				status: v.string(),
				httpStatus: v.optional(v.number()),
				error: v.optional(v.string()),
				scheduledAt: v.string(),
				deliveredAt: v.optional(v.string()),
			},
			internal: true,
			async handler(
				ctx,
				args: {
					applicationId: string;
					endpointId: string;
					eventId: string;
					eventType: string;
					url: string;
					attempt: number;
					status: string;
					httpStatus?: number;
					error?: string;
					scheduledAt: string;
					deliveredAt?: string;
				},
			) {
				return ctx.db.insert(attemptEnt, {
					applicationId: args.applicationId,
					endpointId: args.endpointId,
					eventId: args.eventId,
					eventType: args.eventType,
					url: args.url,
					attempt: args.attempt,
					status: args.status,
					httpStatus: args.httpStatus ?? null,
					error: args.error ?? null,
					scheduledAt: args.scheduledAt,
					deliveredAt: args.deliveredAt ?? null,
				});
			},
		}),

		_pylonWebhookDeliver: action({
			args: {
				endpointId: v.string(),
				eventId: v.string(),
				eventType: v.string(),
				occurredAt: v.string(),
				data: v.string(),
				applicationId: v.string(),
				attempt: v.number(),
			},
			internal: true,
			async handler(
				ctx,
				args: {
					endpointId: string;
					eventId: string;
					eventType: string;
					occurredAt: string;
					data: string;
					applicationId: string;
					attempt: number;
				},
			) {
				const endpoints = await ctx.runQuery<
					Array<{
						id: string;
						url: string;
						secret: string;
						disabled?: boolean;
						headers?: string | null;
					}>
				>("_pylonWebhookListEndpoints", {
					applicationId: args.applicationId,
				});
				const ep = endpoints.find((e) => e.id === args.endpointId);
				if (!ep || ep.disabled) {
					return { skipped: true };
				}

				const payload = JSON.parse(args.data) as unknown;
				const body = JSON.stringify({
					id: args.eventId,
					type: args.eventType,
					occurredAt: args.occurredAt,
					data: payload,
				});
				const ts = Math.floor(Date.now() / 1000);
				const sig = await signWebhook({
					id: args.eventId,
					timestamp: ts,
					body,
					secrets: [ep.secret],
				});

				const headers: Record<string, string> = Object.create(null);
				if (ep.headers) {
					try {
						const extra = JSON.parse(ep.headers) as Record<string, string>;
						for (const [k, v] of Object.entries(extra)) {
							if (typeof v === "string" && isAllowedCustomHeader(k, v)) {
								headers[k] = v;
							}
						}
					} catch {
						/* malformed headers — ignore */
					}
				}
				headers["Content-Type"] = "application/json";
				headers["webhook-id"] = sig.id;
				headers["webhook-timestamp"] = String(sig.timestamp);
				headers["webhook-signature"] = sig.signature;

				let httpStatus = 0;
				let errorMsg: string | undefined;
				let ok = false;
				try {
					const res = await postWebhookSafely(ep.url, headers, body);
					httpStatus = res.status;
					ok = res.ok;
					if (!ok) errorMsg = `HTTP ${res.status}`;
				} catch (e) {
					errorMsg = e instanceof Error ? e.message : String(e);
				}

				const now = new Date().toISOString();
				await ctx.runMutation("_pylonWebhookRecordAttempt", {
					applicationId: args.applicationId,
					endpointId: ep.id,
					eventId: args.eventId,
					eventType: args.eventType,
					url: ep.url,
					attempt: args.attempt,
					status: ok ? "succeeded" : "failed",
					httpStatus,
					error: errorMsg,
					scheduledAt: now,
					deliveredAt: ok ? now : undefined,
				});

				if (!ok) {
					const nextOffset = retrySchedule[args.attempt - 1];
					if (nextOffset === undefined) {
						// Out of retries — write a `dead` row + bail.
						await ctx.runMutation("_pylonWebhookRecordAttempt", {
							applicationId: args.applicationId,
							endpointId: ep.id,
							eventId: args.eventId,
							eventType: args.eventType,
							url: ep.url,
							attempt: args.attempt + 1,
							status: "dead",
							httpStatus,
							error: errorMsg,
							scheduledAt: now,
						});
						return { delivered: false, dead: true };
					}
					await ctx.scheduler.runAfter(
						nextOffset * 1000,
						"_pylonWebhookDeliver",
						{ ...args, attempt: args.attempt + 1 },
					);
					return { delivered: false, retryIn: nextOffset };
				}
				return { delivered: true, httpStatus };
			},
		}),
	};
}

const RESERVED_HEADERS = new Set([
	"connection",
	"content-length",
	"content-type",
	"host",
	"transfer-encoding",
	"webhook-id",
	"webhook-signature",
	"webhook-timestamp",
]);

function isAllowedCustomHeader(name: string, value: string): boolean {
	return (
		/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name) &&
		!RESERVED_HEADERS.has(name.toLowerCase()) &&
		!/[\r\n]/.test(value)
	);
}
