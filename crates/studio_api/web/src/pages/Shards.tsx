import { useCallback, useEffect, useState } from "react";
import { ArrowLeft, Loader2, Square, UserX } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { useAuth } from "@/auth/AuthContext";
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

function errorText(e: unknown) {
	if (e instanceof ApiError) return `${e.code}: ${e.message}`;
	return e instanceof Error ? e.message : String(e);
}

/** The shard shown in detail lives in `?shard=`, so a detail view links. */
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
			<LockedPage
				title="Shards require admin"
				description="Shard numbers, stopping shards, and kicking subscribers are admin-only."
			/>
		);
	}
	return selected ? (
		<ShardDetailView id={selected} onBack={() => select(null)} />
	) : (
		<ShardList onOpen={select} />
	);
}

function StatusBadge({ shard }: { shard: Shard }) {
	if (shard.error) return <Badge variant="destructive">failed</Badge>;
	return shard.running ? (
		<Badge variant="secondary">running</Badge>
	) : (
		<Badge variant="outline">stopped</Badge>
	);
}

function ShardList({ onOpen }: { onOpen: (id: string) => void }) {
	const [shards, setShards] = useState<Shard[] | null>(null);
	const [err, setErr] = useState<string | null>(null);

	useEffect(() => {
		let live = true;
		const load = async () => {
			try {
				const list = await api<Shard[]>("/api/shards");
				if (live) {
					setShards(list);
					setErr(null);
				}
			} catch (e) {
				if (live) setErr(errorText(e));
			}
		};
		void load();
		const timer = window.setInterval(load, POLL_MS);
		return () => {
			live = false;
			window.clearInterval(timer);
		};
	}, []);

	return (
		<div className="space-y-4">
			{err && (
				<div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
					{err}
				</div>
			)}
			<Card>
				<CardHeader>
					<CardTitle>Shards</CardTitle>
				</CardHeader>
				<CardContent>
					{shards === null ? (
						<Loader2 className="size-4 animate-spin text-muted-foreground" />
					) : shards.length === 0 ? (
						<p className="text-sm text-muted-foreground">
							No shards are running. Functions start them with{" "}
							<code>ctx.shards.create</code>.
						</p>
					) : (
						<Table>
							<TableHeader>
								<TableRow>
									<TableHead>Shard</TableHead>
									<TableHead>Kind</TableHead>
									<TableHead>Status</TableHead>
									<TableHead className="text-right">Subscribers</TableHead>
									<TableHead className="text-right">Tick p50 / p99 (ms)</TableHead>
									<TableHead className="text-right">Overruns</TableHead>
									<TableHead className="text-right">Bytes per tick p99</TableHead>
									<TableHead className="text-right">Dropped frames</TableHead>
									<TableHead className="text-right">Dropped inputs</TableHead>
								</TableRow>
							</TableHeader>
							<TableBody>
								{shards.map((s) => (
									<TableRow
										key={s.id}
										className="cursor-pointer"
										onClick={() => onOpen(s.id)}
									>
										<TableCell className="font-mono text-xs">{s.id}</TableCell>
										<TableCell>{s.kind ?? "—"}</TableCell>
										<TableCell>
											<StatusBadge shard={s} />
										</TableCell>
										<TableCell className="text-right font-mono">{s.subscribers}</TableCell>
										<TableCell className="text-right font-mono">
											{ms(s.stats.tick_ms.p50)} / {ms(s.stats.tick_ms.p99)}
										</TableCell>
										<TableCell className="text-right font-mono">{s.stats.overruns}</TableCell>
										<TableCell className="text-right font-mono">
											{bytes(s.stats.bytes_per_tick.p99)}
										</TableCell>
										<TableCell className="text-right font-mono">
											{s.stats.dropped_frames_total}
										</TableCell>
										<TableCell className="text-right font-mono">
											{s.stats.dropped_inputs_total.queue_full +
												s.stats.dropped_inputs_total.rate_limited}
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					)}
				</CardContent>
			</Card>
		</div>
	);
}

function Stat({ label, value, note }: { label: string; value: React.ReactNode; note?: string }) {
	return (
		<Card>
			<CardContent className="p-4">
				<p className="text-xs text-muted-foreground">{label}</p>
				<p className="mt-1 font-mono text-xl font-semibold">{value}</p>
				{note && <p className="mt-1 text-xs text-muted-foreground">{note}</p>}
			</CardContent>
		</Card>
	);
}

function ShardDetailView({ id, onBack }: { id: string; onBack: () => void }) {
	const [shard, setShard] = useState<ShardDetail | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const [confirmStop, setConfirmStop] = useState(false);
	const [busy, setBusy] = useState(false);
	/** The shard no longer exists (stopped, or removed when idle). */
	const [gone, setGone] = useState(false);
	const path = `/api/shards/${encodeURIComponent(id)}`;

	const [reloads, setReloads] = useState(0);
	const reload = useCallback(() => setReloads((n) => n + 1), []);

	useEffect(() => {
		let live = true;
		let timer: number | undefined;
		const load = async () => {
			try {
				const detail = await api<ShardDetail>(path);
				if (!live) return;
				setShard(detail);
				setErr(null);
			} catch (e) {
				if (!live) return;
				if (e instanceof ApiError && e.code === "SHARD_NOT_FOUND") {
					setGone(true);
					setShard(null);
					window.clearInterval(timer);
					return;
				}
				setErr(errorText(e));
			}
		};
		void load();
		timer = window.setInterval(load, POLL_MS);
		return () => {
			live = false;
			window.clearInterval(timer);
		};
	}, [path, reloads]);

	const stop = async () => {
		setBusy(true);
		try {
			await api(`${path}/stop`, { method: "POST", body: "{}" });
			toast.success(`Stopped ${id}`);
			setConfirmStop(false);
			onBack();
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			setBusy(false);
		}
	};

	const kick = async (subscriber: string) => {
		try {
			await api(`${path}/kick`, {
				method: "POST",
				body: JSON.stringify({ subscriber }),
			});
			toast.success(`Kicked ${subscriber}`);
			reload();
		} catch (e) {
			toast.error(errorText(e));
		}
	};

	const s = shard?.stats;
	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between gap-2">
				<Button size="sm" variant="ghost" onClick={onBack}>
					<ArrowLeft className="size-3.5" />
					All shards
				</Button>
				<Button
					size="sm"
					variant="destructive"
					onClick={() => setConfirmStop(true)}
					disabled={!shard || gone}
				>
					<Square className="size-3.5" />
					Stop shard
				</Button>
			</div>

			{err && (
				<div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
					{err}
				</div>
			)}

			{gone && (
				<p className="text-sm text-muted-foreground">
					Shard <code>{id}</code> is not running any more.
				</p>
			)}

			{shard && s && (
				<>
					<div className="flex items-center gap-3">
						<h2 className="font-mono text-lg font-semibold">{shard.id}</h2>
						{shard.kind && <Badge variant="outline">{shard.kind}</Badge>}
						<StatusBadge shard={shard} />
					</div>
					{shard.error && (
						<div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 font-mono text-xs text-destructive">
							{shard.error}
						</div>
					)}

					<div className="grid gap-4 md:grid-cols-4">
						<Stat label="Tick" value={shard.tick.toLocaleString()} />
						<Stat label="Subscribers" value={shard.subscribers} />
						<Stat label="Input queue" value={shard.input_queue} />
						<Stat
							label="Overruns"
							value={s.overruns}
							note="Ticks skipped because the loop fell behind."
						/>
					</div>

					<Card>
						<CardHeader>
							<CardTitle>Tick time (ms, last minute of ticks)</CardTitle>
						</CardHeader>
						<CardContent>
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>Phase</TableHead>
										<TableHead className="text-right">p50</TableHead>
										<TableHead className="text-right">p99</TableHead>
										<TableHead className="text-right">Max</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{(
										[
											["Whole tick", s.tick_ms],
											["Inputs (apply_input)", s.inputs_ms],
											["Simulation (tick)", s.sim_ms],
											["Interest and snapshots", s.interest_ms],
											["Encoding and queueing", s.encode_ms],
										] as const
									).map(([label, q]) => (
										<TableRow key={label}>
											<TableCell>{label}</TableCell>
											<TableCell className="text-right font-mono">{ms(q.p50)}</TableCell>
											<TableCell className="text-right font-mono">{ms(q.p99)}</TableCell>
											<TableCell className="text-right font-mono">{ms(q.max)}</TableCell>
										</TableRow>
									))}
								</TableBody>
							</Table>
						</CardContent>
					</Card>

					<div className="grid gap-4 md:grid-cols-4">
						<Stat
							label="Bytes per tick"
							value={`${bytes(s.bytes_per_tick.p50)} / ${bytes(s.bytes_per_tick.p99)}`}
							note="p50 / p99, all subscribers"
						/>
						<Stat
							label="Bytes per subscriber"
							value={`${bytes(s.bytes_per_subscriber.p50)} / ${bytes(s.bytes_per_subscriber.p99)}`}
							note="p50 / p99, per tick"
						/>
						<Stat
							label="Dropped frames"
							value={s.dropped_frames_total}
							note="From full subscriber queues."
						/>
						<Stat
							label="Dropped inputs"
							value={s.dropped_inputs_total.queue_full + s.dropped_inputs_total.rate_limited}
							note={`${s.dropped_inputs_total.queue_full} queue full, ${s.dropped_inputs_total.rate_limited} rate limited`}
						/>
					</div>

					<Card>
						<CardHeader>
							<CardTitle>Subscribers</CardTitle>
						</CardHeader>
						<CardContent>
							{shard.subscriber_ids.length === 0 ? (
								<p className="text-sm text-muted-foreground">No subscribers.</p>
							) : (
								<Table>
									<TableHeader>
										<TableRow>
											<TableHead>Subscriber</TableHead>
											<TableHead className="text-right">Connections</TableHead>
											<TableHead />
										</TableRow>
									</TableHeader>
									<TableBody>
										{shard.subscriber_ids.map((sub) => (
											<TableRow key={sub.id}>
												<TableCell className="font-mono text-xs">{sub.id}</TableCell>
												<TableCell className="text-right font-mono">{sub.connections}</TableCell>
												<TableCell className="text-right">
													<Button size="sm" variant="outline" onClick={() => kick(sub.id)}>
														<UserX className="size-3.5" />
														Kick
													</Button>
												</TableCell>
											</TableRow>
										))}
									</TableBody>
								</Table>
							)}
						</CardContent>
					</Card>
				</>
			)}

			<Dialog open={confirmStop} onOpenChange={setConfirmStop}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Stop {id}?</DialogTitle>
						<DialogDescription>
							The shard stops ticking and every subscriber's connection closes. Its
							state is gone unless the game saves it.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="ghost" onClick={() => setConfirmStop(false)}>
							Cancel
						</Button>
						<Button variant="destructive" onClick={stop} disabled={busy}>
							{busy && <Loader2 className="size-3.5 animate-spin" />}
							Stop shard
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}
