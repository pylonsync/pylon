/**
 * Built-in destinations. Optional — apps that only want primary
 * persistence skip the `destinations` config entirely.
 *
 * Pattern: each helper returns an `AuditDestination` { name, deliver }.
 */

import type { AuditDestination, AuditEvent } from "./types";

/**
 * Webhook destination. POSTs each event as JSON to `url` with an
 * HMAC-SHA256 signature in `X-Pylon-Audit-Signature`. Recipient
 * computes the same HMAC over the request body and compares
 * constant-time.
 */
export function webhookDestination(opts: {
	url: string;
	secret: string;
	headers?: Record<string, string>;
}): AuditDestination {
	return {
		name: `webhook(${new URL(opts.url).host})`,
		async deliver(event: AuditEvent) {
			const body = JSON.stringify(event);
			const sig = await hmacSha256Hex(opts.secret, body);
			const headers: Record<string, string> = {
				"Content-Type": "application/json",
				"X-Pylon-Audit-Signature": sig,
				"X-Pylon-Audit-Event": event.action,
				...(opts.headers ?? {}),
			};
			const res = await fetch(opts.url, {
				method: "POST",
				headers,
				body,
			});
			if (!res.ok) {
				throw new Error(`audit webhook ${opts.url} → HTTP ${res.status}`);
			}
		},
	};
}

/**
 * Datadog Logs destination. https://docs.datadoghq.com/api/latest/logs/
 *
 * Maps audit events to Datadog log entries with `ddsource: "pylon-audit"`,
 * `service: <action.prefix>`, and the event JSON as the message body.
 */
export function datadogDestination(opts: {
	apiKey: string;
	region?: "us" | "eu";
	service?: string;
}): AuditDestination {
	const host =
		opts.region === "eu" ? "https://http-intake.logs.datadoghq.eu" : "https://http-intake.logs.datadoghq.com";
	return {
		name: "datadog",
		async deliver(event: AuditEvent) {
			const body = JSON.stringify({
				ddsource: "pylon-audit",
				service: opts.service ?? event.action.split(".")[0] ?? "audit",
				message: event,
				ddtags: `action:${event.action},org:${event.organizationId ?? "none"}`,
				hostname: "pylon",
				date: new Date(event.occurredAt).toISOString(),
			});
			const res = await fetch(`${host}/api/v2/logs`, {
				method: "POST",
				headers: {
					"Content-Type": "application/json",
					"DD-API-KEY": opts.apiKey,
				},
				body,
			});
			if (!res.ok) {
				throw new Error(`datadog audit ingest → HTTP ${res.status}`);
			}
		},
	};
}

/**
 * Compute HMAC-SHA256 over `body` with `secret`, returning hex. Same
 * primitive as the Stripe signature helper but isolated here so the
 * audit-log package doesn't pull a dependency on @pylonsync/stripe
 * just for one crypto call.
 */
async function hmacSha256Hex(secret: string, body: string): Promise<string> {
	const enc = new TextEncoder();
	const secretBytes = enc.encode(secret);
	const secretBuf = new ArrayBuffer(secretBytes.byteLength);
	new Uint8Array(secretBuf).set(secretBytes);
	const bodyBytes = enc.encode(body);
	const bodyBuf = new ArrayBuffer(bodyBytes.byteLength);
	new Uint8Array(bodyBuf).set(bodyBytes);
	const key = await crypto.subtle.importKey(
		"raw",
		secretBuf,
		{ name: "HMAC", hash: "SHA-256" },
		false,
		["sign"],
	);
	const sig = await crypto.subtle.sign("HMAC", key, bodyBuf);
	return [...new Uint8Array(sig)]
		.map((b) => b.toString(16).padStart(2, "0"))
		.join("");
}
