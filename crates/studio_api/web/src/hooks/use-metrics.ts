import { useQuery } from "@/hooks/use-query";
import { api } from "@/lib/pylon";

export interface MetricsSnapshot {
	uptime_secs: number;
	requests: {
		total: number;
		ok: number;
		error: number;
		per_minute?: number[];
		errors_per_minute?: number[];
	};
	/**
	 * Latency of the last 60 minutes, in milliseconds. A percentile is the
	 * upper bound of the histogram bucket that holds it. `null` means no
	 * request was timed.
	 */
	latency_ms?: {
		p50: number | null;
		p95: number | null;
		p99: number | null;
		p50_per_minute: Array<number | null>;
		p95_per_minute: Array<number | null>;
		p99_per_minute: Array<number | null>;
	} | null;
	/** The page cache since the server started. */
	ssr_cache?: {
		fresh_hits: number;
		stale_serves: number;
		stale_on_error: number;
		no_page: number;
		stores: number;
		removals: number;
		memory_entries: number;
		memory_bytes: number;
		memory_budget_bytes: number;
	};
	methods?: Record<string, number>;
	jobs?: {
		pending: number;
		running: number;
		completed: number;
		failed: number;
		dead: number;
		handlers: string[];
	};
	workflows?: {
		pending: number;
		running: number;
		waiting: number;
		sleeping: number;
		completed: number;
		failed: number;
		cancelled: number;
	};
	realtime?: {
		ws_connections: number;
		sse_connections: number;
	};
}

export function fetchMetrics(): Promise<MetricsSnapshot> {
	return api<MetricsSnapshot>("/metrics", {
		headers: { Accept: "application/json" },
	});
}

/** Poll `/metrics` as JSON. The last good snapshot stays while a poll fails. */
export function useMetrics(pollMs = 5000) {
	const { data, error } = useQuery("metrics", fetchMetrics, { pollMs });
	return { metrics: data, error };
}
