/**
 * Dispatch path: app calls `dispatch(ctx, cfg, { type, data })`, the
 * plugin finds every matching endpoint, schedules a delivery job
 * for each.
 *
 * Jobs are scheduled via Pylon's `ctx.scheduler.runAfter(ms, fnName,
 * args)` API — the framework's job worker picks them up + invokes
 * the `_pylonWebhookDeliver` internal action.
 *
 * Caller's ctx.auth identity is propagated to the job (per v0.3.76
 * fix), so the delivery handler executes with the same auth as the
 * dispatcher — required because the worker reads the WebhookEndpoint
 * + writes the WebhookAttempt under the same tenant scope.
 */

import type { DispatchInput, WebhookConfig, WebhookCtx } from "./types";

export async function dispatch(
	ctx: WebhookCtx,
	cfg: WebhookConfig,
	input: DispatchInput,
): Promise<{ scheduled: number; eventId: string }> {
	const eventId = input.id ?? generateEventId();
	const occurredAt = input.occurredAt ?? new Date().toISOString();
	const applicationId =
		input.applicationId ??
		(cfg.getApplicationId
			? await cfg.getApplicationId(ctx, {
					id: eventId,
					type: input.type,
					occurredAt,
					data: input.data,
				})
			: ctx.auth.tenantId);
	if (!applicationId) {
		throw ctx.error(
			"NO_APPLICATION",
			"webhooks dispatch needs an applicationId (no active tenant)",
		);
	}

	const endpoints = await ctx.runQuery<
		Array<{
			id: string;
			url: string;
			eventTypes: string;
			disabled?: boolean;
		}>
	>("_pylonWebhookListEndpoints", { applicationId });

	let scheduled = 0;
	for (const ep of endpoints) {
		if (ep.disabled) continue;
		const types = safeParseArray(ep.eventTypes);
		if (types.length > 0 && !types.includes(input.type)) continue;
		await ctx.scheduler.runAfter(0, "_pylonWebhookDeliver", {
			endpointId: ep.id,
			eventId,
			eventType: input.type,
			occurredAt,
			data: JSON.stringify(input.data),
			applicationId,
			attempt: 1,
		});
		scheduled++;
	}
	return { scheduled, eventId };
}

function generateEventId(): string {
	const bytes = new Uint8Array(16);
	crypto.getRandomValues(bytes);
	return `evt_${[...bytes].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
}

function safeParseArray(s: string): string[] {
	try {
		const out = JSON.parse(s);
		return Array.isArray(out) ? (out as string[]) : [];
	} catch {
		return [];
	}
}
