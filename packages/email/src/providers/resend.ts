/**
 * Resend adapter. https://resend.com/docs/api-reference/emails/send-email
 *
 * Uses raw fetch instead of the official `resend` package because
 * we already cross over to JSON-and-fetch for every other provider
 * here — keeps the surface uniform + the dep tree small. Add the
 * official SDK back if React-Email rendering ever moves into the
 * provider boundary; that's the only thing fetch doesn't cover.
 */

import type {
	EmailDeliveryEvent,
	EmailProvider,
	EmailSendResult,
	ProviderSendArgs,
} from "../types";

const API_BASE = "https://api.resend.com";

export const resendProvider: EmailProvider = {
	name: "resend",

	async send(args: ProviderSendArgs): Promise<EmailSendResult> {
		const body: Record<string, unknown> = {
			from: args.from,
			to: args.to,
			subject: args.subject,
			text: args.text,
			html: args.html,
			reply_to: args.replyTo,
			cc: args.cc,
			bcc: args.bcc,
			scheduled_at: args.scheduledAt,
			tags: args.tags
				? Object.entries(args.tags).map(([name, value]) => ({ name, value }))
				: undefined,
			...args.extra,
		};
		const headers: Record<string, string> = {
			Authorization: `Bearer ${args.apiKey}`,
			"Content-Type": "application/json",
		};
		if (args.idempotencyKey) {
			headers["Idempotency-Key"] = args.idempotencyKey;
		}
		const res = await fetch(`${API_BASE}/emails`, {
			method: "POST",
			headers,
			body: JSON.stringify(body),
		});
		const text = await res.text();
		const parsed = text ? safeJson(text) : null;
		if (!res.ok) {
			const msg =
				(parsed as { message?: string } | null)?.message ??
				`HTTP ${res.status}`;
			throw new Error(`Resend send failed: ${msg}`);
		}
		const id = (parsed as { id?: string } | null)?.id ?? "";
		return { messageId: id };
	},

	async verifyWebhook(
		secret: string,
		headers: Record<string, string | undefined>,
		rawBody: string,
	): Promise<true | string> {
		// Resend uses Svix-signed webhooks. Headers:
		//   svix-id, svix-timestamp, svix-signature: "v1,<base64>"
		const id = headers["svix-id"];
		const ts = headers["svix-timestamp"];
		const sig = headers["svix-signature"];
		if (!id || !ts || !sig) return "MISSING_HEADER";

		const tsNum = Number.parseInt(ts, 10);
		if (Number.isNaN(tsNum)) return "BAD_TIMESTAMP";
		const ageSecs = Math.abs(Date.now() / 1000 - tsNum);
		if (ageSecs > 5 * 60) return "REPLAYED";

		// Svix secret is `whsec_<base64>`; we sign HMAC-SHA256 over
		// `<id>.<ts>.<rawBody>` with the decoded bytes and compare
		// against any of the signatures in the header.
		const rawSecret: ArrayBuffer = secret.startsWith("whsec_")
			? base64Decode(secret.slice("whsec_".length))
			: toArrayBuffer(new TextEncoder().encode(secret));
		const payload = toArrayBuffer(
			new TextEncoder().encode(`${id}.${ts}.${rawBody}`),
		);
		const key = await crypto.subtle.importKey(
			"raw",
			rawSecret,
			{ name: "HMAC", hash: "SHA-256" },
			false,
			["sign"],
		);
		const expected = await crypto.subtle.sign("HMAC", key, payload);
		const expectedB64 = base64Encode(new Uint8Array(expected));

		for (const part of sig.split(" ")) {
			const [version, candidate] = part.split(",");
			if (version !== "v1") continue;
			if (constantTimeEqual(candidate, expectedB64)) return true;
		}
		return "INVALID_SIGNATURE";
	},

	parseWebhookEvent(rawBody: string): EmailDeliveryEvent | null {
		try {
			const evt = JSON.parse(rawBody) as {
				type?: string;
				data?: {
					email_id?: string;
					to?: string[] | string;
					created_at?: string;
					reason?: string;
					link?: { url?: string };
				};
			};
			const type = (evt.type ?? "").replace(/^email\./, "");
			const recipient = Array.isArray(evt.data?.to)
				? (evt.data?.to as string[])[0]
				: ((evt.data?.to as string) ?? "");
			const occurredAt = evt.data?.created_at ?? new Date().toISOString();
			const messageId = evt.data?.email_id ?? "";
			if (!type || !messageId || !recipient) return null;
			return {
				type: type as EmailDeliveryEvent["type"],
				messageId,
				recipient,
				occurredAt,
				reason: evt.data?.reason,
				url: evt.data?.link?.url,
			};
		} catch {
			return null;
		}
	},
};

function safeJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return null;
	}
}

function base64Decode(s: string): ArrayBuffer {
	const bin = atob(s);
	const out = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
	return toArrayBuffer(out);
}

/**
 * Copy a Uint8Array into a freshly-allocated ArrayBuffer. Without
 * the copy, lib.dom's WebCrypto types reject `Uint8Array<SharedArrayBuffer>`
 * because `crypto.subtle.sign` requires `BufferSource` over a plain
 * `ArrayBuffer`. The copy is one allocation per webhook, negligible.
 */
function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
	const buf = new ArrayBuffer(bytes.byteLength);
	new Uint8Array(buf).set(bytes);
	return buf;
}

function base64Encode(bytes: Uint8Array): string {
	let s = "";
	for (const b of bytes) s += String.fromCharCode(b);
	return btoa(s);
}

function constantTimeEqual(a: string, b: string): boolean {
	if (a.length !== b.length) return false;
	let diff = 0;
	for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
	return diff === 0;
}
