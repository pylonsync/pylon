import { useState } from "react";
import { RefreshCw } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { CodeBlock } from "@/components/code-block";
import { DetailSheet } from "@/components/detail-sheet";
import { Fact, FactBlock, Facts } from "@/components/facts";
import { Page, Toolbar } from "@/components/page";
import { Dash } from "@/components/renderers";
import { EmptyState, ErrorState, LoadingRows, Spinner } from "@/components/states";
import { Stat, StatStrip } from "@/components/stat-strip";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Tabs, TabsList, TabsPanel, TabsTab } from "@/components/ui/tabs";
import { Tooltip } from "@/components/ui/tooltip";
import { useQuery } from "@/hooks/use-query";
import {
	errorText,
	formatDateTime,
	formatDuration,
	parseDate,
	plural,
	relativeTime,
} from "@/lib/format";
import { api } from "@/lib/pylon";
import { toast } from "@/lib/toast";
import { LockedPage } from "./Locked";

type Job = {
	id: string;
	name: string;
	status: string;
	queue?: string;
	priority?: string;
	payload?: unknown;
	error?: string | null;
	retry_count?: number;
	max_retries?: number;
	delay_secs?: number;
	created_at?: string | null;
	started_at?: string | null;
	completed_at?: string | null;
};

type Stats = {
	pending: number;
	running: number;
	completed: number;
	failed: number;
	dead: number;
	handlers: string[];
};

type Task = {
	name: string;
	enabled: boolean;
	schedule?: string;
	last_run?: number | null;
	next_run?: number | null;
};

const STATUS_VARIANT: Record<string, BadgeProps["variant"]> = {
	pending: "gray",
	running: "blue",
	completed: "green",
	failed: "amber",
	dead: "red",
	cancelled: "outline",
};

const POLL_MS = 3000;

function when(v: unknown): React.ReactNode {
	const d = parseDate(v);
	return d ? <span title={d.toISOString()}>{formatDateTime(d)}</span> : <Dash />;
}

function span(from: unknown, to: unknown): string | null {
	const a = parseDate(from);
	const b = parseDate(to);
	return a && b ? formatDuration(Math.max(0, b.getTime() - a.getTime())) : null;
}

