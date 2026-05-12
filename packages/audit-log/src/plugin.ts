import { mutation, query, v } from "@pylonsync/functions";

import { buildAuditLogManifest } from "./manifest";
import { logEvent } from "./log";
import type { AuditConfig } from "./types";

export function auditLog(cfg: AuditConfig = {}) {
	const entityName = cfg.entityName ?? "AuditEvent";

	return {
		config: cfg,
		manifest: buildAuditLogManifest(cfg),
		log: logEvent,

		handlers: {
			// Internal insert. Apps don't call this directly — logEvent()
			// wraps it. Internal: true keeps it off the public HTTP
			// surface where it'd otherwise let any caller forge audit
			// events.
			_pylonAuditInsert: mutation({
				args: {
					entityName: v.string(),
					idempotencyKey: v.string(),
					organizationId: v.optional(v.string()),
					actor: v.string(),
					action: v.string(),
					targets: v.string(),
					context: v.string(),
					metadata: v.string(),
					occurredAt: v.string(),
				},
				internal: true,
				async handler(
					ctx,
					args: {
						entityName: string;
						idempotencyKey: string;
						organizationId?: string | null;
						actor: string;
						action: string;
						targets: string;
						context: string;
						metadata: string;
						occurredAt: string;
					},
				) {
					// Idempotency: skip if a row with the same key already
					// exists. Insertion via the entity-API runs through
					// any policy/plugin layer (TenantScopePlugin etc.)
					// just like a regular write — that's intentional, the
					// audit log is a normal entity that happens to be
					// append-only.
					const existing = (await ctx.db.query(args.entityName, {
						idempotencyKey: args.idempotencyKey,
					})) as Array<{ id: string }>;
					if (existing[0]) return existing[0].id;

					return ctx.db.insert(args.entityName, {
						idempotencyKey: args.idempotencyKey,
						organizationId: args.organizationId ?? null,
						actor: args.actor,
						action: args.action,
						targets: args.targets,
						context: args.context,
						metadata: args.metadata,
						occurredAt: args.occurredAt,
					});
				},
			}),

			// Admin-only prune. Apps call this from a cron with their
			// retention-policy cutoff timestamp. Pylon's policy engine
			// blocks non-admin callers from hitting it because the
			// AuditEvent entity has allowDelete: "false" globally —
			// this mutation goes through ctx.db.delete which would be
			// blocked too. We bypass via the internal: true flag, but
			// the handler doubles down by requiring ctx.auth.isAdmin.
			pruneAuditEvents: mutation({
				args: { cutoffISO: v.string() },
				internal: true,
				async handler(ctx, args: { cutoffISO: string }) {
					if (!ctx.auth.isAdmin) {
						throw new Error("FORBIDDEN: admin only");
					}
					// We don't have a bulk-delete primitive that takes a
					// predicate; iterate + delete. For multi-million-row
					// audit logs the caller should chunk via a cron loop,
					// but for typical SaaS volumes (10k events/day) a
					// single pass is fine.
					const rows = (await ctx.db.query(entityName, {})) as Array<{
						id: string;
						occurredAt: string;
					}>;
					let deleted = 0;
					for (const r of rows) {
						if (r.occurredAt < args.cutoffISO) {
							await ctx.db.delete(entityName, r.id);
							deleted++;
						}
					}
					return { deleted };
				},
			}),

			// Public read endpoint — apps point their admin dashboard's
			// "Activity" table at this. Tenant-scoped by default; the
			// policy on the AuditEvent entity does the actual gating.
			listAuditEvents: query({
				args: {
					organizationId: v.optional(v.string()),
					action: v.optional(v.string()),
					limit: v.optional(v.number()),
				},
				async handler(
					ctx,
					args: {
						organizationId?: string;
						action?: string;
						limit?: number;
					},
				) {
					const filter: Record<string, unknown> = {};
					if (args.organizationId)
						filter.organizationId = args.organizationId;
					if (args.action) filter.action = args.action;
					filter.$order = { occurredAt: "desc" };
					filter.$limit = args.limit ?? 100;
					return ctx.db.query(entityName, filter);
				},
			}),
		},
	};
}
