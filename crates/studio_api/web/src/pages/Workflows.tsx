import { useState } from "react";
import { RefreshCw } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { CodeBlock } from "@/components/code-block";
import { ConfirmDialog } from "@/components/confirm-dialog";
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
import { useMetrics } from "@/hooks/use-metrics";
import { useQuery } from "@/hooks/use-query";
import {
	errorText,
	formatDateTime,
	formatDuration,
	humanize,
	parseDate,
	plural,
} from "@/lib/format";
import { api } from "@/lib/pylon";
import { toast } from "@/lib/toast";
import { LockedPage } from "./Locked";

type Step = {
	step_id: string;
	name: string;
	status: string;
	output?: unknown;
	error?: string | null;
	started_at?: string | null;
	completed_at?: string | null;
	duration_ms?: number | null;
	retry_count?: number;
};

type Run = {
	id: string;
	name: string;
	status: string;
	key?: string | null;
	input?: unknown;
	output?: unknown;
	error?: string | null;
	cancel_reason?: string | null;
	steps: Step[];
	created_at?: string | null;
	started_at?: string | null;
	completed_at?: string | null;
	wake_at?: number | null;
	waiting_for?: string | null;
	wait_deadline?: number | null;
	pending_events?: Array<{ seq: number; event: string; data?: unknown; received_at?: string }>;
};

type Definition = {
	name: string;
	description?: string;
	file?: string;
	max_retries?: number;
	step_timeout_secs?: number;
};

const STATUS_VARIANT: Record<string, BadgeProps["variant"]> = {
	Pending: "gray",
	Running: "blue",
	Sleeping: "outline",
	WaitingForEvent: "amber",
	Completed: "green",
	Failed: "red",
	Cancelled: "outline",
};
const TERMINAL = new Set(["Completed", "Failed", "Cancelled"]);
/** The values of the `status` filter of `GET /api/workflows`, with their labels. */
const STATUS_FILTERS = [
	{ value: "pending", label: "Pending" },
	{ value: "running", label: "Running" },
	{ value: "waiting", label: "Waiting for event" },
	{ value: "sleeping", label: "Sleeping" },
	{ value: "completed", label: "Completed" },
	{ value: "failed", label: "Failed" },
	{ value: "cancelled", label: "Cancelled" },
];

function when(v: unknown): React.ReactNode {
	const d = parseDate(v);
	return d ? <span title={d.toISOString()}>{formatDateTime(d)}</span> : <Dash />;
}

function StatusBadge({ status }: { status: string }) {
	return <Badge variant={STATUS_VARIANT[status] ?? "gray"}>{humanize(status).toLowerCase()}</Badge>;
}