export function JobsPage() {
	const { me } = useAuth();
	const isAdmin = !!me?.is_admin;
	const [tab, setTab] = useState("jobs");
	const [status, setStatus] = useState("all");
	const [openId, setOpenId] = useState<string | null>(null);

	const stats = useQuery("job-stats", () => api<Stats>("/api/jobs/stats"), {
		pollMs: POLL_MS,
		enabled: isAdmin,
	});
	const listUrl = `/api/jobs?limit=200${status === "all" ? "" : `&status=${status}`}`;
	const jobs = useQuery(listUrl, async () => (await api<Job[]>(listUrl)) ?? [], {
		pollMs: POLL_MS,
		enabled: isAdmin && tab === "jobs",
	});
	const dead = useQuery("jobs-dead", async () => (await api<Job[]>("/api/jobs/dead")) ?? [], {
		pollMs: POLL_MS,
		enabled: isAdmin && tab === "dead",
	});
	const tasks = useQuery("scheduler", async () => (await api<Task[]>("/api/scheduler")) ?? [], {
		pollMs: 10_000,
		enabled: isAdmin && tab === "schedules",
	});

	if (!isAdmin) {
		return (
			<Page title="Jobs">
				<LockedPage
					title="Jobs are for admins"
					description="Sign in with an admin account to see the job queue."
				/>
			</Page>
		);
	}

	const current = tab === "dead" ? dead : tab === "schedules" ? tasks : jobs;
	const open =
		openId === null
			? null
			: ([...(jobs.data ?? []), ...(dead.data ?? [])].find((j) => j.id === openId) ?? null);
	const reloadAll = () => Promise.all([stats.reload(), current.reload()]);

	const act = async (path: string, done: string, failed: string) => {
		try {
			await api(path, { method: "POST", body: "{}" });
			toast.success(done);
			await reloadAll();
		} catch (err) {
			toast.error(failed, { description: errorText(err) });
		}
	};

	const s = stats.data;
	return (
		<Page
			title="Jobs"
			actions={
				<Tooltip label="Load the jobs again">
					<Button
						variant="ghost"
						size="icon"
						aria-label="Refresh"
						disabled={current.loading}
						onClick={reloadAll}
					>
						{current.loading ? <Spinner /> : <RefreshCw />}
					</Button>
				</Tooltip>
			}
		>
			<StatStrip>
				<Stat label="Pending" value={s ? s.pending.toLocaleString() : "–"} />
				<Stat label="Running" value={s ? s.running.toLocaleString() : "–"} />
				<Stat label="Completed" value={s ? s.completed.toLocaleString() : "–"} />
				<Stat
					label="Failed"
					value={s ? s.failed.toLocaleString() : "–"}
					tone={s && s.failed > 0 ? "warn" : "default"}
				/>
				<Stat
					label="Dead letter"
					value={s ? s.dead.toLocaleString() : "–"}
					tone={s && s.dead > 0 ? "bad" : "default"}
				/>
			</StatStrip>
			<Tabs value={tab} onValueChange={(v) => v && setTab(v)} className="flex-1">
				<TabsList aria-label="Job views">
					<TabsTab value="jobs">Queue</TabsTab>
					<TabsTab value="dead">
						Dead letter
						{s && s.dead > 0 && <Badge variant="red">{s.dead}</Badge>}
					</TabsTab>
					<TabsTab value="schedules">Schedules</TabsTab>
					<TabsTab value="handlers">Handlers</TabsTab>
				</TabsList>

				<TabsPanel value="jobs" className="flex flex-col">
					<Toolbar>
						<Select
							aria-label="Status"
							value={status}
							onValueChange={setStatus}
							options={[
								{ value: "all", label: "All states" },
								{ value: "pending", label: "Pending" },
								{ value: "running", label: "Running" },
								{ value: "completed", label: "Completed" },
								{ value: "failed", label: "Failed" },
								{ value: "dead", label: "Dead letter" },
							]}
						/>
						<span className="ml-auto text-xs text-muted-foreground tabular-nums">
							{jobs.data ? `The newest ${plural(jobs.data.length, "job")}` : ""}
						</span>
					</Toolbar>
					<JobTable
						query={jobs}
						openId={openId}
						onOpen={setOpenId}
						empty={
							status === "all"
								? "The queue has no jobs. A function enqueues a job with ctx.scheduler, and each schedule enqueues one when it is due."
								: `No job is in the "${status}" state.`
						}
					/>
				</TabsPanel>

				<TabsPanel value="dead" className="flex flex-col">
					<JobTable
						query={dead}
						openId={openId}
						onOpen={setOpenId}
						empty="No job is in the dead-letter queue. A job goes there when it fails and has no retries left."
					/>
				</TabsPanel>

				<TabsPanel value="schedules" className="overflow-auto">
					{tasks.error && tasks.data === null ? (
						<ErrorState
							title="Studio could not load the schedules"
							message={tasks.error}
							onRetry={tasks.reload}
						/>
					) : tasks.data === null ? (
						<LoadingRows />
					) : tasks.data.length === 0 ? (
						<EmptyState
							title="No schedules"
							description="A schedule runs a job on a cron expression."
						/>
					) : (
						<Table aria-label="Schedules">
							<TableHeader>
								<tr>
									<TableHead>Task</TableHead>
									<TableHead className="w-40">Schedule</TableHead>
									<TableHead className="w-24">State</TableHead>
									<TableHead className="w-48">Last run</TableHead>
									<TableHead className="w-48">Next run</TableHead>
									<TableHead className="w-px">
										<span className="sr-only">Actions</span>
									</TableHead>
								</tr>
							</TableHeader>
							<TableBody>
								{tasks.data.map((t) => {
									const next = parseDate(t.next_run);
									return (
										<TableRow key={t.name}>
											<TableCell className="font-mono text-xs">{t.name}</TableCell>
											<TableCell className="font-mono text-xs">
												{t.schedule ?? <Dash />}
											</TableCell>
											<TableCell>
												<Badge variant={t.enabled ? "green" : "gray"}>
													{t.enabled ? "enabled" : "disabled"}
												</Badge>
											</TableCell>
											<TableCell>{t.last_run ? when(t.last_run) : "Not since the start"}</TableCell>
											<TableCell>
												{next ? (
													<span title={next.toISOString()}>
														{formatDateTime(next)}{" "}
														<span className="text-muted-foreground">
															({relativeTime(next)})
														</span>
													</span>
												) : (
													<Dash />
												)}
											</TableCell>
											<TableCell>
												<Button
													size="sm"
													variant="outline"
													onClick={() =>
														act(
															`/api/scheduler/trigger/${encodeURIComponent(t.name)}`,
															`${t.name} is in the queue`,
															"Studio could not run the task",
														)
													}
												>
													Run now
												</Button>
											</TableCell>
										</TableRow>
									);
								})}
							</TableBody>
						</Table>
					)}
				</TabsPanel>

				<TabsPanel value="handlers" className="overflow-auto">
					{!s ? (
						<LoadingRows />
					) : s.handlers.length === 0 ? (
						<EmptyState
							title="No job handlers"
							description="A handler is the code that runs a job of one name."
						/>
					) : (
						<Table aria-label="Job handlers">
							<TableHeader>
								<tr>
									<TableHead>Job name</TableHead>
								</tr>
							</TableHeader>
							<TableBody>
								{[...s.handlers].sort().map((h) => (
									<TableRow key={h}>
										<TableCell className="max-w-none font-mono text-xs">{h}</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					)}
				</TabsPanel>
			</Tabs>

			<DetailSheet
				open={open !== null}
				title={open?.name ?? ""}
				subtitle={open?.id}
				onClose={() => setOpenId(null)}
				footer={
					open?.status === "pending" ? (
						<Button
							variant="destructive"
							onClick={() =>
								act(
									`/api/jobs/${encodeURIComponent(open.id)}/cancel`,
									"The job is cancelled",
									"Studio could not cancel the job",
								)
							}
						>
							Cancel job
						</Button>
					) : open?.status === "dead" ? (
						<Button
							onClick={() =>
								act(
									`/api/jobs/dead/${encodeURIComponent(open.id)}/retry`,
									"The job is in the queue again",
									"Studio could not retry the job",
								)
							}
						>
							Retry job
						</Button>
					) : undefined
				}
			>
				{open && (
					<>
						<Facts className="pt-2">
							<Fact label="State">
								<Badge variant={STATUS_VARIANT[open.status] ?? "gray"}>{open.status}</Badge>
							</Fact>
							<Fact label="Queue">{open.queue ?? <Dash />}</Fact>
							<Fact label="Priority">{open.priority ?? <Dash />}</Fact>
							<Fact label="Attempts">
								{(open.retry_count ?? 0) + (open.started_at ? 1 : 0)} of{" "}
								{(open.max_retries ?? 0) + 1}
							</Fact>
							<Fact label="Created">{when(open.created_at)}</Fact>
							<Fact label="Delay">
								{open.delay_secs ? formatDuration(open.delay_secs * 1000) : "None"}
							</Fact>
							<Fact label="Started">{when(open.started_at)}</Fact>
							<Fact label="Finished">{when(open.completed_at)}</Fact>
							<Fact label="Run time">
								{span(open.started_at, open.completed_at) ?? <Dash />}
							</Fact>
						</Facts>
						{open.error && (
							<FactBlock label="Last error">
								<CodeBlock className="max-h-48 whitespace-pre-wrap" value={open.error} />
							</FactBlock>
						)}
						<FactBlock label="Payload">
							<CodeBlock
								className="max-h-72"
								value={JSON.stringify(open.payload ?? null, null, 2)}
							/>
						</FactBlock>
					</>
				)}
			</DetailSheet>
		</Page>
	);
}

function JobTable({
	query,
	openId,
	onOpen,
	empty,
}: {
	query: { data: Job[] | null; error: string | null; reload: () => unknown };
	openId: string | null;
	onOpen: (id: string) => void;
	empty: string;
}) {
	const { data, error, reload } = query;
	return (
		<div className="min-h-0 flex-1 overflow-auto">
			{error && data === null ? (
				<ErrorState title="Studio could not load the jobs" message={error} onRetry={reload} />
			) : data === null ? (
				<LoadingRows />
			) : data.length === 0 ? (
				<EmptyState title="No jobs" description={empty} />
			) : (
				<Table aria-label="Jobs">
					<TableHeader>
						<tr>
							<TableHead className="w-24">State</TableHead>
							<TableHead>Job</TableHead>
							<TableHead className="w-24">Queue</TableHead>
							<TableHead className="w-24 text-right">Attempts</TableHead>
							<TableHead className="w-44">Created</TableHead>
							<TableHead className="w-24 text-right">Run time</TableHead>
							<TableHead>Error</TableHead>
						</tr>
					</TableHeader>
					<TableBody>
						{data.map((j) => (
							<TableRow
								key={j.id}
								tabIndex={0}
								data-current={j.id === openId}
								className="cursor-default"
								onClick={() => onOpen(j.id)}
								onKeyDown={(e) => {
									if (e.key !== "Enter" && e.key !== " ") return;
									e.preventDefault();
									onOpen(j.id);
								}}
							>
								<TableCell>
									<Badge variant={STATUS_VARIANT[j.status] ?? "gray"}>{j.status}</Badge>
								</TableCell>
								<TableCell className="font-mono text-xs">{j.name}</TableCell>
								<TableCell className="text-muted-foreground">{j.queue ?? <Dash />}</TableCell>
								<TableCell className="text-right">
									{(j.retry_count ?? 0) + (j.started_at ? 1 : 0)} / {(j.max_retries ?? 0) + 1}
								</TableCell>
								<TableCell>{when(j.created_at)}</TableCell>
								<TableCell className="text-right">
									{span(j.started_at, j.completed_at) ?? <Dash />}
								</TableCell>
								<TableCell className="text-status-red-fg" title={j.error ?? undefined}>
									{j.error ?? <Dash />}
								</TableCell>
							</TableRow>
						))}
					</TableBody>
				</Table>
			)}
		</div>
	);
}
