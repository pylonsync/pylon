/**
 * How Studio counts a response status. The server counts the same way in
 * `/metrics` (crates/runtime/src/metrics.rs).
 *
 * - A 1xx, 2xx or 3xx status is a success.
 * - A 4xx status is a client error: the server refused the request (not
 *   found, unauthorized, rate limited). Scanners and bad clients make these.
 * - A 5xx status, or a status below 100, is a server error. Only these count
 *   in the error rate.
 */
export type StatusClass = "ok" | "client" | "server";

export function statusClass(status: number): StatusClass {
	if (status >= 100 && status < 400) return "ok";
	if (status >= 400 && status < 500) return "client";
	return "server";
}

export function isServerError(status: number): boolean {
	return statusClass(status) === "server";
}
