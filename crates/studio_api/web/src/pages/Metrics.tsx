import { useMemo } from "react";

import { useAuth } from "@/auth/auth";
import { LatencyChart, Legend, RequestChart } from "@/components/charts";
import { Page, PageScroll, Section } from "@/components/page";
import { ErrorBanner } from "@/components/states";
import { Stat, StatStrip } from "@/components/stat-strip";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { useMetrics } from "@/hooks/use-metrics";
import { useQuery } from "@/hooks/use-query";
import { formatDuration, percentile, routeOfPath } from "@/lib/format";
import { api } from "@/lib/pylon";
import { LockedPage } from "./Locked";

type RequestRow = {
	method: string;
	path: string;
	status: number;
	cpu_ms: number;
	studio?: boolean;
};
type Trace = {
	fn_name?: string;
	fn_type?: string;
	duration_ms?: number;
	outcome?: { status?: string };
};

type Breakdown = {
	key: string;
	label: string;
	note?: string;
	count: number;
	errors: number;
	avg: number;
	p95: number;
	max: number;
};

/** Group samples by key and compute the count, errors and latency of each group. */
function breakdown<T>(
	items: T[],
	keyOf: (item: T) => { key: string; label: string; note?: string },
	durationOf: (item: T) => number,
	failed: (item: T) => boolean,
): Breakdown[] {
	const groups = new Map<
		string,
		{ label: string; note?: string; durations: number[]; errors: number }
	>();
	for (const item of items) {
		const { key, label, note } = keyOf(item);
		let g = groups.get(key);
		if (!g) {
			g = { label, note, durations: [], errors: 0 };
			groups.set(key, g);
		}
		g.durations.push(durationOf(item));
		if (failed(item)) g.errors++;
	}
	return [...groups.entries()]
		.map(([key, g]) => {
			const sorted = g.durations.sort((a, b) => a - b);
			const total = sorted.reduce((sum, d) => sum + d, 0);
			return {
				key,
				label: g.label,
				note: g.note,
				count: sorted.length,
				errors: g.errors,
				avg: total / sorted.length,
				p95: percentile(sorted, 0.95) ?? 0,
				max: sorted[sorted.length - 1] ?? 0,
			};
		})
		.sort((a, b) => b.count - a.count);
}

const ms = (v: number | null | undefined) => (v == null ? "–" : formatDuration(v));
const n = (v: number | undefined) => (v === undefined ? "–" : v.toLocaleString());

