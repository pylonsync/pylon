import { useEffect, useState } from "react";
import { Pause, Play } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { CodeBlock } from "@/components/code-block";
import { DetailSheet } from "@/components/detail-sheet";
import { Fact, FactBlock, Facts } from "@/components/facts";
import { Page, PageScroll } from "@/components/page";
import { RefCell } from "@/components/renderers";
import { EmptyState, ErrorBanner } from "@/components/states";
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
import { useMetrics } from "@/hooks/use-metrics";
import { formatTime, jsonPreview, parseDate } from "@/lib/format";
import { API_BASE, MANIFEST } from "@/lib/pylon";
import { LockedPage } from "./Locked";

type Change = {
	seq: number;
	kind: string;
	entity: string;
	row_id: string;
	timestamp?: string;
	/** The fields of the row. The server redacts serverOnly and encrypted values. */
	fields: Record<string, unknown> | null;
	/** When this page received the change. */
	seenAt: number;
};

type LogStatus = { head_seq: number; retained: number };

const MAX_EVENTS = 500;

const KIND_VARIANT: Record<string, BadgeProps["variant"]> = {
	insert: "green",
	update: "blue",
	delete: "red",
};

/**
 * The tail of the change log: each insert, update and delete, as the server
 * records it for sync clients. The page reads the operator change stream
 * (`GET /admin/changes/stream`, Server-Sent Events), which starts at the
 * change after the newest one when the page opens.
 */
