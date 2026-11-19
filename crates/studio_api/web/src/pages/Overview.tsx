import { Legend, RequestChart } from "@/components/charts";
import { Page, PageScroll, Section } from "@/components/page";
import { ErrorBanner } from "@/components/states";
import { Stat, StatStrip } from "@/components/stat-strip";
import { Badge } from "@/components/ui/badge";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { useCounts } from "@/lib/counts";
import { useMetrics } from "@/hooks/use-metrics";
import { formatDuration, formatUptime, plural } from "@/lib/format";
import { onLinkClick } from "@/lib/navigation";
import { MANIFEST } from "@/lib/pylon";
import { routeToPath } from "@/lib/route";

const n = (v: number | undefined) => (v === undefined ? "–" : v.toLocaleString());

export function OverviewPage() {
	const { metrics, error } = useMetrics();
	const counts = useCounts();

	const total = metrics?.requests.total;
	const errorRate =
		metrics === null
			? null
			: metrics.requests.total === 0
				? 0
				: (metrics.requests.error / metrics.requests.total) * 100;
	// Index 0 is the current minute, which is not complete. Index 1 is the
	// last complete minute.
	const series = metrics?.requests.per_minute;
	const lastMinute = series && series.length > 0 ? (series[1] ?? series[0]) : null;

	return (
		<Page title="Overview">
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
						value={n(total)}
						note={lastMinute !== null ? `${lastMinute} in the last minute` : undefined}
					/>
					<Stat
						label="Error rate"
						value={errorRate === null ? "–" : `${errorRate.toFixed(2)}%`}
						tone={
							errorRate === null || errorRate === 0
								? "default"
								: errorRate > 5
									? "bad"
									: "warn"
						}
						note={
							metrics
								? `${n(metrics.requests.error)} of ${n(metrics.requests.total)}`
								: undefined
						}
					/>
					<Stat
						label="WebSocket clients"
						value={n(metrics?.realtime?.ws_connections)}
					/>
					<Stat label="SSE clients" value={n(metrics?.realtime?.sse_connections)} />
					<Stat
						label="Latency p95"
						value={
							metrics?.latency_ms?.p95 != null
								? formatDuration(metrics.latency_ms.p95)
								: "–"
						}
						note="Last 60 minutes"
					/>
					<Stat
						label="Uptime"
						value={metrics ? formatUptime(metrics.uptime_secs) : "–"}
					/>
				</StatStrip>

				<Section
					title="Requests in the last 60 minutes"
					aside={
						<>
							<Legend className="bg-primary" label="OK" />
							<Legend className="bg-status-red-fg" label="Error" />
							<PageLink page="logs">Logs</PageLink>
							<PageLink page="metrics">Metrics</PageLink>
						</>
					}
				>
					<RequestChart
						series={metrics?.requests.per_minute ?? []}
						errors={metrics?.requests.errors_per_minute ?? []}
					/>
				</Section>

				<Section
					title="Jobs"
					aside={
						<>
							{plural(metrics?.jobs?.handlers.length ?? 0, "handler")}
							<PageLink page="jobs">Jobs</PageLink>
						</>
					}
				>
					<CountStrip
						counts={[
							["Pending", metrics?.jobs?.pending],
							["Running", metrics?.jobs?.running],
							["Completed", metrics?.jobs?.completed],
							["Failed", metrics?.jobs?.failed, "bad"],
							["Dead letter", metrics?.jobs?.dead, "bad"],
						]}
					/>
				</Section>
				<Section title="Workflows" aside={<PageLink page="workflows">Workflows</PageLink>}>
					<CountStrip
						counts={[
							["Pending", metrics?.workflows?.pending],
							["Running", metrics?.workflows?.running],
							["Waiting", metrics?.workflows?.waiting],
							["Sleeping", metrics?.workflows?.sleeping],
							["Completed", metrics?.workflows?.completed],
							["Failed", metrics?.workflows?.failed, "bad"],
							["Cancelled", metrics?.workflows?.cancelled],
						]}
					/>
				</Section>

				<Section title="Entities" aside={MANIFEST.entities.length} className="border-b-0">
					{MANIFEST.entities.length === 0 ? (
						<p className="px-3 pb-3 text-xs text-muted-foreground">
							The app has no entities. Declare one with <code>entity()</code> in{" "}
							<code>app.ts</code>.
						</p>
					) : (
						<Table aria-label="Entities" className="border-t">
							<TableHeader className="static">
								<tr>
									<TableHead className="w-64">Name</TableHead>
									<TableHead className="w-24 text-right">Rows</TableHead>
									<TableHead className="w-24 text-right">Fields</TableHead>
									<TableHead className="w-24 text-right">Indexes</TableHead>
									<TableHead>Sync</TableHead>
								</tr>
							</TableHeader>
							<TableBody>
								{MANIFEST.entities.map((e) => {
									const url = routeToPath({ kind: "resource", entity: e.name });
									return (
										<TableRow key={e.name}>
											<TableCell>
												<a
													href={url}
													className="font-medium text-link underline-offset-2 hover:underline"
													onClick={(ev) => onLinkClick(ev, url)}
												>
													{e.name}
												</a>
											</TableCell>
											<TableCell className="text-right">{n(counts[e.name])}</TableCell>
											<TableCell className="text-right">{e.fields.length}</TableCell>
											<TableCell className="text-right">
												{e.indexes?.length ?? 0}
											</TableCell>
											<TableCell>
												{e.crdt ? (
													<Badge variant="blue">CRDT</Badge>
												) : (
													<span className="text-muted-foreground">Row</span>
												)}
											</TableCell>
										</TableRow>
									);
								})}
							</TableBody>
						</Table>
					)}
				</Section>
			</PageScroll>
		</Page>
	);
}

/** A link to another Studio page, for the right side of a section title. */
function PageLink({ page, children }: { page: string; children: React.ReactNode }) {
	const url = routeToPath({ kind: "page", id: page });
	return (
		<a
			href={url}
			className="text-link underline-offset-2 hover:underline"
			onClick={(e) => onLinkClick(e, url)}
		>
			{children}
		</a>
	);
}

/** One strip of counts by state. A count above zero can take a tone. */
function CountStrip({
	counts,
}: {
	counts: Array<[string, number | undefined, "bad"?]>;
}) {
	return (
		<StatStrip className="border-t border-b-0">
			{counts.map(([label, value, tone]) => (
				<Stat
					key={label}
					size="sm"
					label={label}
					value={n(value)}
					tone={tone === "bad" && value ? "bad" : "default"}
					className={value ? undefined : "[&>dd]:font-normal [&>dd]:text-muted-foreground"}
				/>
			))}
		</StatStrip>
	);
}
