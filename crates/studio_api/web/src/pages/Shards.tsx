import { useCallback, useEffect, useState } from "react";
import { Square, UserX } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { Page, PageScroll, Section } from "@/components/page";
import { Dash } from "@/components/renderers";
import {
	EmptyState,
	ErrorBanner,
	ErrorState,
	LoadingRows,
} from "@/components/states";
import { Stat, StatStrip } from "@/components/stat-strip";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { toast } from "@/lib/toast";
import { useQuery } from "@/hooks/use-query";
import { errorText, plural } from "@/lib/format";
import { ApiError, api } from "@/lib/pylon";
import { LockedPage } from "./Locked";

type Quantiles = { p50: number; p99: number; max: number };

type ShardStats = {
	ticks: number;
	overruns: number;
	subscribers: number;
	input_queue: number;
	tick_ms: Quantiles;
	inputs_ms: Quantiles;
	sim_ms: Quantiles;
	interest_ms: Quantiles;
	frames_ms: Quantiles;
	encode_ms: Quantiles;
	bytes_per_tick: Quantiles;
	bytes_per_subscriber: Quantiles;
	bytes_total: number;
	dropped_frames_total: number;
	dropped_inputs_total: { queue_full: number; rate_limited: number };
};

type Shard = {
	id: string;
	kind: string | null;
	running: boolean;
	error: string | null;
	tick: number;
	subscribers: number;
	input_queue: number;
	stats: ShardStats;
};

type ShardDetail = Shard & {
	subscriber_ids: Array<{ id: string; connections: number }>;
};

const POLL_MS = 2000;

function ms(v: number) {
	return v < 10 ? v.toFixed(2) : v.toFixed(1);
}

function bytes(v: number) {
	if (v < 1024) return `${Math.round(v)} B`;
	if (v < 1024 * 1024) return `${(v / 1024).toFixed(1)} KB`;
	return `${(v / 1024 / 1024).toFixed(1)} MB`;
}

/** The shard in the detail view is `?shard=` in the URL, so the view has a link. */
function useSelectedShard(): [string | null, (id: string | null) => void] {
	const read = () => new URLSearchParams(window.location.search).get("shard");
	const [selected, setSelected] = useState<string | null>(read);
	useEffect(() => {
		const onPop = () => setSelected(read());
		window.addEventListener("popstate", onPop);
		return () => window.removeEventListener("popstate", onPop);
	}, []);
	const select = useCallback((id: string | null) => {
		const url = new URL(window.location.href);
		if (id) url.searchParams.set("shard", id);
		else url.searchParams.delete("shard");
		window.history.pushState(null, "", url);
		setSelected(id);
	}, []);
	return [selected, select];
}

export function ShardsPage() {
	const { me } = useAuth();
	const [selected, select] = useSelectedShard();

	if (!me?.is_admin) {
		return (
			<Page title="Shards">
				<LockedPage
					title="Shards are for admins"
					description="Sign in with an admin account to see shard numbers, stop a shard, or kick a subscriber."
				/>
			</Page>
		);
	}
	return selected ? (
		<ShardDetailView id={selected} onBack={() => select(null)} />
	) : (
		<ShardList onOpen={select} />
	);
}

function StatusBadge({ shard }: { shard: Shard }) {
	if (shard.error) return <Badge variant="red">failed</Badge>;
	return shard.running ? (
		<Badge variant="green" dot>
			running
		</Badge>
	) : (
		<Badge variant="gray">stopped</Badge>
	);
}