export function SyncPage() {
	const { me } = useAuth();
	const isAdmin = !!me?.is_admin;
	const { metrics } = useMetrics();
	const [events, setEvents] = useState<Change[]>([]);
	const [log, setLog] = useState<LogStatus | null>(null);
	const [paused, setPaused] = useState(false);
	const [connected, setConnected] = useState(false);
	const [missed, setMissed] = useState(false);
	const [open, setOpen] = useState<Change | null>(null);

	useEffect(() => {
		if (!isAdmin || paused) return;
		const source = new EventSource(`${API_BASE}/admin/changes/stream`, {
			withCredentials: true,
		});
		const onStatus = (e: MessageEvent<string>) => {
			try {
				setLog(JSON.parse(e.data) as LogStatus);
			} catch {
				// A frame that is not JSON carries no status.
			}
		};
		source.onopen = () => setConnected(true);
		// The browser connects again by itself after an error.
		source.onerror = () => setConnected(false);
		source.addEventListener("hello", onStatus);
		source.addEventListener("status", onStatus);
		source.addEventListener("resync", (e) => {
			onStatus(e);
			setMissed(true);
		});
		source.addEventListener("change", (e) => {
			try {
				const change = JSON.parse(e.data) as Omit<Change, "seenAt">;
				setEvents((prev) =>
					[{ ...change, seenAt: Date.now() }, ...prev].slice(0, MAX_EVENTS),
				);
				setLog((prev) => ({
					head_seq: Math.max(prev?.head_seq ?? 0, change.seq),
					retained: prev?.retained ?? 0,
				}));
			} catch {
				// A change that does not parse is dropped.
			}
		});
		return () => {
			source.close();
			setConnected(false);
		};
	}, [isAdmin, paused]);

	if (!isAdmin) {
		return (
			<Page title="Live sync">
				<LockedPage
					title="Live sync is for admins"
					description="Sign in with an admin account to see the change events."
				/>
			</Page>
		);
	}

	const n = (v: number | undefined) => (v === undefined ? "–" : v.toLocaleString());
	return (
		<Page
			title="Live sync"
			meta={
				<Badge variant={paused ? "gray" : connected ? "green" : "amber"} dot>
					{paused ? "Paused" : connected ? "Live" : "Connecting"}
				</Badge>
			}
			actions={
				<>
					<Button variant="outline" onClick={() => setPaused((p) => !p)}>
						{paused ? <Play /> : <Pause />}
						{paused ? "Resume" : "Pause"}
					</Button>
					<Button
						variant="outline"
						disabled={events.length === 0}
						onClick={() => {
							setEvents([]);
							setMissed(false);
						}}
					>
						Clear
					</Button>
				</>
			}
		>
			<PageScroll>
				{missed && (
					<ErrorBanner>
						The page fell behind the change log and missed some changes. The
						list continues from the newest change.
					</ErrorBanner>
				)}
				<StatStrip>
					<Stat size="sm" label="Last sequence number" value={n(log?.head_seq)} />
					<Stat size="sm" label="Changes in the log" value={n(log?.retained)} />
					<Stat size="sm" label="WebSocket clients" value={n(metrics?.realtime?.ws_connections)} />
					<Stat size="sm" label="SSE clients" value={n(metrics?.realtime?.sse_connections)} />
					<Stat size="sm" label="Events on this page" value={events.length.toLocaleString()} />
				</StatStrip>
				{events.length === 0 ? (
					<EmptyState
						title={paused ? "The stream is paused" : "No changes since you opened this page"}
						description={`Each insert, update and delete of a row shows here as the server records it. Change a row in the app or in a table to see one. The page keeps the last ${MAX_EVENTS} events.`}
					/>
				) : (
					<Table aria-label="Change events">
						<TableHeader className="static">
							<tr>
								<TableHead className="w-24 text-right">Seq</TableHead>
								<TableHead className="w-24">Kind</TableHead>
								<TableHead className="w-40">Entity</TableHead>
								<TableHead className="w-40">Row</TableHead>
								<TableHead>Fields</TableHead>
								<TableHead className="w-28 text-right">Time</TableHead>
							</tr>
						</TableHeader>
						<TableBody>
							{events.map((e) => (
								<TableRow
									key={e.seq}
									tabIndex={0}
									data-current={open?.seq === e.seq}
									className="cursor-default"
									onClick={() => setOpen(e)}
									onKeyDown={(ev) => {
										if (ev.target !== ev.currentTarget) return;
										if (ev.key !== "Enter" && ev.key !== " ") return;
										ev.preventDefault();
										setOpen(e);
									}}
								>
									<TableCell className="text-right font-mono text-xs text-muted-foreground">
										{e.seq}
									</TableCell>
									<TableCell>
										<Badge variant={KIND_VARIANT[e.kind] ?? "gray"}>{e.kind}</Badge>
									</TableCell>
									<TableCell>{e.entity}</TableCell>
									<TableCell>
										{e.kind !== "delete" &&
										MANIFEST.entities.some((m) => m.name === e.entity) ? (
											<RefCell entity={e.entity} id={e.row_id} />
										) : (
											<span className="font-mono text-xs" title={e.row_id}>
												{e.row_id}
											</span>
										)}
									</TableCell>
									<TableCell className="max-w-md font-mono text-xs text-muted-foreground">
										{e.fields ? jsonPreview(e.fields) : "–"}
									</TableCell>
									<TableCell className="text-right text-muted-foreground">
										{formatTime(parseDate(e.timestamp) ?? new Date(e.seenAt))}
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				)}
			</PageScroll>
			<DetailSheet
				open={open !== null}
				title={open ? `${open.kind} ${open.entity}` : ""}
				subtitle={open?.row_id}
				onClose={() => setOpen(null)}
			>
				{open && (
					<>
						<Facts className="pt-2">
							<Fact label="Sequence number">{open.seq}</Fact>
							<Fact label="Kind">
								<Badge variant={KIND_VARIANT[open.kind] ?? "gray"}>{open.kind}</Badge>
							</Fact>
							<Fact label="Entity">{open.entity}</Fact>
							<Fact label="Row">
								<code className="text-xs">{open.row_id}</code>
							</Fact>
							<Fact label="Time">
								{formatTime(parseDate(open.timestamp) ?? new Date(open.seenAt))}
							</Fact>
						</Facts>
						<FactBlock label="Fields">
							{open.fields ? (
								<CodeBlock value={JSON.stringify(open.fields, null, 2)} />
							) : (
								<p className="text-xs text-muted-foreground">
									The change carries no fields.
								</p>
							)}
						</FactBlock>
						<p className="px-3 pb-3 text-xs text-muted-foreground">
							The server replaces the value of a serverOnly or encrypted field
							with [redacted].
						</p>
					</>
				)}
			</DetailSheet>
		</Page>
	);
}
