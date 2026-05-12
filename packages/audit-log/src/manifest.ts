import {
	type EntityDefinition,
	type PolicyDefinition,
	entity,
	field,
	policy,
} from "@pylonsync/sdk";

import type { AuditConfig } from "./types";

export interface AuditLogManifestFragment {
	entities: EntityDefinition[];
	policies: PolicyDefinition[];
}

export function buildAuditLogManifest(
	cfg: AuditConfig = {},
): AuditLogManifestFragment {
	const name = cfg.entityName ?? "AuditEvent";
	const AuditEvent = entity(name, {
		// Composite uniqueness via idempotencyKey — re-delivery from
		// destinations (e.g. webhook retries) lands the same logical
		// event without duplicating. We don't make this a true
		// unique index because some apps want multiple events with
		// the same key (different metadata) under exceptional
		// circumstances; the plugin's INSERT path skips duplicates,
		// but apps writing directly via ctx.db can override.
		idempotencyKey: field.string(),
		organizationId: field.string().optional(),
		actor: field.string(), // JSON
		action: field.string(),
		targets: field.string(), // JSON array
		context: field.string(), // JSON
		metadata: field.string(), // JSON
		occurredAt: field.string(),
	});

	const auditPolicy = policy({
		name: `${name.toLowerCase()}_append_only`,
		entity: name,
		// Apps with tenant scoping read by tenantId; the plugin
		// stamps it at insert. Cross-tenant audit access stays
		// admin-only (auth.is_admin checked separately in admin
		// dashboards).
		allowRead:
			"auth.is_admin == true or auth.tenantId == data.organizationId",
		allowInsert: "true",
		// Append-only — no updates, no deletes. Pruning happens via
		// the dedicated `pruneAuditEvents()` action which runs as
		// admin and is the one place where DELETE is permitted.
		allowUpdate: "false",
		allowDelete: "false",
	});

	return { entities: [AuditEvent], policies: [auditPolicy] };
}
