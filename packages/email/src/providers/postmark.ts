/**
 * Postmark adapter. https://postmarkapp.com/developer/api/email-api
 */

import type {
	EmailDeliveryEvent,
	EmailProvider,
	EmailSendResult,
	ProviderSendArgs,
} from "../types";

const API_BASE = "https://api.postmarkapp.com";

export const postmarkProvider: EmailProvider = {
	name: "postmark",

	async send(args: ProviderSendArgs): Promise<EmailSendResult> {
		const body: Record<string, unknown> = {
			From: args.from,
			To: args.to.join(","),
			Subject: args.subject,
			TextBody: args.text,
			HtmlBody: args.html,
			ReplyTo: args.replyTo,
			Cc: args.cc?.join(","),
			Bcc: args.bcc?.join(","),
			MessageStream: (args.extra?.MessageStream as string) ?? "outbound",
			Tag: args.tags?.tag,
			Metadata: args.tags,
		};
		const res = await fetch(`${API_BASE}/email`, {
			method: "POST",
			headers: {
				Accept: "application/json",
				"Content-Type": "application/json",
				"X-Postmark-Server-Token": args.apiKey,
			},
			body: JSON.stringify(body),
		});
		const text = await res.text();
		const parsed = text ? safeJson(text) : null;
		if (!res.ok) {
			const msg =
				(parsed as { Message?: string } | null)?.Message ??
				`HTTP ${res.status}`;
			throw new Error(`Postmark send failed: ${msg}`);
		}
		const id = (parsed as { MessageID?: string } | null)?.MessageID ?? "";
		return { messageId: id };
	},

	async verifyWebhook(
		secret: string,
		headers: Record<string, string | undefined>,
		rawBody: string,
	): Promise<true | string> {
		// Postmark uses basic-auth on the webhook URL itself rather
		// than per-event HMAC. Our convention: configure the webhook
		// URL as `https://user:secret@host/...` in the Postmark
		// dashboard, and the secret here matches what's after the
		// colon. Pylon's HTTP router exposes basic-auth via the
		// `Authorization` header; we compare in constant time.
		const auth = headers.authorization ?? "";
		const expected = `Basic ${btoa(`pylon:${secret}`)}`;
		void rawBody; // unused — signature is on the request itself
		if (auth.length !== expected.length) return "INVALID_SIGNATURE";
		let diff = 0;
		for (let i = 0; i < auth.length; i++) {
			diff |= auth.charCodeAt(i) ^ expected.charCodeAt(i);
		}
		return diff === 0 ? true : "INVALID_SIGNATURE";
	},

	parseWebhookEvent(rawBody: string): EmailDeliveryEvent | null {
		try {
			const evt = JSON.parse(rawBody) as {
				RecordType?: string;
				MessageID?: string;
				Recipient?: string;
				Email?: string;
				Description?: string;
				BouncedAt?: string;
				DeliveredAt?: string;
				ReceivedAt?: string;
				OriginalLink?: string;
			};
			const recordType = (evt.RecordType ?? "").toLowerCase();
			const type = mapPostmarkType(recordType);
			if (!type) return null;
			return {
				type,
				messageId: evt.MessageID ?? "",
				recipient: evt.Recipient ?? evt.Email ?? "",
				occurredAt:
					evt.DeliveredAt ?? evt.BouncedAt ?? evt.ReceivedAt ?? new Date().toISOString(),
				reason: evt.Description,
				url: evt.OriginalLink,
			};
		} catch {
			return null;
		}
	},
};

function mapPostmarkType(t: string): EmailDeliveryEvent["type"] | null {
	switch (t) {
		case "delivery":
			return "delivered";
		case "bounce":
			return "bounced";
		case "spamcomplaint":
			return "complained";
		case "open":
			return "opened";
		case "click":
			return "clicked";
		default:
			return null;
	}
}

function safeJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return null;
	}
}
