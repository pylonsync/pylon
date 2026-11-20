import { useEffect, useMemo, useRef, useState } from "react";
import { Pause, Play, Search } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { CodeBlock } from "@/components/code-block";
import { DetailSheet } from "@/components/detail-sheet";
import { Fact, FactBlock, Facts } from "@/components/facts";
import { Page, Toolbar } from "@/components/page";
import { Dash } from "@/components/renderers";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
import { useQuery } from "@/hooks/use-query";
import {
	errorText,
	formatBytes,
	formatDateTime,
	formatDuration,
	formatTime,
	parseDate,
	plural,
	shortId,
} from "@/lib/format";
import { api } from "@/lib/pylon";
import { statusClass } from "@/lib/status";
import { LockedPage } from "./Locked";

type RequestRow = {
	timestamp: string;
	method: string;
	path: string;
	status: number;
	cpu_ms: number;
	bytes_in?: number;
	bytes_out?: number;
	user?: string;
	/** The request is one of Studio's own. */
	studio?: boolean;
	error?: string;
};

type TraceOp = {
	op: string;
	entity?: string;
	id?: string;
	duration_ms?: number;
	row_count?: number;
	ok?: boolean;
	error?: string;
};

type Trace = {
	call_id?: string;
	fn_name?: string;
	fn_type?: string;
	user_id?: string | null;
	started_at?: number;
	duration_ms?: number;
	outcome?: { status?: string; value?: unknown; error?: unknown };
	ops?: TraceOp[];
};

type AuditEvent = {
	id: string;
	createdAt?: number | string;
	action?: string;
	actor?: string | null;
	subject?: string | null;
	tenant?: string | null;
	entity?: string | null;
	entityId?: string | null;
	ip?: string | null;
	success?: boolean;
	reason?: string | null;
	meta?: unknown;
};

type Session = {
	id?: string;
	user_id?: string | null;
	device?: string | null;
	tenant_id?: string | null;
	token_preview?: string;
	created_at?: number | string;
	expires_at?: number | string;
};

/** The request log keeps this many rows. It matches the ring in the server. */
const MAX_ROWS = 1000;
const TAIL_MS = 2000;

function statusVariant(status: number): BadgeProps["variant"] {
	const cls = statusClass(status);
	if (cls === "server") return "red";
	if (cls === "client") return "amber";
	if (status >= 300) return "blue";
	return "green";
}