export function WorkflowsPage() {
	const { me } = useAuth();
	const isAdmin = !!me?.is_admin;
	const { metrics } = useMetrics();
	const [tab, setTab] = useState("runs");
	const [status, setStatus] = useState("all");
	const [name, setName] = useState("all");
	const [openId, setOpenId] = useState<string | null>(null);
	const [cancelling, setCancelling] = useState<Run | null>(null);

	const definitions = useQuery(
		"workflow-definitions",
		async () => (await api<Definition[]>("/api/workflows/definitions")) ?? [],
		{ enabled: isAdmin },
	);
	const params = new URLSearchParams({ limit: "200" });
	if (status !== "all") params.set("status", status);
	if (name !== "all") params.set("name", name);
	const listUrl = `/api/workflows?${params.toString()}`;
	const runs = useQuery(listUrl, async () => (await api<Run[]>(listUrl)) ?? [], {
		pollMs: 3000,
		enabled: isAdmin,
	});

	if (!isAdmin) {
		return (
			<Page title="Workflows">
				<LockedPage
					title="Workflows are for admins"
					description="Sign in with an admin account to see the workflow runs."
				/>
			</Page>
		);
	}

	const open = openId ? (runs.data?.find((r) => r.id === openId) ?? null) : null;
	const counts = metrics?.workflows;
	const c = (v: number | undefined) => (v === undefined ? "–" : v.toLocaleString());

	const cancel = async (run: Run) => {
		try {
			await api(`/api/workflows/${encodeURIComponent(run.id)}/cancel`, {
				method: "POST",
				body: JSON.stringify({ reason: "Cancelled from Studio" }),
			});
			toast.success("The run is cancelled");
			await runs.reload();
		} catch (err) {
			toast.error("Studio could not cancel the run", { description: errorText(err) });
		}
	};

	return (
		<Page
			title="Workflows"
			actions={
				<Tooltip label="Load the runs again">
					<Button
						variant="ghost"
						size="icon"
						aria-label="Refresh"
						disabled={runs.loading}
						onClick={runs.reload}
					>
						{runs.loading ? <Spinner /> : <RefreshCw />}
					</Button>
				</Tooltip>
			}
		>
			<StatStrip>
				<Stat size="sm" label="Pending" value={c(counts?.pending)} />
				<Stat size="sm" label="Running" value={c(counts?.running)} />
				<Stat size="sm" label="Waiting" value={c(counts?.waiting)} />
				<Stat size="sm" label="Sleeping" value={c(counts?.sleeping)} />
				<Stat size="sm" label="Completed" value={c(counts?.completed)} />
				<Stat
					size="sm"
					label="Failed"
					value={c(counts?.failed)}
					tone={counts?.failed ? "bad" : "default"}
				/>
				<Stat size="sm" label="Cancelled" value={c(counts?.cancelled)} />
			</StatStrip>
			<Tabs value={tab} onValueChange={(v) => v && setTab(v)} className="flex-1">
				<TabsList aria-label="Workflow views">
					<TabsTab value="runs">Runs</TabsTab>
					<TabsTab value="definitions">
						Definitions
						<span className="text-xs text-muted-foreground tabular-nums">
							{definitions.data?.length ?? 0}
						</span>
					</TabsTab>
				</TabsList>
				<TabsPanel value="runs" className="flex flex-col">
					<Toolbar>
						<Select
							aria-label="Status"
							value={status}
							onValueChange={setStatus}
							options={[
								{ value: "all", label: "All states" },
								...STATUS_FILTERS,
							]}
						/>
						<Select
							aria-label="Workflow"
							value={name}
							onValueChange={setName}
							options={[
								{ value: "all", label: "All workflows" },
								...(definitions.data ?? []).map((d) => ({ value: d.name, label: d.name })),
							]}
						/>
						<span className="ml-auto text-xs text-muted-foreground tabular-nums">
							{runs.data ? plural(runs.data.length, "run") : ""}
						</span>
					</Toolbar>
					<div className="min-h-0 flex-1 overflow-auto">
						{runs.error && runs.data === null ? (
							<ErrorState
								title="Studio could not load the workflow runs"
								message={runs.error}
								onRetry={runs.reload}
							/>
						) : runs.data === null ? (
							<LoadingRows />
						) : runs.data.length === 0 ? (
							<EmptyState
								title={status === "all" && name === "all" ? "No workflow runs" : "No runs match"}
								description={
									(definitions.data?.length ?? 0) === 0
										? "The app has no workflows. Add a file to the workflows/ directory, then start a run with ctx.workflows.start."
										: "A function starts a run with ctx.workflows.start."
								}
							/>
						) : (
							<Table aria-label="Workflow runs">
								<TableHeader>
									<tr>
										<TableHead className="w-32">State</TableHead>
										<TableHead>Workflow</TableHead>
										<TableHead>Run</TableHead>
										<TableHead className="w-20 text-right">Steps</TableHead>
										<TableHead className="w-44">Started</TableHead>
										<TableHead>Waiting for, or error</TableHead>
									</tr>
								</TableHeader>
								<TableBody>
									{runs.data.map((r) => (
										<TableRow
											key={r.id}
											tabIndex={0}
											data-current={r.id === openId}
											className="cursor-default"
											onClick={() => setOpenId(r.id)}
											onKeyDown={(e) => {
												if (e.key !== "Enter" && e.key !== " ") return;
												e.preventDefault();
												setOpenId(r.id);
											}}
										>
											<TableCell>
												<StatusBadge status={r.status} />
											</TableCell>
											<TableCell className="font-mono text-xs">{r.name}</TableCell>
											<TableCell className="font-mono text-xs text-muted-foreground">
												{r.key ?? r.id}
											</TableCell>
											<TableCell className="text-right">{r.steps.length}</TableCell>
											<TableCell>{when(r.started_at ?? r.created_at)}</TableCell>
											<TableCell
												className={r.error ? "text-status-red-fg" : undefined}
												title={r.error ?? undefined}
											>
												{r.error ??
													(r.waiting_for ? (
														<>
															event <code className="text-xs">{r.waiting_for}</code>
														</>
													) : r.wake_at ? (
														<>wakes {when(r.wake_at)}</>
													) : (
														<Dash />
													))}
											</TableCell>
										</TableRow>
									))}
								</TableBody>
							</Table>
						)}
					</div>
				</TabsPanel>
				<TabsPanel value="definitions" className="overflow-auto">
					{definitions.error && definitions.data === null ? (
						<ErrorState
							title="Studio could not load the workflow definitions"
							message={definitions.error}
							onRetry={definitions.reload}
						/>
					) : definitions.data === null ? (
						<LoadingRows />
					) : definitions.data.length === 0 ? (
						<EmptyState
							title="The app has no workflows"
							description="Add a file to the workflows/ directory that exports a workflow()."
						/>
					) : (
						<Table aria-label="Workflow definitions">
							<TableHeader>
								<tr>
									<TableHead className="w-64">Name</TableHead>
									<TableHead className="w-32 text-right">Step retries</TableHead>
									<TableHead className="w-32 text-right">Step timeout</TableHead>
									<TableHead>Description</TableHead>
								</tr>
							</TableHeader>
							<TableBody>
								{definitions.data.map((d) => (
									<TableRow key={d.name}>
										<TableCell className="font-mono text-xs">{d.name}</TableCell>
										<TableCell className="text-right">{d.max_retries ?? <Dash />}</TableCell>
										<TableCell className="text-right">
											{d.step_timeout_secs != null
												? formatDuration(d.step_timeout_secs * 1000)
												: "–"}
										</TableCell>
										<TableCell className="max-w-none text-muted-foreground">
											{d.description || <Dash />}
										</TableCell>
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
					open && !TERMINAL.has(open.status) ? (
						<Button variant="destructive" onClick={() => setCancelling(open)}>
							Cancel run
						</Button>
					) : undefined
				}
			>
				{open && (
					<>
						<Facts className="pt-2">
							<Fact label="State">
								<StatusBadge status={open.status} />
							</Fact>
							{open.key && (
								<Fact label="Key">
									<code className="text-xs">{open.key}</code>
								</Fact>
							)}
							<Fact label="Created">{when(open.created_at)}</Fact>
							<Fact label="Started">{when(open.started_at)}</Fact>
							<Fact label="Finished">{when(open.completed_at)}</Fact>
							{open.waiting_for && (
								<Fact label="Waiting for">
									event <code className="text-xs">{open.waiting_for}</code>
									{open.wait_deadline ? <> until {when(open.wait_deadline)}</> : null}
								</Fact>
							)}
							{open.wake_at ? <Fact label="Wakes">{when(open.wake_at)}</Fact> : null}
							{open.cancel_reason && <Fact label="Cancel reason">{open.cancel_reason}</Fact>}
						</Facts>
						{open.error && (
							<FactBlock label="Error">
								<CodeBlock className="max-h-40 whitespace-pre-wrap" value={open.error} />
							</FactBlock>
						)}
						<FactBlock label={`Steps (${open.steps.length})`}>
							{open.steps.length === 0 ? (
								<p className="text-xs text-muted-foreground">No step has run.</p>
							) : (
								<ol className="divide-y rounded-md border">
									{open.steps.map((s, i) => (
										<li key={`${s.step_id}-${i}`} className="px-2.5 py-2">
											<div className="flex items-center gap-2">
												<span className="w-5 text-right text-xs text-muted-foreground tabular-nums">
													{i + 1}
												</span>
												<code className="min-w-0 flex-1 truncate text-xs">{s.name}</code>
												{(s.retry_count ?? 0) > 0 && (
													<span className="text-[11px] text-muted-foreground">
														retry {s.retry_count}
													</span>
												)}
												<span className="text-xs text-muted-foreground tabular-nums">
													{s.duration_ms != null ? formatDuration(s.duration_ms) : ""}
												</span>
												<StatusBadge status={s.status} />
											</div>
											{s.error ? (
												<p className="mt-1 pl-7 text-xs break-words text-status-red-fg">
													{s.error}
												</p>
											) : s.output !== null && s.output !== undefined ? (
												<pre className="mt-1 max-h-24 overflow-auto pl-7 text-[11px] text-muted-foreground">
													{JSON.stringify(s.output)}
												</pre>
											) : null}
										</li>
									))}
								</ol>
							)}
						</FactBlock>
						{(open.pending_events?.length ?? 0) > 0 && (
							<FactBlock label="Events that wait for the run">
								<CodeBlock
									className="max-h-48"
									value={JSON.stringify(open.pending_events, null, 2)}
								/>
							</FactBlock>
						)}
						<FactBlock label="Input">
							<CodeBlock className="max-h-48" value={JSON.stringify(open.input ?? null, null, 2)} />
						</FactBlock>
						{open.output !== null && open.output !== undefined && (
							<FactBlock label="Output">
								<CodeBlock className="max-h-48" value={JSON.stringify(open.output, null, 2)} />
							</FactBlock>
						)}
					</>
				)}
			</DetailSheet>

			<ConfirmDialog
				open={cancelling !== null}
				title="Cancel the run?"
				description={`The run ${cancelling?.id ?? ""} of ${cancelling?.name ?? ""} stops. Steps that are complete stay complete.`}
				confirmLabel="Cancel run"
				cancelLabel="Keep run"
				destructive
				onCancel={() => setCancelling(null)}
				onConfirm={async () => {
					if (cancelling) await cancel(cancelling);
					setCancelling(null);
				}}
			/>
		</Page>
	);
}
