/**
 * Core write path. Apps call:
 *
 *   await logEvent(ctx, cfg, {
 *     actor: { id: userId, type: "user" },
 *     action: "project.deleted",
 *     targets: [{ id: projectId, type: "project" }],
 *     metadata: { reason: "user_initiated" },
 *   });
 *
 * The plugin computes the idempotency key (if not supplied), stamps
 * occurredAt + context (IP / UA from ctx.request if available),
 * persists via the AuditEvent mutation, then fans out to every
 * registered destination. Destination errors are caught + logged
 * but don't fail the primary write.
 */

import type {
	AuditConfig,
	AuditCtx,
	AuditEvent,
	AuditEventInput,
} from "./types";

export async function logEvent(
	ctx: AuditCtx,
	cfg: AuditConfig,
	input: AuditEventInput,
): Promise<AuditEvent> {
	const entityName = cfg.entityName ?? "AuditEvent";
	const occurredAt = input.occurredAt ?? new Date().toISOString();
	const idempotencyKey =
		input.idempotencyKey ?? deriveIdempotencyKey(input, occurredAt);

	const ipAddress =
		input.context?.ipAddress ??
		ctx.request?.headers["x-forwarded-for"]?.split(",")[0].trim() ??
		ctx.request?.headers["fly-client-ip"] ??
		undefined;
	const userAgent =
		input.context?.userAgent ?? ctx.request?.headers["user-agent"] ?? undefined;

	const id = await ctx.runMutation<string>("_pylonAuditInsert", {
		entityName,
		idempotencyKey,
		organizationId:
			input.organizationId ?? ctx.auth.tenantId ?? null,
		actor: JSON.stringify(input.actor),
		action: input.action,
		targets: JSON.stringify(input.targets ?? []),
		context: JSON.stringify({ ipAddress, userAgent }),
		metadata: JSON.stringify(input.metadata ?? {}),
		occurredAt,
	});

	const event: AuditEvent = {
		id,
		idempotencyKey,
		organizationId:
			input.organizationId ?? ctx.auth.tenantId ?? null,
		actor: input.actor,
		action: input.action,
		targets: input.targets ?? [],
		context: { ipAddress, userAgent },
		metadata: input.metadata ?? {},
		occurredAt,
	};

	// Fan out to destinations. Each is awaited individually but the
	// outer loop swallows errors — one failing destination cannot
	// poison the rest.
	for (const dest of cfg.destinations ?? []) {
		try {
			await dest.deliver(event);
		} catch (e) {
			// eslint-disable-next-line no-console
			console.warn(
				`[audit-log] destination "${dest.name}" delivery failed: ${e instanceof Error ? e.message : String(e)}`,
			);
		}
	}

	return event;
}

function deriveIdempotencyKey(
	input: AuditEventInput,
	occurredAt: string,
): string {
	// Without a caller-supplied key, dedup on the natural composite:
	// (actor, action, target-ids, occurredAt). Cheap hash to stay short.
	const targetIds = (input.targets ?? []).map((t) => t.id).join(",");
	return `${input.actor.id}:${input.action}:${targetIds}:${occurredAt}`;
}
