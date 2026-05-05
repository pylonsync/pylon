import { useEffect, useMemo, useState } from "react";
import {
	Activity,
	Briefcase,
	Database,
	Network,
	Radio,
	Workflow,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { api, MANIFEST } from "@/lib/pylon";

interface MetricsSnapshot {
	uptime_secs: number;
	requests: {
		total: number;
		ok: number;
		error: number;
		per_minute?: number[];
		errors_per_minute?: number[];
	};
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

const POLL_MS = 5000;

export function OverviewPage() {
	const [metrics, setMetrics] = useState<MetricsSnapshot | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let cancelled = false;
		const tick = () =>
			api<MetricsSnapshot>("/metrics", {
				headers: { Accept: "application/json" },
			})
				.then((m) => {
					if (cancelled) return;
					setMetrics(m);
					setError(null);
				})
				.catch((e) => {
					if (cancelled) return;
					setError(e instanceof Error ? e.message : String(e));
				});
		void tick();
		const id = setInterval(tick, POLL_MS);
		return () => {
			cancelled = true;
			clearInterval(id);
		};
	}, []);

	const errorRate = useMemo(() => {
		if (!metrics) return null;
		const total = metrics.requests.total;
		if (total === 0) return 0;
		return (metrics.requests.error / total) * 100;
	}, [metrics]);

	const reqPerMinNow = useMemo(() => {
		const series = metrics?.requests.per_minute;
		if (!series || series.length === 0) return null;
		// Index 0 is the partial current minute; show the previous full
		// minute as the headline number for stability.
		return series[1] ?? series[0] ?? 0;
	}, [metrics]);

	return (
		<div className="space-y-6">
			<div>
				<h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
				<p className="mt-1 text-sm text-muted-foreground">
					{MANIFEST.name} · v{MANIFEST.version}
					{metrics && (
						<span className="ml-2 text-xs">
							· uptime {formatUptime(metrics.uptime_secs)}
						</span>
					)}
				</p>
			</div>

			{error && (
				<div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
					Couldn&apos;t load /metrics: {error}
				</div>
			)}

			<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
				<StatCard
					label="Requests"
					value={metrics ? metrics.requests.total.toLocaleString() : "—"}
					icon={Activity}
					sub={
						reqPerMinNow !== null
							? `${reqPerMinNow}/min last minute`
							: undefined
					}
				/>
				<StatCard
					label="Error rate"
					value={errorRate === null ? "—" : `${errorRate.toFixed(2)}%`}
					icon={Activity}
					sub={
						metrics
							? `${metrics.requests.error.toLocaleString()} of ${metrics.requests.total.toLocaleString()}`
							: undefined
					}
					tone={errorRate !== null && errorRate > 5 ? "warn" : "default"}
				/>
				<StatCard
					label="WebSocket"
					value={metrics?.realtime?.ws_connections.toString() ?? "—"}
					icon={Radio}
					sub="connected clients"
				/>
				<StatCard
					label="SSE"
					value={metrics?.realtime?.sse_connections.toString() ?? "—"}
					icon={Network}
					sub="streamed clients"
				/>
			</div>

			<Card>
				<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
					<CardTitle className="text-sm font-medium">
						Requests (last 60 minutes)
					</CardTitle>
					<span className="text-xs text-muted-foreground">
						green = ok, red = error
					</span>
				</CardHeader>
				<CardContent>
					<Sparkline
						series={metrics?.requests.per_minute ?? []}
						errors={metrics?.requests.errors_per_minute ?? []}
					/>
				</CardContent>
			</Card>

			<div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
						<CardTitle className="text-sm font-medium flex items-center gap-2">
							<Briefcase className="size-4" /> Jobs
						</CardTitle>
						<span className="text-xs text-muted-foreground">
							{metrics?.jobs?.handlers.length ?? 0} handlers
						</span>
					</CardHeader>
					<CardContent>
						<MiniGrid
							rows={[
								["Pending", metrics?.jobs?.pending],
								["Running", metrics?.jobs?.running],
								["Completed", metrics?.jobs?.completed],
								["Failed", metrics?.jobs?.failed],
								["Dead-letter", metrics?.jobs?.dead],
							]}
						/>
					</CardContent>
				</Card>
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
						<CardTitle className="text-sm font-medium flex items-center gap-2">
							<Workflow className="size-4" /> Workflows
						</CardTitle>
					</CardHeader>
					<CardContent>
						<MiniGrid
							rows={[
								["Pending", metrics?.workflows?.pending],
								["Running", metrics?.workflows?.running],
								["Waiting (event)", metrics?.workflows?.waiting],
								["Sleeping", metrics?.workflows?.sleeping],
								["Completed", metrics?.workflows?.completed],
								["Failed", metrics?.workflows?.failed],
							]}
						/>
					</CardContent>
				</Card>
			</div>

			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<Database className="size-4" /> Entities ({MANIFEST.entities.length})
					</CardTitle>
				</CardHeader>
				<CardContent>
					<div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
						{MANIFEST.entities.map((e) => (
							<div
								key={e.name}
								className="flex items-center justify-between rounded-md border p-3"
							>
								<div className="flex flex-col">
									<span className="text-sm font-medium">{e.name}</span>
									<span className="text-xs text-muted-foreground">
										{e.fields.length} fields
									</span>
								</div>
								{e.crdt && (
									<span className="rounded-md bg-status-blue-bg px-2 py-0.5 text-xs text-status-blue-fg">
										CRDT
									</span>
								)}
							</div>
						))}
					</div>
				</CardContent>
			</Card>
		</div>
	);
}

