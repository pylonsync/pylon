import {
	type EntityDefinition,
	type PolicyDefinition,
	entity,
	field,
	policy,
} from "@pylonsync/sdk";

import type { WebhookConfig } from "./types";

export interface WebhookManifestFragment {
	entities: EntityDefinition[];
	policies: PolicyDefinition[];
}

export function buildWebhookManifest(
	cfg: WebhookConfig,
): WebhookManifestFragment {
	const endpointName = cfg.entities?.endpoint ?? "WebhookEndpoint";
	const attemptName = cfg.entities?.attempt ?? "WebhookAttempt";
	const secretField = cfg.encryptCredentials
		? field.string().serverOnly().encrypted()
		: field.string().serverOnly();
	const headersField = cfg.encryptCredentials
		? field.string().serverOnly().encrypted().optional()
		: field.string().serverOnly().optional();

	const Endpoint = entity(endpointName, {
		applicationId: field.string(),
		url: field.string(),
		secret: secretField,
		eventTypes: field.string(), // JSON array
		headers: headersField, // JSON
		disabled: field.boolean().optional(),
		createdAt: field.string(),
	});

	const Attempt = entity(attemptName, {
		applicationId: field.string(),
		endpointId: field.string(),
		eventId: field.string(),
		eventType: field.string(),
		url: field.string(),
		attempt: field.number(),
		status: field.string(),
		httpStatus: field.number().optional(),
		error: field.string().optional(),
		scheduledAt: field.string(),
		deliveredAt: field.string().optional(),
	});

	// Endpoints are tenant-scoped (app == tenant in the common
	// multi-tenant SaaS case). Apps that want admin-only access
	// override these in their own manifest.
	const endpointPolicy = policy({
		name: `${endpointName.toLowerCase()}_tenant_scoped`,
		entity: endpointName,
		allowRead: "auth.tenantId == data.applicationId or auth.is_admin == true",
		allowInsert:
			"auth.tenantId == data.applicationId or auth.is_admin == true",
		allowUpdate:
			"auth.tenantId == data.applicationId or auth.is_admin == true",
		allowDelete:
			"auth.tenantId == data.applicationId or auth.is_admin == true",
	});

	// Attempt rows are read-only — they're an internal audit trail.
	// Tenant-scoped reads so the customer's dashboard can show
	// delivery history; writes only via the plugin's internal
	// mutation.
	const attemptPolicy = policy({
		name: `${attemptName.toLowerCase()}_read_only`,
		entity: attemptName,
		allowRead: "auth.tenantId == data.applicationId or auth.is_admin == true",
		allowInsert: "false",
		allowUpdate: "false",
		allowDelete: "false",
	});

	return {
		entities: [Endpoint, Attempt],
		policies: [endpointPolicy, attemptPolicy],
	};
}
