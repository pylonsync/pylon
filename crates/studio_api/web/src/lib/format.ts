// Text formatting shared by the pages.

import { ApiError } from "@/lib/pylon";

/** "createdAt" and "created_at" both read "Created at". */
export function humanize(s: string): string {
	const out = s
		.replace(/[_-]/g, " ")
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.toLowerCase();
	return out.charAt(0).toUpperCase() + out.slice(1);
}

/**
 * A short form of an id for a table cell. Ids of 10 characters or fewer stay
 * whole. A longer id shows its first 8 characters and an ellipsis.
 *
 * An id that Pylon generates is a zero-padded timestamp and then a counter,
 * so its first characters are the same in every row. An id that starts with
 * 6 zeros shows an ellipsis and its last 12 characters: the low bits of the
 * timestamp and the counter.
 */
export function shortId(id: string): string {
	if (id.length <= 10) return id;
	return id.startsWith("000000") ? `…${id.slice(-12)}` : `${id.slice(0, 8)}…`;
}

export function clip(s: string, max: number): string {
	return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

export function plural(n: number, one: string, many = `${one}s`): string {
	return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

/** Unix times under this value are seconds. Larger values are milliseconds. */
const SECONDS_LIMIT = 100_000_000_000;

/**
 * Read a date from a Date, an ISO 8601 string, or a Unix time. A Unix time
 * can be a number or digits in a string (the framework tables write
 * `"1795112402Z"`), in seconds or in milliseconds.
 */
export function parseDate(v: unknown): Date | null {
	if (v instanceof Date) return Number.isNaN(v.getTime()) ? null : v;
	if (typeof v === "string" && /^\d{9,13}Z?$/.test(v)) v = Number(v.replace("Z", ""));
	if (typeof v === "number") {
		if (!Number.isFinite(v)) return null;
		return new Date(v < SECONDS_LIMIT ? v * 1000 : v);
	}
	if (typeof v === "string" && v) {
		const d = new Date(v);
		return Number.isNaN(d.getTime()) ? null : d;
	}
	return null;
}

const dateTime = new Intl.DateTimeFormat(undefined, {
	year: "numeric",
	month: "short",
	day: "numeric",
	hour: "2-digit",
	minute: "2-digit",
});

const timeOnly = new Intl.DateTimeFormat(undefined, {
	hour: "2-digit",
	minute: "2-digit",
	second: "2-digit",
});

/** A short date and time in the local time zone, such as "Nov 19, 2026, 14:05". */
export function formatDateTime(d: Date): string {
	return dateTime.format(d);
}

export function formatTime(d: Date): string {
	return timeOnly.format(d);
}

export function relativeTime(d: Date): string {
	const delta = (Date.now() - d.getTime()) / 1000;
	const sign = delta >= 0 ? -1 : 1;
	const abs = Math.abs(delta);
	const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
	if (abs < 60) return rtf.format(sign * Math.round(abs), "second");
	if (abs < 3600) return rtf.format(sign * Math.round(abs / 60), "minute");
	if (abs < 86_400) return rtf.format(sign * Math.round(abs / 3600), "hour");
	if (abs < 86_400 * 30) return rtf.format(sign * Math.round(abs / 86_400), "day");
	if (abs < 86_400 * 365)
		return rtf.format(sign * Math.round(abs / (86_400 * 30)), "month");
	return rtf.format(sign * Math.round(abs / (86_400 * 365)), "year");
}

export function formatBytes(n: number): string {
	const units = ["B", "KB", "MB", "GB", "TB", "PB"];
	let u = 0;
	let v = n;
	while (v >= 1024 && u < units.length - 1) {
		v /= 1024;
		u++;
	}
	return `${v.toFixed(u === 0 ? 0 : 1)} ${units[u]}`;
}

export function formatUptime(secs: number): string {
	const days = Math.floor(secs / 86_400);
	const hours = Math.floor((secs % 86_400) / 3600);
	const minutes = Math.floor((secs % 3600) / 60);
	if (days > 0) return `${days}d ${hours}h`;
	if (hours > 0) return `${hours}h ${minutes}m`;
	if (minutes > 0) return `${minutes}m`;
	return `${Math.floor(secs)}s`;
}

/** One line of JSON for a table cell. */
export function jsonPreview(value: unknown): string {
	try {
		return JSON.stringify(value) ?? String(value);
	} catch {
		return String(value);
	}
}

/** The message to show for a failed request. */
export function errorText(e: unknown): string {
	if (e instanceof ApiError) return `${e.code}: ${e.message}`;
	return e instanceof Error ? e.message : String(e);
}

/** Whether a field of this manifest type holds numbers. Number columns align right. */
export function isNumericType(type: string | undefined): boolean {
	return type === "int" || type === "float";
}

/** A duration in milliseconds as "12 ms", "1.4 s" or "2m 5s". */
export function formatDuration(ms: number): string {
	if (ms < 1000) return `${ms < 10 ? Math.round(ms * 10) / 10 : Math.round(ms)} ms`;
	if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
	const secs = Math.round(ms / 1000);
	return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

/** The value at percentile `p` (0 to 1) of numbers sorted from low to high. */
export function percentile(sorted: number[], p: number): number | null {
	if (sorted.length === 0) return null;
	const rank = Math.max(1, Math.ceil(sorted.length * p));
	return sorted[Math.min(sorted.length, rank) - 1];
}

/**
 * The route of a request path: each segment that is an id becomes `:id`, so
 * requests for different rows count as one route. An id is a segment with a
 * digit that is 8 characters or longer, or a segment of digits.
 */
export function routeOfPath(path: string): string {
	return path
		.split("/")
		.map((seg) =>
			/^\d+$/.test(seg) || (seg.length >= 8 && /\d/.test(seg) && /^[\w:.-]+$/.test(seg))
				? ":id"
				: seg,
		)
		.join("/");
}
