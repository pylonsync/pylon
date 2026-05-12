/**
 * `@pylonsync/audit-log` — append-only event store for compliance,
 * SIEM forwarding, and admin transparency.
 *
 * Schema follows WorkOS' audit-log shape so events port cleanly to
 * downstream SIEM tools (Splunk, Datadog, Sumo Logic) without
 * re-mapping. Required fields per event:
 *
 *   - actor:     { id, type, name?, metadata? }   // who did it
 *   - action:    string                            // event type
 *   - targets:   [{ id, type, name?, metadata? }] // what was touched
 *   - context:   { location?, userAgent? }        // network state
 *   - occurredAt: ISO-8601                         // when
 *   - organizationId: string                       // tenant
 *   - metadata:  Record<string, scalar>            // freeform context
 *   - idempotencyKey: string                       // dedup
 *
 * Storage is append-only: the manifest fragment declares an
 * `AuditEvent` entity with policy `allowInsert: "true"` but
 * `allowUpdate: "false"` + `allowDelete: "false"`. Operators who
 * want hard tamper-evidence layer a hash chain on top (each row's
 * hash includes the previous row's hash); we ship that as a
 * follow-on rather than baking it in by default — it doubles write
 * cost and most SaaS apps don't need it.
 *
 * Other plugins emit audit events automatically when they detect
 * `@pylonsync/audit-log` is installed: org member changes, billing
 * mutations, admin actions. Apps can also emit custom events for
 * their own domain (e.g. `recording.deleted`, `org.transferred`).
 */

export type {
	AuditEvent,
	AuditActor,
	AuditTarget,
	AuditEventInput,
	AuditCtx,
	AuditConfig,
} from "./types";
export { auditLog } from "./plugin";
export { logEvent } from "./log";
export { buildAuditLogManifest } from "./manifest";
export type { AuditLogManifestFragment } from "./manifest";
export { webhookDestination, datadogDestination } from "./destinations";
export type { AuditDestination } from "./types";
