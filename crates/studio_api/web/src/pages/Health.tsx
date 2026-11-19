import { RefreshCw } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { Fact, Facts } from "@/components/facts";
import { Page, PageScroll, Section } from "@/components/page";
import { ErrorBanner, Spinner } from "@/components/states";
import { Stat, StatStrip } from "@/components/stat-strip";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Tooltip } from "@/components/ui/tooltip";
import { fetchMetrics } from "@/hooks/use-metrics";
import { useQuery } from "@/hooks/use-query";
import { formatBytes, formatDuration, formatUptime } from "@/lib/format";
import { onLinkClick } from "@/lib/navigation";
import { api } from "@/lib/pylon";
import { routeToPath } from "@/lib/route";
import { LockedPage } from "./Locked";

type Probe = { ok: boolean; latency_ms: number; error: string | null };

/** The answer of `GET /admin/health`. */
type HealthReport = {
	/** `ready`, `warming` (the first page render is not done) or `draining`. */
	status: string;
	pylon_version: string;
	app: { name: string; version: string };
	uptime_secs: number;
	database: {
		backend: string;
		probe: Probe | null;
		/** SQLite: the file sizes. Postgres: pool use and the database size. */
		size:
			| { path: string; file_bytes: number; wal_bytes: number; total_bytes: number }
			| {
					pool_size: number;
					pool_open: number;
					pool_idle: number;
					pool_in_use: number;
					database_bytes: number | null;
					error: string | null;
			  }
			| null;
	};
	/** The file system that holds the database, or the working directory. */
	disk: { path: string; total_bytes: number; available_bytes: number } | null;
	file_storage:
		| { provider: "local"; dir: string; files: number; bytes: number; scanned_secs_ago: number }
		| { provider: "local"; dir: string; scanning: true }
		| { provider: string };
	functions: Probe | null;
	jobs: {
		stats: { pending: number; running: number; failed: number; dead: number };
		lag_secs: number | null;
		persistent: boolean;
	};
	scheduler: { leader: boolean; tasks: number };
	realtime: {
		ws_connections: number;
		sse_connections: number;
		change_seq: number;
		changes_retained: number;
	};
	memory: {
		total_bytes: number;
		available_bytes: number;
		process_rss_bytes: number;
		children_rss_bytes: number;
		children: number;
	} | null;
};

type State = "ok" | "warn" | "bad" | "none";

/** Less than 10% free is a warning, less than 3% a problem. */
function diskCheck(disk: HealthReport["disk"]): Check {
	if (!disk) {
		return { name: "Disk", state: "none", detail: "The server cannot read disk space on this platform" };
	}
	const free = disk.total_bytes > 0 ? disk.available_bytes / disk.total_bytes : 1;
	return {
		name: "Disk",
		state: free < 0.03 ? "bad" : free < 0.1 ? "warn" : "ok",
		detail: `${formatBytes(disk.available_bytes)} free of ${formatBytes(disk.total_bytes)} (${(free * 100).toFixed(0)}%) on the disk that holds ${disk.path}`,
	};
}

type Check = {
	name: string;
	state: State;
	detail: string;
	/** The page that shows the detail. */
	page?: string;
};

const CHECK_BADGE: Record<State, { variant: BadgeProps["variant"]; label: string }> = {
	ok: { variant: "green", label: "ok" },
	warn: { variant: "amber", label: "warning" },
	bad: { variant: "red", label: "problem" },
	none: { variant: "gray", label: "no data" },
};

const POLL_MS = 10_000;

function databaseSize(db: HealthReport["database"]): string {
	const size = db.size;
	if (!size) return "In memory";
	if ("file_bytes" in size) {
		return `${formatBytes(size.total_bytes)} (SQLite file ${formatBytes(size.file_bytes)}, log ${formatBytes(size.wal_bytes)})`;
	}
	if (size.database_bytes === null) return `Postgres. The size is not available: ${size.error ?? "no answer"}`;
	return `${formatBytes(size.database_bytes)} (Postgres)`;
}

function fileStorage(fs: HealthReport["file_storage"]): string {
	if (fs.provider !== "local") return `Stored by ${fs.provider}. The server does not count them`;
	if ("scanning" in fs) return `Counting the files in ${fs.dir}`;
	if (!("files" in fs)) return "Not available";
	return `${fs.files.toLocaleString()} ${fs.files === 1 ? "file" : "files"}, ${formatBytes(fs.bytes)} in ${fs.dir}`;
}

