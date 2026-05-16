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

	// `_pylonWebhookDeliver` is internal:true. Pylon framework's
	// scheduler refuses public-action → internal-target enqueues to
	// stop public actions becoming a router-bypass into internal
	// helpers. Dispatch is by design called from public app actions
	// (when an HTTP-triggered mutation wants to emit an event), so
	// we need to grant the call admin authority before scheduling.
	//
	// On framework v0.3.118+ ctx.auth.elevate() exists; on older
	// runtimes it doesn't. Guard the call so apps pinned to old
	// pylon versions still type-check + run (they'd just get the
	// SCHEDULE_FAILED error at the runAfter call, which is what they
	// got before this fix landed — net no regression).
	if (typeof ctx.auth.elevate === "function") {
		await ctx.auth.elevate({
			admin: true,
			reason:
				"webhooks plugin: scheduling _pylonWebhookDeliver (internal:true) — caller authority is the app's own dispatch trust boundary, the worker validates endpoint+tenant",
		});
	}

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