function ShardList({ onOpen }: { onOpen: (id: string) => void }) {
	const {
		data: shards,
		error: err,
		reload,
	} = useQuery("shards", async () => (await api<Shard[]>("/api/shards")) ?? [], {
		pollMs: POLL_MS,
	});

	return (
		<Page
			title="Shards"
			meta={shards ? plural(shards.length, "shard") : undefined}
		>
			<PageScroll>
				{err && shards !== null && (
					<ErrorBanner>
						Studio could not refresh the shards: {err}. The table shows the last
						good response.
					</ErrorBanner>
				)}
				{shards === null ? (
					err ? (
						<ErrorState
							title="Studio could not load the shards"
							message={err}
							onRetry={reload}
						/>
					) : (
						<LoadingRows />
					)
				) : shards.length === 0 ? (
					<EmptyState
						title="No shards"
						description={
							<>
								A function starts a shard with <code>ctx.shards.create</code>.
							</>
						}
					/>
				) : (
					<Table aria-label="Shards">
						<TableHeader>
							<tr>
								<TableHead>Shard</TableHead>
								<TableHead>Kind</TableHead>
								<TableHead>Status</TableHead>
								<TableHead className="text-right">Subscribers</TableHead>
								<TableHead className="text-right">Tick p50 / p99 (ms)</TableHead>
								<TableHead className="text-right">Overruns</TableHead>
								<TableHead className="text-right">Bytes per tick p99</TableHead>
								<TableHead className="text-right">Dropped frames</TableHead>
								<TableHead className="text-right">Dropped inputs</TableHead>
							</tr>
						</TableHeader>
						<TableBody>
							{shards.map((s) => (
								<TableRow
									key={s.id}
									tabIndex={0}
									className="cursor-default"
									onClick={() => onOpen(s.id)}
									onKeyDown={(e) => {
										if (e.key !== "Enter" && e.key !== " ") return;
										e.preventDefault();
										onOpen(s.id);
									}}
								>
									<TableCell className="font-mono text-xs">{s.id}</TableCell>
									<TableCell>{s.kind ?? <Dash />}</TableCell>
									<TableCell>
										<StatusBadge shard={s} />
									</TableCell>
									<TableCell className="text-right">{s.subscribers}</TableCell>
									<TableCell className="text-right">
										{ms(s.stats.tick_ms.p50)} / {ms(s.stats.tick_ms.p99)}
									</TableCell>
									<TableCell className="text-right">{s.stats.overruns}</TableCell>
									<TableCell className="text-right">
										{bytes(s.stats.bytes_per_tick.p99)}
									</TableCell>
									<TableCell className="text-right">
										{s.stats.dropped_frames_total}
									</TableCell>
									<TableCell className="text-right">
										{s.stats.dropped_inputs_total.queue_full +
											s.stats.dropped_inputs_total.rate_limited}
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				)}
			</PageScroll>
		</Page>
	);
}

function ShardDetailView({ id, onBack }: { id: string; onBack: () => void }) {
	const [confirmStop, setConfirmStop] = useState(false);
	/** The shard does not exist: it stopped, or the server removed it when idle. */
	const [gone, setGone] = useState(false);
	const path = `/api/shards/${encodeURIComponent(id)}`;

	const {
		data: shard,
		error: err,
		reload,
	} = useQuery(
		`shard:${id}`,
		async () => {
			try {
				return await api<ShardDetail>(path);
			} catch (e) {
				if (e instanceof ApiError && e.code === "SHARD_NOT_FOUND") {
					setGone(true);
					return null;
				}
				throw e;
			}
		},
		{ pollMs: gone ? 0 : POLL_MS },
	);

	const stop = async () => {
		try {
			await api(`${path}/stop`, { method: "POST", body: "{}" });
			toast.success(`The shard ${id} is stopped`);
			setConfirmStop(false);
			onBack();
		} catch (e) {
			toast.error("Studio could not stop the shard", {
				description: errorText(e),
			});
		}
	};

	const kick = async (subscriber: string) => {
		try {
			await api(`${path}/kick`, {
				method: "POST",
				body: JSON.stringify({ subscriber }),
			});
			toast.success(`The subscriber ${subscriber} is disconnected`);
			reload();
		} catch (e) {
			toast.error("Studio could not kick the subscriber", {
				description: errorText(e),
			});
		}
	};

	const s = shard?.stats;
	return (
		<Page
			parent={
				<button
					type="button"
					className="cursor-default rounded hover:text-foreground"
					onClick={onBack}
				>
					Shards
				</button>
			}
			title={<span className="font-mono">{id}</span>}
			meta={
				shard && (
					<span className="inline-flex items-center gap-1.5">
						{shard.kind && <Badge variant="outline">{shard.kind}</Badge>}
						<StatusBadge shard={shard} />
					</span>
				)
			}
			actions={
				<Button
					variant="destructive"
					disabled={!shard || gone}
					onClick={() => setConfirmStop(true)}
				>
					<Square />
					Stop shard
				</Button>
			}
		>
			<PageScroll>
				{err && shard && (
					<ErrorBanner>
						Studio could not refresh the shard: {err}. The numbers show the last
						good response.
					</ErrorBanner>
				)}
				{gone ? (
					<EmptyState
						title="The shard does not run"
						description={`The shard ${id} stopped, or the server removed it.`}
						action={
							<Button variant="outline" onClick={onBack}>
								Go to all shards
							</Button>
						}
					/>
				) : !shard || !s ? (
					err ? (
						<ErrorState
							title="Studio could not load the shard"
							message={err}
							onRetry={reload}
						/>
					) : (
						<LoadingRows />
					)
				) : (
					<>
						{shard.error && (
							<ErrorBanner>
								<span className="font-mono">{shard.error}</span>
							</ErrorBanner>
						)}
						<StatStrip>
							<Stat label="Tick" value={shard.tick.toLocaleString()} />
							<Stat label="Subscribers" value={shard.subscribers} />
							<Stat label="Input queue" value={shard.input_queue} />
							<Stat
								label="Overruns"
								value={s.overruns}
								tone={s.overruns > 0 ? "warn" : "default"}
								note="Ticks the loop skipped"
							/>
						</StatStrip>
						<StatStrip>
							<Stat
								label="Bytes per tick"
								value={`${bytes(s.bytes_per_tick.p50)} / ${bytes(s.bytes_per_tick.p99)}`}
								note="p50 / p99, all subscribers"
							/>
							<Stat
								label="Bytes per subscriber"
								value={`${bytes(s.bytes_per_subscriber.p50)} / ${bytes(s.bytes_per_subscriber.p99)}`}
								note="p50 / p99, for each tick"
							/>
							<Stat
								label="Dropped frames"
								value={s.dropped_frames_total}
								tone={s.dropped_frames_total > 0 ? "warn" : "default"}
								note="Subscriber queue was full"
							/>
							<Stat
								label="Dropped inputs"
								value={
									s.dropped_inputs_total.queue_full +
									s.dropped_inputs_total.rate_limited
								}
								tone={
									s.dropped_inputs_total.queue_full +
										s.dropped_inputs_total.rate_limited >
									0
										? "warn"
										: "default"
								}
								note={`${s.dropped_inputs_total.queue_full} queue full, ${s.dropped_inputs_total.rate_limited} rate limited`}
							/>
						</StatStrip>

						<Section title="Tick time in ms, last minute of ticks">
							<Table aria-label="Tick time by phase" className="border-t">
								<TableHeader className="static">
									<tr>
										<TableHead>Phase</TableHead>
										<TableHead className="text-right">p50</TableHead>
										<TableHead className="text-right">p99</TableHead>
										<TableHead className="text-right">Max</TableHead>
									</tr>
								</TableHeader>
								<TableBody>
									{(
										[
											["Whole tick", s.tick_ms],
											["Inputs (apply_input)", s.inputs_ms],
											["Simulation (tick)", s.sim_ms],
											["Interest and snapshots", s.interest_ms],
											["Replication frames", s.frames_ms],
											["Encoding and queueing", s.encode_ms],
										] as const
									).map(([label, q]) => (
										<TableRow key={label}>
											<TableCell>{label}</TableCell>
											<TableCell className="text-right">{ms(q.p50)}</TableCell>
											<TableCell className="text-right">{ms(q.p99)}</TableCell>
											<TableCell className="text-right">{ms(q.max)}</TableCell>
										</TableRow>
									))}
								</TableBody>
							</Table>
						</Section>

						<Section title="Subscribers" aside={shard.subscriber_ids.length}>
							{shard.subscriber_ids.length === 0 ? (
								<p className="px-3 pb-3 text-xs text-muted-foreground">
									The shard has no subscribers.
								</p>
							) : (
								<Table aria-label="Subscribers" className="border-t">
									<TableHeader className="static">
										<tr>
											<TableHead>Subscriber</TableHead>
											<TableHead className="text-right">Connections</TableHead>
											<TableHead className="w-px">
												<span className="sr-only">Actions</span>
											</TableHead>
										</tr>
									</TableHeader>
									<TableBody>
										{shard.subscriber_ids.map((sub) => (
											<TableRow key={sub.id}>
												<TableCell className="font-mono text-xs">{sub.id}</TableCell>
												<TableCell className="text-right">{sub.connections}</TableCell>
												<TableCell className="text-right">
													<Button
														size="sm"
														variant="outline"
														onClick={() => kick(sub.id)}
													>
														<UserX />
														Kick
													</Button>
												</TableCell>
											</TableRow>
										))}
									</TableBody>
								</Table>
							)}
						</Section>
					</>
				)}
			</PageScroll>

			<ConfirmDialog
				open={confirmStop}
				title={`Stop the shard ${id}?`}
				description="The shard stops its ticks and closes the connection of every subscriber. The state of the shard is lost unless the game saves it."
				confirmLabel="Stop shard"
				destructive
				onCancel={() => setConfirmStop(false)}
				onConfirm={stop}
			/>
		</Page>
	);
}