function probeCheck(name: string, probe: Probe | null, absent: string, slowMs: number): Check {
	if (!probe) return { name, state: "none", detail: absent };
	if (!probe.ok) return { name, state: "bad", detail: probe.error ?? "The probe failed" };
	return {
		name,
		state: probe.latency_ms > slowMs ? "warn" : "ok",
		detail: `Answered in ${formatDuration(probe.latency_ms)}`,
	};
}

export function HealthPage() {
	const { me } = useAuth();
	const isAdmin = !!me?.is_admin;
	const { data, error, loading, reload } = useQuery(
		"health",
		async () => {
			const report = await api<HealthReport>("/admin/health");
			// The report answered. When the metrics request fails, the page
			// shows the report with no request numbers.
			const metrics = await fetchMetrics().catch(() => null);
			return { report, metrics };
		},
		{ pollMs: POLL_MS, enabled: isAdmin },
	);

	if (!isAdmin) {
		return (
			<Page title="Health">
				<LockedPage
					title="Health is for admins"
					description="Sign in with an admin account to see the server health."
				/>
			</Page>
		);
	}

	const report = data?.report ?? null;
	const metrics = data?.metrics ?? null;

	const recent = (metrics?.requests.per_minute ?? []).reduce((a, b) => a + b, 0);
	const recentErrors = (metrics?.requests.errors_per_minute ?? []).reduce((a, b) => a + b, 0);
	const errorRate = recent > 0 ? (recentErrors / recent) * 100 : 0;
	const p95 = metrics?.latency_ms?.p95 ?? null;
	const jobs = report?.jobs;

	const database = probeCheck(
		"Database",
		report?.database.probe ?? null,
		"The app has no entities, so there is no table to read",
		250,
	);
	const checks: Check[] = report
		? [
				{
					name: "Readiness",
					state: report.status === "ready" ? "ok" : report.status === "draining" ? "bad" : "warn",
					detail:
						report.status === "ready"
							? "The server takes traffic (GET /health/ready answers 200)"
							: report.status === "draining"
								? "The server shuts down and takes no new requests"
								: "The server starts. The first page render is not done",
				},
				{
					...database,
					detail: `${report.database.backend}. ${database.detail}`,
				},
				{
					...probeCheck(
						"Functions runtime",
						report.functions,
						"The app has no functions runtime",
						250,
					),
					page: "functions",
				},
				{
					name: "Job queue",
					page: "jobs",
					state:
						(jobs?.stats.dead ?? 0) > 0 || (jobs?.lag_secs ?? 0) > 300
							? "bad"
							: (jobs?.stats.failed ?? 0) > 0 || (jobs?.lag_secs ?? 0) > 30
								? "warn"
								: "ok",
					detail: `${
						jobs?.lag_secs == null
							? "No job waits past its time"
							: `The oldest due job waits ${formatDuration(jobs.lag_secs * 1000)}`
					}. ${jobs?.stats.pending ?? 0} pending, ${jobs?.stats.running ?? 0} running, ${jobs?.stats.failed ?? 0} failed, ${jobs?.stats.dead ?? 0} in the dead-letter queue. ${
						jobs?.persistent ? "The queue is stored" : "The queue is in memory only"
					}`,
				},
				diskCheck(report.disk),
				{
					name: "Scheduler",
					page: "jobs",
					state: report.scheduler.leader ? "ok" : "none",
					detail: report.scheduler.leader
						? `This machine runs the cron schedules (${report.scheduler.tasks} tasks)`
						: `Another machine runs the cron schedules (${report.scheduler.tasks} tasks)`,
				},
				{
					name: "Realtime",
					page: "sync",
					state: "ok",
					detail: `${report.realtime.ws_connections} WebSocket clients, ${report.realtime.sse_connections} SSE clients. Change sequence ${report.realtime.change_seq.toLocaleString()}`,
				},
				{
					name: "Request errors",
					page: "logs",
					state: !metrics ? "none" : errorRate > 5 ? "bad" : recentErrors > 0 ? "warn" : "ok",
					detail: metrics
						? `${recentErrors.toLocaleString()} of ${recent.toLocaleString()} requests failed in the last 60 minutes (${errorRate.toFixed(1)}%)`
						: "Metrics are not loaded",
				},
				{
					name: "Latency",
					page: "metrics",
					state: p95 === null ? "none" : p95 > 2500 ? "bad" : p95 > 500 ? "warn" : "ok",
					detail:
						p95 === null
							? "No timed request in the last 60 minutes"
							: `p95 is ${formatDuration(p95)} in the last 60 minutes`,
				},
			]
		: [];
	const problems = checks.filter((c) => c.state === "bad").length;
	const warnings = checks.filter((c) => c.state === "warn").length;
	const memory = report?.memory ?? null;

	return (
		<Page
			title="Health"
			actions={
				<Tooltip label="Run the checks again">
					<Button
						variant="ghost"
						size="icon"
						aria-label="Refresh"
						disabled={loading}
						onClick={reload}
					>
						{loading ? <Spinner /> : <RefreshCw />}
					</Button>
				</Tooltip>
			}
		>
			<PageScroll>
				{error && (
					<ErrorBanner>
						Studio could not load the health report: {error}. Make sure the server
						runs, then refresh.
					</ErrorBanner>
				)}
				<StatStrip>
					<Stat
						label="Checks"
						value={
							!report
								? "–"
								: problems > 0
									? `${problems} ${problems === 1 ? "problem" : "problems"}`
									: warnings > 0
										? `${warnings} ${warnings === 1 ? "warning" : "warnings"}`
										: "All ok"
						}
						tone={!report ? "default" : problems > 0 ? "bad" : warnings > 0 ? "warn" : "good"}
					/>
					<Stat
						label="State"
						value={report?.status ?? "–"}
						tone={!report ? "default" : report.status === "ready" ? "good" : "warn"}
					/>
					<Stat label="Uptime" value={report ? formatUptime(report.uptime_secs) : "–"} />
					<Stat label="Pylon version" value={report?.pylon_version ?? "–"} />
					<Stat
						label="App version"
						value={report?.app.version ?? "–"}
						note={report?.app.name}
					/>
				</StatStrip>

				<Section title="Checks">
					{checks.length === 0 ? (
						<p className="px-3 pb-3 text-xs text-muted-foreground">
							{error ? "The checks did not run." : "Loading"}
						</p>
					) : (
						<Table aria-label="Checks" className="border-t">
							<TableHeader className="static">
								<tr>
									<TableHead className="w-24">Result</TableHead>
									<TableHead className="w-48">Check</TableHead>
									<TableHead>Detail</TableHead>
								</tr>
							</TableHeader>
							<TableBody>
								{checks.map((c) => {
									const url = c.page ? routeToPath({ kind: "page", id: c.page }) : null;
									return (
										<TableRow key={c.name}>
											<TableCell>
												<Badge variant={CHECK_BADGE[c.state].variant}>
													{CHECK_BADGE[c.state].label}
												</Badge>
											</TableCell>
											<TableCell>
												{url ? (
													<a
														href={url}
														className="text-link underline-offset-2 hover:underline"
														onClick={(e) => onLinkClick(e, url)}
													>
														{c.name}
													</a>
												) : (
													c.name
												)}
											</TableCell>
											<TableCell className="max-w-none whitespace-normal text-muted-foreground">
												{c.detail}
											</TableCell>
										</TableRow>
									);
								})}
							</TableBody>
						</Table>
					)}
				</Section>

				<Section title="Memory">
					{memory ? (
						<Facts className="max-w-xl">
							<Fact label="Machine">
								{formatBytes(memory.total_bytes - memory.available_bytes)} of{" "}
								{formatBytes(memory.total_bytes)} in use
							</Fact>
							<Fact label="Pylon process">{formatBytes(memory.process_rss_bytes)}</Fact>
							<Fact label="Function runners">
								{formatBytes(memory.children_rss_bytes)} in {memory.children}{" "}
								{memory.children === 1 ? "process" : "processes"}
							</Fact>
						</Facts>
					) : (
						<p className="px-3 pb-3 text-xs text-muted-foreground">
							{report
								? "The server cannot read memory numbers on this platform."
								: "Loading"}
						</p>
					)}
				</Section>

				<Section title="Storage">
					{report ? (
						<Facts className="max-w-xl">
							<Fact label="Database">{databaseSize(report.database)}</Fact>
							{report.database.size && "pool_size" in report.database.size && (
								<Fact label="Connections">
									{report.database.size.pool_in_use} in use, {report.database.size.pool_open} open of{" "}
									{report.database.size.pool_size}
								</Fact>
							)}
							<Fact label="Disk">
								{report.disk
									? `${formatBytes(report.disk.total_bytes - report.disk.available_bytes)} of ${formatBytes(report.disk.total_bytes)} in use`
									: "Not available on this platform"}
							</Fact>
							<Fact label="Files">{fileStorage(report.file_storage)}</Fact>
						</Facts>
					) : (
						<p className="px-3 pb-3 text-xs text-muted-foreground">Loading</p>
					)}
				</Section>
			</PageScroll>
		</Page>
	);
}
