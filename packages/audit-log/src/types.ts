export interface AuditActor {
	id: string;
	type: "user" | "system" | "api_key" | "service" | string;
	name?: string;
	metadata?: Record<string, unknown>;
}

export interface AuditTarget {
	id: string;
	type: string;
	name?: string;
	metadata?: Record<string, unknown>;
}

export interface AuditEvent {
	id: string;
	idempotencyKey: string;
	organizationId: string | null;
	actor: AuditActor;
	action: string;
	targets: AuditTarget[];
	context: {
		ipAddress?: string;
		userAgent?: string;
	};
	metadata: Record<string, unknown>;
	occurredAt: string;
}

export interface AuditEventInput {
	idempotencyKey?: string;
	organizationId?: string | null;
	actor: AuditActor;
	action: string;
	targets?: AuditTarget[];
	context?: {
		ipAddress?: string;
		userAgent?: string;
	};
	metadata?: Record<string, unknown>;
	occurredAt?: string;
}

export interface AuditCtx {
	env: Record<string, string | undefined>;
	auth: { userId?: string | null; tenantId?: string | null; isAdmin?: boolean };
	request?: {
		headers: Record<string, string | undefined>;
		rawBody: string;
	};
	runMutation: <T = unknown>(
		name: string,
		args: Record<string, unknown>,
	) => Promise<T>;
	runQuery: <T>(name: string, args: Record<string, unknown>) => Promise<T>;
	error: (code: string, message: string) => Error;
}

export interface AuditConfig {
	/**
	 * Entity name override. Default `AuditEvent`. Cloud uses
	 * `AuditEvent` already, so most apps don't need this.
	 */
	entityName?: string;
	/**
	 * Streaming destinations. Each destination receives every event
	 * as it's persisted. Errors in destinations DO NOT fail the
	 * primary write — audit logs must reliably persist even when
	 * SIEM ingest is degraded.
	 *
	 * Built-in destinations:
	 *   - `webhook({ url, secret })` — HMAC-signed POST per event
	 *   - `s3({ bucket, prefix, region })` — daily Parquet flush
	 *   - `datadog({ apiKey })` — Datadog Logs ingest
	 *
	 * Custom destinations implement the `AuditDestination` interface.
	 */
	destinations?: AuditDestination[];
	/**
	 * Retention period in days. Apps that want forever-retention
	 * leave this undefined; SOC2-strict configs typically set 365.
	 * The plugin doesn't auto-prune — apps wire a cron to call
	 * `pruneAuditEvents()` with this value as the cutoff. Surfacing
	 * the policy here keeps it declarative + documented in one place.
	 */
	retentionDays?: number;
}

export interface AuditDestination {
	name: string;
	deliver(event: AuditEvent): Promise<void>;
}