function StatCard({
	label,
	value,
	icon: Icon,
	sub,
	tone,
}: {
	label: string;
	value: string;
	icon: React.ComponentType<{ className?: string }>;
	sub?: string;
	tone?: "default" | "warn";
}) {
	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium text-muted-foreground">
					{label}
				</CardTitle>
				<Icon className="size-4 text-muted-foreground" />
			</CardHeader>
			<CardContent>
				<div
					className={`text-2xl font-semibold ${tone === "warn" ? "text-status-amber-fg" : ""}`}
				>
					{value}
				</div>
				{sub && (
					<div className="mt-1 text-xs text-muted-foreground">{sub}</div>
				)}
			</CardContent>
		</Card>
	);
}

function MiniGrid({ rows }: { rows: Array<[string, number | undefined]> }) {
	return (
		<div className="grid grid-cols-2 gap-3">
			{rows.map(([label, value]) => (
				<div key={label} className="flex items-baseline justify-between gap-3">
					<span className="text-xs text-muted-foreground">{label}</span>
					<span className="font-mono text-sm tabular-nums">
						{value ?? "—"}
					</span>
				</div>
			))}
		</div>
	);
}

/**
 * Tiny inline SVG sparkline. Index 0 = current (partial) minute, so we
 * reverse for left-to-right flow. Stacked bars: green for ok-share,
 * red on top for error-share.
 */
function Sparkline({
	series,
	errors,
}: {
	series: number[];
	errors: number[];
}) {
	const reversed = [...series].reverse();
	const reversedErr = [...errors].reverse();
	const max = Math.max(1, ...reversed);
	const width = 800;
	const height = 80;
	const barWidth = reversed.length > 0 ? width / reversed.length : 0;
	return (
		<div className="overflow-x-auto">
			<svg
				viewBox={`0 0 ${width} ${height}`}
				preserveAspectRatio="none"
				className="w-full h-20"
			>
				{reversed.map((v, i) => {
					const totalH = (v / max) * (height - 4);
					const errH = ((reversedErr[i] ?? 0) / max) * (height - 4);
					const okH = Math.max(0, totalH - errH);
					const x = i * barWidth + 0.5;
					const w = Math.max(1, barWidth - 1);
					return (
						<g key={i}>
							{okH > 0 && (
								<rect
									x={x}
									y={height - okH - errH}
									width={w}
									height={okH}
									fill="var(--color-status-green-fg, #16a34a)"
									opacity={v === 0 ? 0.15 : 0.85}
								/>
							)}
							{errH > 0 && (
								<rect
									x={x}
									y={height - errH}
									width={w}
									height={errH}
									fill="var(--color-status-red-fg, #dc2626)"
								/>
							)}
						</g>
					);
				})}
			</svg>
			<div className="mt-1 flex justify-between text-[10px] text-muted-foreground">
				<span>60m ago</span>
				<span>now</span>
			</div>
		</div>
	);
}

function formatUptime(secs: number): string {
	const days = Math.floor(secs / 86400);
	const hours = Math.floor((secs % 86400) / 3600);
	const minutes = Math.floor((secs % 3600) / 60);
	if (days > 0) return `${days}d ${hours}h`;
	if (hours > 0) return `${hours}h ${minutes}m`;
	return `${minutes}m`;
}