export function MetricsPage() {
	const { me } = useAuth();
	const isAdmin = !!me?.is_admin;
	const { metrics, error } = useMetrics();
	const samples = useQuery(
		"metrics-samples",
		async () => {
			const [log, traces] = await Promise.all([
				api<{ rows?: RequestRow[] }>("/admin/logs/tail"),
				api<Trace[] | { data?: Trace[] }>("/admin/fn/traces"),
			]);
			return {
				// The requests that Studio makes are left out.
				requests: (log?.rows ?? []).filter((r) => !r.studio),
				traces: Array.isArray(traces) ? traces : (traces?.data ?? []),
			};
		},
		{ pollMs: 10_000, enabled: isAdmin },
	);

	const routes = useMemo(
		() =>
			breakdown(
				samples.data?.requests ?? [],
				(r) => {
					const route = routeOfPath(r.path);
					return { key: `${r.method} ${route}`, label: route, note: r.method };
				},
				(r) => r.cpu_ms,
				(r) => r.status >= 400 || r.status < 100,
			),
		[samples.data],
	);
	const functions = useMemo(
		() =>
			breakdown(
				samples.data?.traces ?? [],
				(t) => ({ key: t.fn_name ?? "?", label: t.fn_name ?? "?", note: t.fn_type }),
				(t) => t.duration_ms ?? 0,
				(t) => t.outcome?.status !== undefined && t.outcome.status !== "ok",
			),
		[samples.data],
	);

	if (!isAdmin) {
		return (
			<Page title="Metrics">
				<LockedPage
					title="Metrics are for admins"
					description="Sign in with an admin account to see the metrics."
				/>
			</Page>
		);
	}

	const requests = metrics?.requests;
	const errorRate =
		!requests || requests.total === 0 ? null : (requests.error / requests.total) * 100;
	const lastMinute = requests?.per_minute?.[1] ?? requests?.per_minute?.[0];
	const latency = metrics?.latency_ms;
	const methods = Object.entries(metrics?.methods ?? {});
	const methodTotal = methods.reduce((sum, [, c]) => sum + c, 0);

	return (
		<Page title="Metrics">
			<PageScroll>
				{error && (
					<ErrorBanner>
						Studio could not load the metrics: {error}. The numbers show the last
						good response.
					</ErrorBanner>
				)}
				<StatStrip>
					<Stat
						label="Requests"
						value={n(requests?.total)}
						note={lastMinute !== undefined ? `${lastMinute} in the last minute` : undefined}
					/>
					<Stat
						label="Error rate"
						value={errorRate === null ? "–" : `${errorRate.toFixed(2)}%`}
						tone={!errorRate ? "default" : errorRate > 5 ? "bad" : "warn"}
						note={requests ? `${n(requests.error)} errors` : undefined}
					/>
					<Stat label="Latency p50" value={ms(latency?.p50)} note="Last 60 minutes" />
					<Stat label="Latency p95" value={ms(latency?.p95)} />
					<Stat label="Latency p99" value={ms(latency?.p99)} />
				</StatStrip>
				<StatStrip>
					<Stat size="sm" label="WebSocket clients" value={n(metrics?.realtime?.ws_connections)} />
					<Stat size="sm" label="SSE clients" value={n(metrics?.realtime?.sse_connections)} />
					<Stat size="sm" label="Jobs pending" value={n(metrics?.jobs?.pending)} />
					<Stat size="sm" label="Jobs running" value={n(metrics?.jobs?.running)} />
					<Stat size="sm" label="Workflows running" value={n(metrics?.workflows?.running)} />
				</StatStrip>

				<div className="grid lg:grid-cols-2">
					<Section
						title="Requests for each minute"
						className="lg:border-r"
						aside={
							<>
								<Legend className="bg-primary" label="OK" />
								<Legend className="bg-status-red-fg" label="Error" />
							</>
						}
					>
						<RequestChart
							series={requests?.per_minute ?? []}
							errors={requests?.errors_per_minute ?? []}
						/>
					</Section>
					<Section
						title="Latency for each minute"
						aside={
							<>
								<Legend className="bg-primary" label="p50" />
								<Legend className="bg-status-amber-fg/45" label="p95" />
							</>
						}
					>
						<LatencyChart
							p50={latency?.p50_per_minute ?? []}
							p95={latency?.p95_per_minute ?? []}
						/>
					</Section>
				</div>

				<Section
					title="Routes"
					aside={`The last ${(samples.data?.requests.length ?? 0).toLocaleString()} app requests`}
				>
					<BreakdownTable
						label="Routes"
						nameHeader="Route"
						rows={routes}
						loading={samples.data === null && !samples.error}
						empty="The request log has no app requests. Open the app or call the API."
					/>
				</Section>
				<Section
					title="Functions"
					aside={`The last ${(samples.data?.traces.length ?? 0).toLocaleString()} calls`}
				>
					<BreakdownTable
						label="Functions"
						nameHeader="Function"
						rows={functions}
						loading={samples.data === null && !samples.error}
						empty="The server has no function traces. Call a function to record one."
					/>
				</Section>
				<Section title="Requests by method" aside="Since the server started">
					<Table aria-label="Requests by method" className="border-t">
						<TableHeader className="static">
							<tr>
								<TableHead className="w-64">Method</TableHead>
								<TableHead className="w-28 text-right">Requests</TableHead>
								<TableHead className="w-28 text-right">Share</TableHead>
								<TableHead />
							</tr>
						</TableHeader>
						<TableBody>
							{methods.map(([method, count]) => (
								<TableRow key={method}>
									<TableCell className="font-mono text-xs">{method}</TableCell>
									<TableCell className="text-right">{count.toLocaleString()}</TableCell>
									<TableCell className="text-right text-muted-foreground">
										{methodTotal > 0 ? `${((count / methodTotal) * 100).toFixed(1)}%` : "–"}
									</TableCell>
									<TableCell />
								</TableRow>
							))}
						</TableBody>
					</Table>
				</Section>
				<p className="px-3 py-3 text-xs text-muted-foreground">
					Latency is the time from the first byte of a request to the response,
					in histogram buckets: a percentile shows the upper bound of its
					bucket. The route and function tables come from the request log and
					the function traces, which hold the most recent items only. The server
					does not report page cache or database statistics.
				</p>
			</PageScroll>
		</Page>
	);
}

function BreakdownTable({
	label,
	nameHeader,
	rows,
	loading,
	empty,
}: {
	label: string;
	nameHeader: string;
	rows: Breakdown[];
	loading: boolean;
	empty: string;
}) {
	if (rows.length === 0) {
		return (
			<p className="px-3 pb-3 text-xs text-muted-foreground">
				{loading ? "Loading" : empty}
			</p>
		);
	}
	return (
		<div className="overflow-x-auto border-t">
			<Table aria-label={label}>
				<TableHeader className="static">
					<tr>
						<TableHead>{nameHeader}</TableHead>
						<TableHead className="w-24 text-right">Count</TableHead>
						<TableHead className="w-24 text-right">Errors</TableHead>
						<TableHead className="w-24 text-right">Average</TableHead>
						<TableHead className="w-24 text-right">p95</TableHead>
						<TableHead className="w-24 text-right">Max</TableHead>
					</tr>
				</TableHeader>
				<TableBody>
					{rows.slice(0, 50).map((r) => (
						<TableRow key={r.key}>
							<TableCell className="max-w-none font-mono text-xs">
								{r.note && (
									<span className="mr-2 inline-block w-14 text-muted-foreground">
										{r.note}
									</span>
								)}
								{r.label}
							</TableCell>
							<TableCell className="text-right">{r.count.toLocaleString()}</TableCell>
							<TableCell
								className={
									r.errors > 0 ? "text-right text-status-red-fg" : "text-right text-muted-foreground"
								}
							>
								{r.errors.toLocaleString()}
							</TableCell>
							<TableCell className="text-right">{formatDuration(r.avg)}</TableCell>
							<TableCell className="text-right">{formatDuration(r.p95)}</TableCell>
							<TableCell className="text-right">{formatDuration(r.max)}</TableCell>
						</TableRow>
					))}
				</TableBody>
			</Table>
		</div>
	);
}
