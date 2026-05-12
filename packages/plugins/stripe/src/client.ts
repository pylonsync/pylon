/**
 * Stripe REST client over `fetch`. Stripe's API is form-urlencoded
 * with bracket-style nesting (`foo[bar]=1`); JSON is rejected. We
 * skip the official `stripe` SDK because it ships a 90 MB dep tree
 * for what amounts to one fetch + one body encoder.
 */

const STRIPE_API_BASE = "https://api.stripe.com/v1";
const DEFAULT_API_VERSION = "2024-12-18.acacia";

export class StripeError extends Error {
	constructor(
		public status: number,
		public code: string,
		public stripeType: string,
		message: string,
	) {
		super(message);
		this.name = "StripeError";
	}
}

export interface StripeClientConfig {
	secretKey: string;
	apiVersion?: string;
	baseUrl?: string;
	/** Optional idempotency key for non-GET requests. */
	idempotencyKey?: string;
}

/**
 * Encode a nested object as Stripe's form-urlencoded bracket convention:
 *   { foo: { bar: 1 } } → foo[bar]=1
 *   { items: [{ price: "x" }] } → items[0][price]=x
 *
 * Stripe SDKs all do this internally; we surface it because the
 * webhook + checkout helpers in this package use it directly.
 */
export function encodeFormBody(input: Record<string, unknown>): string {
	const params = new URLSearchParams();
	const walk = (prefix: string, value: unknown): void => {
		if (value === undefined || value === null) return;
		if (Array.isArray(value)) {
			value.forEach((v, i) => walk(`${prefix}[${i}]`, v));
			return;
		}
		if (typeof value === "object") {
			for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
				walk(`${prefix}[${k}]`, v);
			}
			return;
		}
		params.append(prefix, String(value));
	};
	for (const [k, v] of Object.entries(input)) walk(k, v);
	return params.toString();
}

export async function stripeRequest<T>(
	cfg: StripeClientConfig,
	method: "GET" | "POST" | "DELETE",
	path: string,
	body?: Record<string, unknown>,
): Promise<T> {
	const baseUrl = cfg.baseUrl ?? STRIPE_API_BASE;
	const url = `${baseUrl}${path}`;
	const headers: Record<string, string> = {
		Authorization: `Bearer ${cfg.secretKey}`,
		"Stripe-Version": cfg.apiVersion ?? DEFAULT_API_VERSION,
	};
	if (cfg.idempotencyKey) {
		headers["Idempotency-Key"] = cfg.idempotencyKey;
	}
	let init: RequestInit = { method, headers };
	if (method !== "GET" && body) {
		headers["Content-Type"] = "application/x-www-form-urlencoded";
		init = { ...init, body: encodeFormBody(body) };
	}
	const res = await fetch(url, init);
	const text = await res.text();
	const parsed: unknown = text ? safeJson(text) : null;
	if (!res.ok) {
		const errObj = (
			parsed as { error?: { code?: string; type?: string; message?: string } }
		)?.error;
		const code = errObj?.code ?? `HTTP_${res.status}`;
		const type = errObj?.type ?? "api_error";
		const msg = errObj?.message ?? (text || res.statusText);
		throw new StripeError(res.status, code, type, msg);
	}
	return parsed as T;
}

function safeJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return null;
	}
}