export function LogsPage() {
	const { me } = useAuth();
	const [tab, setTab] = useState("requests");
	if (!me?.is_admin) {
		return (
			<Page title="Logs">
				<LockedPage
					title="Logs are for admins"
					description="Sign in with an admin account to see the request log."
				/>
			</Page>
		);
	}
	return (
		<Page title="Logs">
			<Tabs value={tab} onValueChange={(v) => v && setTab(v)} className="flex-1">
				<TabsList aria-label="Logs">
					<TabsTab value="requests">Requests</TabsTab>
					<TabsTab value="functions">Function calls</TabsTab>
					<TabsTab value="audit">Audit events</TabsTab>
					<TabsTab value="sessions">Sessions</TabsTab>
				</TabsList>
				<TabsPanel value="requests" className="flex flex-col">
					<RequestLog />
				</TabsPanel>
				<TabsPanel value="functions" className="flex flex-col">
					<FunctionCalls />
				</TabsPanel>
				<TabsPanel value="audit" className="flex flex-col">
					<AuditLog />
				</TabsPanel>
				<TabsPanel value="sessions" className="flex flex-col">
					<Sessions />
				</TabsPanel>
			</Tabs>
		</Page>
	);
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/**
 * The request log, newest first. The server keeps the last requests: 1,000 by
 * default, or PYLON_REQUEST_LOG_SIZE. The page loads the newest 1,000, then
 * asks every 2 seconds for the rows after the newest one it has.
 */
function RequestLog() {
	const [rows, setRows] = useState<RequestRow[] | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [live, setLive] = useState(true);
	const [statusFilter, setStatusFilter] = useState("all");
	const [method, setMethod] = useState("all");
	const [source, setSource] = useState("app");
	const [text, setText] = useState("");
	const [open, setOpen] = useState<RequestRow | null>(null);
	const cursor = useRef<string | null>(null);

	useEffect(() => {
		if (!live) return;
		let alive = true;
		const poll = async () => {
			try {
				const since = cursor.current;
				const res = await api<{ rows?: RequestRow[]; cursor?: string | null }>(
					`/admin/logs/tail?limit=${MAX_ROWS}${since ? `&since=${encodeURIComponent(since)}` : ""}`,
				);
				if (!alive) return;
				const fresh = res?.rows ?? [];
				if (res?.cursor) cursor.current = res.cursor;
				setRows((prev) =>
					fresh.length === 0 && prev ? prev : [...fresh, ...(prev ?? [])].slice(0, MAX_ROWS),
				);
				setError(null);
			} catch (err) {
				if (alive) setError(errorText(err));
			}
		};
		// The next request starts after the previous one finishes, so two
		// requests never read from the same cursor.
		let timer = 0;
		const loop = async () => {
			await poll();
			if (alive) timer = window.setTimeout(loop, TAIL_MS);
		};
		void loop();
		return () => {
			alive = false;
			window.clearTimeout(timer);
		};
	}, [live]);

	const methods = useMemo(
		() => [...new Set((rows ?? []).map((r) => r.method))].sort(),
		[rows],
	);
	const shown = useMemo(() => {
		const q = text.trim().toLowerCase();
		return (rows ?? []).filter((r) => {
			if (source === "app" && r.studio) return false;
			if (method !== "all" && r.method !== method) return false;
			if (statusFilter === "server" || statusFilter === "client") {
				if (statusClass(r.status) !== statusFilter) return false;
			} else if (statusFilter !== "all" && Math.floor(r.status / 100) !== Number(statusFilter)) {
				return false;
			}
			return (
				!q ||
				r.path.toLowerCase().includes(q) ||
				(r.user ?? "").toLowerCase().includes(q) ||
				(r.error ?? "").toLowerCase().includes(q)
			);
		});
	}, [rows, source, method, statusFilter, text]);
	const narrowed = method !== "all" || statusFilter !== "all" || text.trim() !== "";

	return (
		<>
			<Toolbar>
				<div className="relative min-w-0 flex-1 sm:w-64 sm:flex-none">
					<Search className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
					<Input
						type="search"
						name="path"
						aria-label="Filter by path, user or error"
						placeholder="Path, user or error"
						value={text}
						onChange={(e) => setText(e.target.value)}
						className="pl-7"
					/>
				</div>
				<Select
					aria-label="Source"
					value={source}
					onValueChange={setSource}
					options={[
						{ value: "app", label: "App requests" },
						{ value: "all", label: "App and Studio requests" },
					]}
				/>
				<Select
					aria-label="Status"
					value={statusFilter}
					onValueChange={setStatusFilter}
					options={[
						{ value: "all", label: "All statuses" },
						{ value: "server", label: "Server errors (5xx)" },
						{ value: "client", label: "Client errors (4xx)" },
						{ value: "2", label: "2xx" },
						{ value: "3", label: "3xx" },
					]}
				/>
				<Select
					aria-label="Method"
					value={method}
					onValueChange={setMethod}
					options={[
						{ value: "all", label: "All methods" },
						...methods.map((m) => ({ value: m, label: m })),
					]}
				/>
				<span className="ml-auto text-xs text-muted-foreground tabular-nums">
					{rows
						? shown.length === rows.length
							? plural(rows.length, "request")
							: `${shown.length.toLocaleString()} of ${rows.length.toLocaleString()}`
						: ""}
				</span>
				<Button variant="outline" onClick={() => setLive((v) => !v)}>
					{live ? <Pause /> : <Play />}
					{live ? "Pause" : "Resume"}
				</Button>
			</Toolbar>
			<div className="min-h-0 flex-1 overflow-auto">
				{error && rows === null ? (
					<ErrorState
						title="Studio could not load the request log"
						message={error}
						onRetry={() => {
							setLive(false);
							window.setTimeout(() => setLive(true), 0);
						}}
					/>
				) : rows === null ? (
					<LoadingRows rows={12} />
				) : shown.length === 0 ? (
					<EmptyState
						title={narrowed ? "No requests match" : "No requests in the log"}
						description={
							narrowed
								? "Change the text or the filters."
								: `The request log has no app requests. Open the app or call the API to see a request here. The requests that Studio makes are hidden: select "App and Studio requests" to see them.`
						}
					/>
				) : (
					<Table aria-label="Requests">
						<TableHeader>
							<tr>
								<TableHead className="w-28">Time</TableHead>
								<TableHead className="w-16">Status</TableHead>
								<TableHead className="w-20">Method</TableHead>
								<TableHead>Path</TableHead>
								<TableHead className="w-24 text-right">Duration</TableHead>
								<TableHead className="w-24 text-right">Response</TableHead>
								<TableHead className="w-48">User</TableHead>
							</tr>
						</TableHeader>
						<TableBody>
							{shown.map((r, i) => {
								const at = parseDate(r.timestamp);
								return (
									<TableRow
										key={`${r.timestamp}-${i}`}
										tabIndex={0}
										data-current={open === r}
										className="cursor-default"
										onClick={() => setOpen(r)}
										onKeyDown={(e) => {
											if (e.key !== "Enter" && e.key !== " ") return;
											e.preventDefault();
											setOpen(r);
										}}
									>
										<TableCell className="text-muted-foreground" title={r.timestamp}>
											{at ? formatTime(at) : r.timestamp}
										</TableCell>
										<TableCell>
											<Badge variant={statusVariant(r.status)}>{r.status}</Badge>
										</TableCell>
										<TableCell className="font-mono text-xs">{r.method}</TableCell>
										<TableCell className="max-w-none font-mono text-xs" title={r.path}>
											{r.path}
											{r.error && (
												<span className="ml-2 font-sans text-status-red-fg">{r.error}</span>
											)}
										</TableCell>
										<TableCell className="text-right">{formatDuration(r.cpu_ms)}</TableCell>
										<TableCell className="text-right text-muted-foreground">
											{r.bytes_out ? formatBytes(r.bytes_out) : <Dash />}
										</TableCell>
										<TableCell className="max-w-48 font-mono text-xs" title={r.user}>
											{r.user ?? <Dash />}
										</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				)}
			</div>
			<DetailSheet
				open={open !== null}
				title={open ? `${open.method} ${open.path}` : ""}
				subtitle={open?.timestamp}
				onClose={() => setOpen(null)}
			>
				{open && (
					<Facts className="pt-2">
						<Fact label="Status">
							<Badge variant={statusVariant(open.status)}>{open.status}</Badge>
						</Fact>
						<Fact label="Method">
							<code className="text-xs">{open.method}</code>
						</Fact>
						<Fact label="Path">
							<code className="text-xs">{open.path}</code>
						</Fact>
						<Fact label="Time">
							{parseDate(open.timestamp)
								? formatDateTime(parseDate(open.timestamp) as Date)
								: open.timestamp}
						</Fact>
						<Fact label="Duration">{formatDuration(open.cpu_ms)}</Fact>
						<Fact label="Request body">{formatBytes(open.bytes_in ?? 0)}</Fact>
						<Fact label="Response body">{formatBytes(open.bytes_out ?? 0)}</Fact>
						<Fact label="User">
							{open.user ? <code className="text-xs">{open.user}</code> : "Not signed in"}
						</Fact>
						{open.error && <Fact label="Error">{open.error}</Fact>}
						<p className="px-3 pt-3 text-xs text-muted-foreground">
							The log keeps the path with no query string, and it keeps no
							headers and no bodies.
						</p>
					</Facts>
				)}
			</DetailSheet>
		</>
	);
}

// ---------------------------------------------------------------------------
// Function calls
// ---------------------------------------------------------------------------

function traceFailed(t: Trace): boolean {
	return t.outcome?.status !== undefined && t.outcome.status !== "ok";
}

function FunctionCalls() {
	const { data, error, reload } = useQuery(
		"fn-traces",
		async () => {
			const res = await api<Trace[] | { data?: Trace[] }>("/admin/fn/traces");
			return Array.isArray(res) ? res : (res?.data ?? []);
		},
		{ pollMs: 5000 },
	);
	const [open, setOpen] = useState<Trace | null>(null);
	const [onlyErrors, setOnlyErrors] = useState("all");
	const shown = (data ?? []).filter((t) => onlyErrors === "all" || traceFailed(t));

	return (
		<>
			<Toolbar>
				<Select
					aria-label="Outcome"
					value={onlyErrors}
					onValueChange={setOnlyErrors}
					options={[
						{ value: "all", label: "All calls" },
						{ value: "errors", label: "Errors" },
					]}
				/>
				<span className="ml-auto text-xs text-muted-foreground tabular-nums">
					{data ? plural(shown.length, "call") : ""}
				</span>
			</Toolbar>
			<div className="min-h-0 flex-1 overflow-auto">
				{error && data === null ? (
					<ErrorState
						title="Studio could not load the function calls"
						message={error}
						onRetry={reload}
					/>
				) : data === null ? (
					<LoadingRows />
				) : shown.length === 0 ? (
					<EmptyState
						title="No function calls"
						description="The server keeps a trace of each recent call to a function in functions/: the outcome, the duration and the database operations. Call a function from the app or from the Functions page."
					/>
				) : (
					<Table aria-label="Function calls">
						<TableHeader>
							<tr>
								<TableHead className="w-28">Time</TableHead>
								<TableHead className="w-16">Outcome</TableHead>
								<TableHead>Function</TableHead>
								<TableHead className="w-24">Type</TableHead>
								<TableHead className="w-24 text-right">Duration</TableHead>
								<TableHead className="w-20 text-right">DB ops</TableHead>
								<TableHead className="w-48">User</TableHead>
							</tr>
						</TableHeader>
						<TableBody>
							{shown.map((t, i) => {
								const at = parseDate(t.started_at);
								return (
									<TableRow
										key={`${t.call_id}-${t.started_at}-${i}`}
										tabIndex={0}
										data-current={open === t}
										className="cursor-default"
										onClick={() => setOpen(t)}
										onKeyDown={(e) => {
											if (e.key !== "Enter" && e.key !== " ") return;
											e.preventDefault();
											setOpen(t);
										}}
									>
										<TableCell className="text-muted-foreground">
											{at ? formatTime(at) : <Dash />}
										</TableCell>
										<TableCell>
											<Badge variant={traceFailed(t) ? "red" : "green"}>
												{t.outcome?.status ?? "ok"}
											</Badge>
										</TableCell>
										<TableCell className="font-mono text-xs">{t.fn_name}</TableCell>
										<TableCell className="text-muted-foreground">{t.fn_type ?? <Dash />}</TableCell>
										<TableCell className="text-right">
											{t.duration_ms != null ? formatDuration(t.duration_ms) : <Dash />}
										</TableCell>
										<TableCell className="text-right">{t.ops?.length ?? 0}</TableCell>
										<TableCell className="max-w-48 font-mono text-xs" title={t.user_id ?? undefined}>
											{t.user_id ?? <Dash />}
										</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				)}
			</div>
			<DetailSheet
				open={open !== null}
				title={open?.fn_name ?? ""}
				subtitle={open?.call_id}
				onClose={() => setOpen(null)}
			>
				{open && (
					<>
						<Facts className="pt-2">
							<Fact label="Outcome">
								<Badge variant={traceFailed(open) ? "red" : "green"}>
									{open.outcome?.status ?? "ok"}
								</Badge>
							</Fact>
							<Fact label="Type">{open.fn_type ?? <Dash />}</Fact>
							<Fact label="Started">
								{parseDate(open.started_at)
									? formatDateTime(parseDate(open.started_at) as Date)
									: "–"}
							</Fact>
							<Fact label="Duration">
								{open.duration_ms != null ? formatDuration(open.duration_ms) : "–"}
							</Fact>
							<Fact label="User">
								{open.user_id ? <code className="text-xs">{open.user_id}</code> : "Not signed in"}
							</Fact>
						</Facts>
						<FactBlock label={traceFailed(open) ? "Error" : "Result"}>
							<CodeBlock
								className="max-h-60"
								value={
									JSON.stringify(
										traceFailed(open)
											? (open.outcome?.error ?? open.outcome)
											: open.outcome?.value,
										null,
										2,
									) ?? "undefined"
								}
							/>
						</FactBlock>
						<FactBlock label={`Database operations (${open.ops?.length ?? 0})`}>
							{open.ops && open.ops.length > 0 ? (
								<div className="overflow-auto rounded-md border">
									<Table aria-label="Database operations">
										<TableHeader className="static">
											<tr>
												<TableHead>Operation</TableHead>
												<TableHead>Entity</TableHead>
												<TableHead className="text-right">Rows</TableHead>
												<TableHead className="text-right">Duration</TableHead>
											</tr>
										</TableHeader>
										<TableBody>
											{open.ops.map((op, i) => (
												<TableRow key={i} className="last:[&>td]:border-b-0">
													<TableCell className="font-mono text-xs">
														{op.op}
														{op.ok === false && (
															<Badge variant="red" className="ml-2">
																failed
															</Badge>
														)}
													</TableCell>
													<TableCell>{op.entity ?? <Dash />}</TableCell>
													<TableCell className="text-right">
														{op.row_count ?? <Dash />}
													</TableCell>
													<TableCell className="text-right">
														{op.duration_ms != null ? formatDuration(op.duration_ms) : <Dash />}
													</TableCell>
												</TableRow>
											))}
										</TableBody>
									</Table>
								</div>
							) : (
								<p className="text-xs text-muted-foreground">
									The call made no database operations.
								</p>
							)}
						</FactBlock>
						<FactBlock label="Trace">
							<CodeBlock className="max-h-72" value={JSON.stringify(open, null, 2)} />
						</FactBlock>
					</>
				)}
			</DetailSheet>
		</>
	);
}

// ---------------------------------------------------------------------------
// Audit events
// ---------------------------------------------------------------------------

function AuditLog() {
	const { data, error, reload } = useQuery(
		"audit",
		async () => (await api<AuditEvent[]>("/api/admin/audit?limit=200")) ?? [],
		{ pollMs: 10_000 },
	);
	const [open, setOpen] = useState<AuditEvent | null>(null);
	return (
		<>
			<div className="min-h-0 flex-1 overflow-auto">
				{error && data === null ? (
					<ErrorState
						title="Studio could not load the audit events"
						message={error}
						onRetry={reload}
					/>
				) : data === null ? (
					<LoadingRows />
				) : data.length === 0 ? (
					<EmptyState
						title="No audit events"
						description="The audit log records sign-ins, password changes, and other account events, and the events that a function writes with ctx.audit. The newest 200 events show here."
					/>
				) : (
					<Table aria-label="Audit events">
						<TableHeader>
							<tr>
								<TableHead className="w-44">Time</TableHead>
								<TableHead className="w-20">Result</TableHead>
								<TableHead>Action</TableHead>
								<TableHead>Actor</TableHead>
								<TableHead>Subject</TableHead>
								<TableHead>Entity</TableHead>
								<TableHead>IP</TableHead>
							</tr>
						</TableHeader>
						<TableBody>
							{data.map((e) => {
								const at = parseDate(e.createdAt);
								return (
									<TableRow
										key={e.id}
										tabIndex={0}
										data-current={open?.id === e.id}
										className="cursor-default"
										onClick={() => setOpen(e)}
										onKeyDown={(ev) => {
											if (ev.key !== "Enter" && ev.key !== " ") return;
											ev.preventDefault();
											setOpen(e);
										}}
									>
										<TableCell className="text-muted-foreground">
											{at ? formatDateTime(at) : <Dash />}
										</TableCell>
										<TableCell>
											<Badge variant={e.success === false ? "red" : "green"}>
												{e.success === false ? "failed" : "ok"}
											</Badge>
										</TableCell>
										<TableCell className="font-mono text-xs">{e.action}</TableCell>
										<TableCell className="max-w-48 font-mono text-xs" title={e.actor ?? undefined}>
											{e.actor ?? <Dash />}
										</TableCell>
										<TableCell className="max-w-48 font-mono text-xs" title={e.subject ?? undefined}>
											{e.subject ?? <Dash />}
										</TableCell>
										<TableCell>
											{e.entity ?? <Dash />}
											{e.entityId && (
												<span className="ml-1 font-mono text-xs text-muted-foreground">
													{shortId(e.entityId)}
												</span>
											)}
										</TableCell>
										<TableCell className="font-mono text-xs">{e.ip ?? <Dash />}</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				)}
			</div>
			<DetailSheet
				open={open !== null}
				title={open?.action ?? ""}
				subtitle={open?.id}
				onClose={() => setOpen(null)}
			>
				{open && (
					<FactBlock label="Event">
						<CodeBlock className="mt-3" value={JSON.stringify(open, null, 2)} />
					</FactBlock>
				)}
			</DetailSheet>
		</>
	);
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/** The sessions the server holds. The server sends a preview of each token, never the token. */
function Sessions() {
	const query = useQuery(
		"sessions",
		async () => {
			const res = await api<Session[] | { data?: Session[] }>("/api/admin/auth/sessions");
			// The time of the load decides which sessions show as expired.
			return { rows: Array.isArray(res) ? res : (res?.data ?? []), loadedAt: Date.now() };
		},
		{ pollMs: 15_000 },
	);
	const data = query.data?.rows ?? null;
	const now = query.data?.loadedAt ?? 0;
	const { error, reload } = query;
	return (
		<div className="min-h-0 flex-1 overflow-auto">
			{error && data === null ? (
				<ErrorState
					title="Studio could not load the sessions"
					message={error}
					onRetry={reload}
				/>
			) : data === null ? (
				<LoadingRows />
			) : data.length === 0 ? (
				<EmptyState
					title="No sessions"
					description="A session starts when a user or a Studio operator signs in."
				/>
			) : (
				<Table aria-label="Sessions">
					<TableHeader>
						<tr>
							<TableHead className="w-24">State</TableHead>
							<TableHead>User</TableHead>
							<TableHead>Device</TableHead>
							<TableHead className="w-44">Created</TableHead>
							<TableHead className="w-44">Expires</TableHead>
							<TableHead className="w-36">Token</TableHead>
						</tr>
					</TableHeader>
					<TableBody>
						{data.map((session, i) => {
							const created = parseDate(session.created_at);
							const expires = parseDate(session.expires_at);
							const expired = expires !== null && expires.getTime() < now;
							return (
								<TableRow key={session.id ?? i}>
									<TableCell>
										<Badge variant={expired ? "gray" : "green"}>
											{expired ? "expired" : "active"}
										</Badge>
									</TableCell>
									<TableCell
										className="max-w-64 font-mono text-xs"
										title={session.user_id ?? undefined}
									>
										{session.user_id ?? <Dash />}
									</TableCell>
									<TableCell>{session.device ?? <Dash />}</TableCell>
									<TableCell className="text-muted-foreground">
										{created ? formatDateTime(created) : <Dash />}
									</TableCell>
									<TableCell className="text-muted-foreground">
										{expires ? formatDateTime(expires) : <Dash />}
									</TableCell>
									<TableCell className="font-mono text-xs text-muted-foreground">
										{session.token_preview ?? <Dash />}
									</TableCell>
								</TableRow>
							);
						})}
					</TableBody>
				</Table>
			)}
		</div>
	);
}
