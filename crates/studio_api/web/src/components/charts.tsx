import { plural } from "@/lib/format";
import { cn } from "@/lib/utils";

/** A color swatch and a label, for the legend of a chart. */
export function Legend({ className, label }: { className: string; label: string }) {
	return (
		<span className="inline-flex items-center gap-1.5">
			<span className={cn("size-2 rounded-[2px]", className)} />
			{label}
		</span>
	);
}

function ChartFrame({
	label,
	peak,
	empty,
	children,
}: {
	label: string;
	peak: string;
	empty: boolean;
	children: React.ReactNode;
}) {
	return (
		<div className="px-3 pb-3">
			<div
				role="img"
				aria-label={label}
				className="relative flex h-24 items-end gap-px border-b"
			>
				<span className="absolute top-0 left-0 text-[11px] text-subtle-foreground tabular-nums">
					{peak}
				</span>
				{children}
				{empty && (
					<span className="m-auto text-xs text-muted-foreground">No data</span>
				)}
			</div>
			<div className="mt-1 flex justify-between text-[11px] text-subtle-foreground">
				<span>60 min ago</span>
				<span>Now</span>
			</div>
		</div>
	);
}

/**
 * Requests for each minute, oldest on the left. The error share of a bar is
 * red and sits on top of the OK share. Index 0 of each series is the current
 * minute.
 */
export function RequestChart({
	series,
	errors,
}: {
	series: number[];
	errors: number[];
}) {
	const points = series
		.map((total, i) => ({
			total,
			errors: Math.min(errors[i] ?? 0, total),
			minutesAgo: i,
		}))
		.reverse();
	const max = Math.max(1, ...series);
	return (
		<ChartFrame
			label={`Requests for each of the last ${points.length} minutes. The highest minute has ${max} requests.`}
			peak={`${max.toLocaleString()} / min`}
			empty={points.length === 0}
		>
			{points.map((p) => (
				<div
					key={p.minutesAgo}
					title={`${ago(p.minutesAgo)}: ${plural(p.total, "request")}, ${plural(p.errors, "error")}`}
					className="flex h-full min-w-0 flex-1 flex-col justify-end hover:bg-accent"
				>
					{p.errors > 0 && (
						<div
							className="mb-px rounded-t-[1px] bg-status-red-fg"
							style={{ height: `${Math.max(2, (p.errors / max) * 100)}%` }}
						/>
					)}
					{p.total - p.errors > 0 && (
						<div
							className="rounded-t-[1px] bg-primary/85"
							style={{
								height: `${Math.max(2, ((p.total - p.errors) / max) * 100)}%`,
							}}
						/>
					)}
				</div>
			))}
		</ChartFrame>
	);
}

/**
 * Latency for each minute, oldest on the left: a bar to the p95 and a darker
 * bar to the p50. A minute with no timed request has no bar. Index 0 of each
 * series is the current minute.
 */
export function LatencyChart({
	p50,
	p95,
}: {
	p50: Array<number | null>;
	p95: Array<number | null>;
}) {
	const points = p95
		.map((high, i) => ({ high, mid: p50[i] ?? null, minutesAgo: i }))
		.reverse();
	const max = Math.max(1, ...p95.map((v) => v ?? 0));
	return (
		<ChartFrame
			label={`p50 and p95 latency for each of the last ${points.length} minutes. The highest p95 is ${max} ms.`}
			peak={`${max.toLocaleString()} ms`}
			empty={points.every((p) => p.high === null)}
		>
			{points.map((p) => (
				<div
					key={p.minutesAgo}
					title={
						p.high === null
							? `${ago(p.minutesAgo)}: no timed requests`
							: `${ago(p.minutesAgo)}: p50 ${p.mid ?? "–"} ms, p95 ${p.high} ms`
					}
					className="relative flex h-full min-w-0 flex-1 flex-col justify-end hover:bg-accent"
				>
					{p.high !== null && (
						<div
							className="rounded-t-[1px] bg-status-amber-fg/45"
							style={{ height: `${Math.max(2, (p.high / max) * 100)}%` }}
						/>
					)}
					{p.mid !== null && (
						<div
							className="absolute inset-x-0 bottom-0 rounded-t-[1px] bg-primary/85"
							style={{ height: `${Math.max(2, (p.mid / max) * 100)}%` }}
						/>
					)}
				</div>
			))}
		</ChartFrame>
	);
}

function ago(minutes: number): string {
	return minutes === 0 ? "This minute" : `${minutes} min ago`;
}
