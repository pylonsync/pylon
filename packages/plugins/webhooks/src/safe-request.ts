import { lookup } from "node:dns/promises";
import { request } from "node:https";
import { isIP } from "node:net";

const DEFAULT_TIMEOUT_MS = 10_000;
const MAX_REDIRECTS = 3;

interface ResolvedAddress {
	address: string;
	family: number;
}

interface ResponseHead {
	status: number;
	location?: string;
}

export interface SafeRequestDependencies {
	resolve?: (hostname: string) => Promise<ResolvedAddress[]>;
	requestOnce?: (
		url: URL,
		init: { headers: Record<string, string>; body: string; signal: AbortSignal },
	) => Promise<ResponseHead>;
	timeoutMs?: number;
}

/**
 * POST a webhook without allowing the destination to reach local services.
 * Redirects stay on the original origin so credentials cannot be forwarded to
 * a second host. Only 307/308 are followed because they preserve the signed
 * POST body.
 */
export async function postWebhookSafely(
	url: string,
	headers: Record<string, string>,
	body: string,
	dependencies: SafeRequestDependencies = {},
): Promise<{ status: number; ok: boolean }> {
	const resolve = dependencies.resolve ?? resolveHostname;
	const timeoutMs = dependencies.timeoutMs ?? DEFAULT_TIMEOUT_MS;
	const controller = new AbortController();
	const timeout = setTimeout(() => controller.abort(), timeoutMs);

	try {
		let target = parseDestination(url);
		const originalOrigin = target.origin;
		let redirects = 0;

		for (;;) {
			await assertPublicDestination(target, resolve, controller.signal);
			const response = await (dependencies.requestOnce ?? requestOnce)(target, {
				headers,
				body,
				signal: controller.signal,
			});

			if (![301, 302, 303, 307, 308].includes(response.status)) {
				return {
					status: response.status,
					ok: response.status >= 200 && response.status < 300,
				};
			}
			if (response.status !== 307 && response.status !== 308) {
				throw new Error("Webhook destination returned an unsafe redirect");
			}
			if (!response.location) {
				throw new Error("Webhook destination returned an invalid redirect");
			}
			if (redirects >= MAX_REDIRECTS) {
				throw new Error("Webhook destination exceeded the redirect limit");
			}

			const next = parseDestination(new URL(response.location, target).toString());
			if (next.origin !== originalOrigin) {
				throw new Error("Webhook destination redirected to another origin");
			}
			target = next;
			redirects++;
		}
	} catch (error) {
		if (controller.signal.aborted) {
			throw new Error("Webhook delivery timed out");
		}
		if (error instanceof SafeDestinationError) throw new Error(error.message);
		if (error instanceof Error && error.message.startsWith("Webhook destination")) {
			throw error;
		}
		throw new Error("Webhook delivery request failed");
	} finally {
		clearTimeout(timeout);
	}
}

export async function assertPublicDestination(
	target: URL,
	resolve: (hostname: string) => Promise<ResolvedAddress[]> = resolveHostname,
	signal?: AbortSignal,
): Promise<void> {
	if (target.protocol !== "https:") {
		throw new SafeDestinationError("Webhook destinations must use HTTPS");
	}
	if (target.username || target.password) {
		throw new SafeDestinationError("Webhook destination credentials are not allowed");
	}

	let addresses: ResolvedAddress[];
	try {
		const resolution = resolve(stripIpv6Brackets(target.hostname));
		addresses = signal ? await rejectOnAbort(resolution, signal) : await resolution;
	} catch {
		throw new SafeDestinationError("Webhook destination could not be resolved");
	}
	if (addresses.length === 0) {
		throw new SafeDestinationError("Webhook destination could not be resolved");
	}
	if (addresses.some(({ address }) => isBlockedIp(address))) {
		throw new SafeDestinationError("Webhook destination resolves to a non-public address");
	}
}

function rejectOnAbort<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
	if (signal.aborted) return Promise.reject(new Error("aborted"));
	return new Promise((resolve, reject) => {
		const onAbort = () => reject(new Error("aborted"));
		signal.addEventListener("abort", onAbort, { once: true });
		promise.then(
			(value) => {
				signal.removeEventListener("abort", onAbort);
				resolve(value);
			},
			(error) => {
				signal.removeEventListener("abort", onAbort);
				reject(error);
			},
		);
	});
}

export function isBlockedIp(address: string): boolean {
	const family = isIP(address);
	if (family === 4) return isBlockedIpv4(address);
	if (family !== 6) return true;

	const bytes = parseIpv6(address);
	if (!bytes) return true;
	if (bytes.every((byte) => byte === 0)) return true; // unspecified
	if (bytes.slice(0, 15).every((byte) => byte === 0) && bytes[15] === 1) return true;
	if ((bytes[0] & 0xfe) === 0xfc) return true; // unique-local fc00::/7
	if (bytes[0] === 0xfe && (bytes[1] & 0xc0) === 0x80) return true; // link-local fe80::/10
	if (bytes[0] === 0xfe && (bytes[1] & 0xc0) === 0xc0) return true; // deprecated site-local fec0::/10
	if (bytes[0] === 0xff) return true; // multicast
	if (bytes[0] === 0x20 && bytes[1] === 0x01 && bytes[2] === 0x0d && bytes[3] === 0xb8) {
		return true; // documentation 2001:db8::/32
	}
	if (bytes.slice(0, 12).every((byte) => byte === 0)) {
		return isBlockedIpv4(bytes.slice(12).join(".")); // IPv4-compatible
	}

	const mappedPrefix = bytes.slice(0, 10).every((byte) => byte === 0);
	if (mappedPrefix && bytes[10] === 0xff && bytes[11] === 0xff) {
		return isBlockedIpv4(bytes.slice(12).join("."));
	}
	return false;
}

async function resolveHostname(hostname: string): Promise<ResolvedAddress[]> {
	if (isIP(hostname)) {
		return [{ address: hostname, family: isIP(hostname) }];
	}
	return lookup(hostname, { all: true, verbatim: true });
}

function parseDestination(value: string): URL {
	let target: URL;
	try {
		target = new URL(value);
	} catch {
		throw new SafeDestinationError("Webhook destination URL is invalid");
	}
	if (target.protocol !== "https:") {
		throw new SafeDestinationError("Webhook destinations must use HTTPS");
	}
	if (target.username || target.password) {
		throw new SafeDestinationError("Webhook destination credentials are not allowed");
	}
	return target;
}

function requestOnce(
	target: URL,
	init: { headers: Record<string, string>; body: string; signal: AbortSignal },
): Promise<ResponseHead> {
	return new Promise((resolve, reject) => {
		const req = request(
			target,
			{
				method: "POST",
				headers: init.headers,
				signal: init.signal,
				lookup(hostname, _options, callback) {
					resolveHostname(hostname)
						.then((addresses) => {
							if (
								addresses.length === 0 ||
								addresses.some(({ address }) => isBlockedIp(address))
							) {
								callback(new Error("unsafe destination"), "", 4);
								return;
							}
							const selected = addresses[0];
							callback(null, selected.address, selected.family);
						})
						.catch(() =>
							callback(new Error("destination lookup failed"), "", 4),
						);
				},
			},
			(response) => {
				const head = {
					status: response.statusCode ?? 0,
					location: Array.isArray(response.headers.location)
						? response.headers.location[0]
						: response.headers.location,
				};
				// Delivery semantics depend only on the response head. Close the
				// body immediately so an endpoint cannot retain a worker socket by
				// streaming an endless response after returning a success status.
				response.destroy();
				resolve(head);
			},
		);
		req.on("error", reject);
		req.end(init.body);
	});
}

function isBlockedIpv4(address: string): boolean {
	const octets = address.split(".").map(Number);
	if (
		octets.length !== 4 ||
		octets.some(
			(part) => !Number.isInteger(part) || part < 0 || part > 255,
		)
	) {
		return true;
	}
	const [a, b, c] = octets;
	return (
		a === 0 ||
		a === 10 ||
		a === 127 ||
		(a === 100 && b >= 64 && b <= 127) ||
		(a === 169 && b === 254) ||
		(a === 172 && b >= 16 && b <= 31) ||
		(a === 192 && b === 0 && c === 0) ||
		(a === 192 && b === 168) ||
		(a === 192 && b === 0 && c === 2) ||
		(a === 198 && (b === 18 || b === 19)) ||
		(a === 198 && b === 51 && c === 100) ||
		(a === 203 && b === 0 && c === 113) ||
		a >= 224
	);
}

function parseIpv6(address: string): number[] | null {
	const normalized = stripIpv6Brackets(address).split("%")[0];
	const halves = normalized.split("::");
	if (halves.length > 2) return null;
	const left = parseIpv6Half(halves[0]);
	const right = halves.length === 2 ? parseIpv6Half(halves[1]) : [];
	if (!left || !right) return null;
	const missing = 8 - left.length - right.length;
	if (
		(halves.length === 1 && missing !== 0) ||
		(halves.length === 2 && missing < 1)
	) {
		return null;
	}
	const segments = [...left, ...Array(missing).fill(0), ...right];
	if (segments.length !== 8) return null;
	return segments.flatMap((segment) => [segment >> 8, segment & 0xff]);
}

function parseIpv6Half(value: string): number[] | null {
	if (!value) return [];
	const parts = value.split(":");
	const segments: number[] = [];
	for (const part of parts) {
		if (part.includes(".")) {
			const octets = part.split(".").map(Number);
			if (
				octets.length !== 4 ||
				octets.some(
					(byte) => !Number.isInteger(byte) || byte < 0 || byte > 255,
				)
			) {
				return null;
			}
			segments.push(
				(octets[0] << 8) | octets[1],
				(octets[2] << 8) | octets[3],
			);
		} else {
			if (!/^[0-9a-f]{1,4}$/i.test(part)) return null;
			segments.push(Number.parseInt(part, 16));
		}
	}
	return segments;
}

function stripIpv6Brackets(hostname: string): string {
	return hostname.startsWith("[") && hostname.endsWith("]")
		? hostname.slice(1, -1)
		: hostname;
}

class SafeDestinationError extends Error {}
