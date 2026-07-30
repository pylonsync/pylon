"use client";

// Every animated primitive visual on the marketing site. Extracted from the
// landing page so the product pages can show the same running demo for the
// primitive they document — previously each /product page explained its
// primitive in prose while the landing page showed it working.
//
// Motion is driven by BentoHoverContext: the landing page's cards publish their
// hover state so only the pointed-at card animates, while product pages provide
// `true` because there the visual is the content, not a teaser.

import { createContext, useContext, useEffect, useState } from "react";
import { Check, Search } from "lucide-react";
import { highlightLine } from "./code-panel";

// Cards run only while pointed at. BentoCard publishes its hover state here and
// every visual reads it, so at idle the grid is thirteen static posters rather
// than thirteen timers.
export const BentoHoverContext = createContext(false);

export function useMotionTick(intervalMs: number): { tick: number; still: boolean } {
	const hovered = useContext(BentoHoverContext);
	const [tick, setTick] = useState(0);
	const [reduced, setReduced] = useState(false);
	// Touch devices never fire hover, so gating on it there would leave the cards
	// permanently frozen and grey. Where hover doesn't exist, always run.
	const [alwaysOn, setAlwaysOn] = useState(false);

	useEffect(() => {
		setReduced(window.matchMedia("(prefers-reduced-motion: reduce)").matches);
		setAlwaysOn(!window.matchMedia("(hover: hover)").matches);
	}, []);

	const running = !reduced && (alwaysOn || hovered);

	useEffect(() => {
		if (!running) {
			// Rewind so the card always returns to the same poster frame.
			setTick(0);
			return;
		}
		const t = setInterval(() => setTick((x) => x + 1), intervalMs);
		return () => clearInterval(t);
	}, [intervalMs, running]);

	// `still` already meant "render the complete resting frame" for reduced
	// motion; not-hovered is the same requirement, so the visuals need no change.
	return { tick, still: !running };
}

// Schema: the whole definition is always on screen; the accent walks the field
// rows and the migration stamp follows it, so nothing depends on catching a
// particular frame.
const SCHEMA_HEAD = 'const Order = entity("Order", {';
const SCHEMA_FIELDS = [
	"  customer: field.string(),",
	"  total: field.float(),",
	"  paid: field.boolean().default(false),",
];

export function SchemaBento() {
	const { tick, still } = useMotionTick(1100);
	const active = still ? -1 : tick % (SCHEMA_FIELDS.length + 1);
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5 font-mono text-[11.5px] leading-[1.85]">
			<div>
				<div>{highlightLine(SCHEMA_HEAD)}</div>
				{SCHEMA_FIELDS.map((line, i) => (
					<div
						// eslint-disable-next-line react/no-array-index-key
						key={i}
						className="-mx-1.5 rounded-[3px] px-1.5 transition-colors duration-500"
						style={
							i === active
								? {
										backgroundColor:
											"color-mix(in oklab, var(--color-cobalt) 10%, transparent)",
									}
								: undefined
						}
					>
						{highlightLine(line)}
					</div>
				))}
				<div>{highlightLine("});")}</div>
			</div>
			<div className="mt-3 flex items-center gap-1.5 border-t border-[var(--color-rule)] pt-2.5 text-[10.5px] text-[var(--color-ink-4)]">
				<Check className="size-3 text-[var(--color-status-live)]" />
				migration applied · 3 columns
			</div>
		</div>
	);
}

// Live queries: the surviving piece of the old hero mockup, cut down to a card.
// Rows insert at the top and flash; the subscriber count drifts. The resting
// frame is a populated table, so a still card still reads as a live view.
const SEED_ORDERS: OrderRow[] = [
	{ id: "ord_9f2a", customer: "Sarah Chen", total: "$1,240" },
	{ id: "ord_7c41", customer: "Marcus Lee", total: "$880" },
	{ id: "ord_5b88", customer: "Priya Nair", total: "$2,100" },
	{ id: "ord_3d10", customer: "Elena Duarte", total: "$640" },
];
type OrderRow = { id: string; customer: string; total: string; fresh?: boolean };
const INCOMING = [
	{ customer: "James Rivera", amount: 1180 },
	{ customer: "Hannah Weiss", amount: 740 },
	{ customer: "David Kim", amount: 2260 },
	{ customer: "Aisha Rahman", amount: 1540 },
	{ customer: "Lucas Moreau", amount: 990 },
	{ customer: "Grace Nakamura", amount: 3120 },
];

export function LiveQueryBento() {
	const { tick, still } = useMotionTick(2200);
	const [rows, setRows] = useState<OrderRow[]>(SEED_ORDERS);
	const clients = still ? 47 : 44 + (tick % 7);

	useEffect(() => {
		if (tick === 0) return;
		const o = INCOMING[tick % INCOMING.length]!;
		setRows((prev) =>
			[
				{
					id: `ord_${(0xac10 + tick * 53).toString(16).slice(-4)}`,
					customer: o.customer,
					total: `$${o.amount.toLocaleString("en-US")}`,
					fresh: true,
				},
				...prev.map((r) => ({ ...r, fresh: false })),
			].slice(0, 4),
		);
	}, [tick]);

	return (
		<div className="flex h-full flex-col">
			<div className="flex items-center gap-2 border-y border-[var(--color-rule)] bg-[var(--color-paper-1)] px-5 py-1.5">
				<code className="font-mono text-[11px] text-[var(--color-ink-3)]">
					db.useQuery(
					<span style={{ color: "var(--color-status-live)" }}>&quot;Order&quot;</span>)
				</code>
				<span className="ml-auto inline-flex items-center gap-1.5 font-mono text-[9.5px] uppercase tracking-[0.16em] text-[var(--color-ink-4)]">
					<span
						className="block size-1.5 rounded-full motion-safe:animate-pulse"
						style={{ backgroundColor: "var(--color-status-live)" }}
					/>
					live
				</span>
			</div>
			<div className="flex-1 divide-y divide-[var(--color-rule-soft)]">
				{rows.map((row) => (
					<div
						key={row.id}
						className="flex items-center gap-3 px-5 py-[7px] transition-colors duration-700"
						style={
							row.fresh
								? {
										backgroundColor:
											"color-mix(in oklab, var(--color-cobalt) 9%, transparent)",
									}
								: undefined
						}
					>
						<span className="w-[58px] shrink-0 font-mono text-[10.5px] text-[var(--color-ink-4)]">
							{row.id}
						</span>
						<span className="flex-1 truncate text-[12px] text-[var(--color-ink-2)]">
							{row.customer}
						</span>
						<span className="shrink-0 font-mono text-[11px] tabular-nums text-[var(--color-ink-3)]">
							{row.total}
						</span>
					</div>
				))}
			</div>
			<div className="flex items-center gap-1.5 border-t border-[var(--color-rule)] px-5 py-2 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				<span
					className="block size-1.5 rounded-full"
					style={{ backgroundColor: "var(--color-status-live)" }}
				/>
				diff streamed to{" "}
				<span className="tabular-nums text-[var(--color-ink-2)]">{clients}</span>{" "}
				clients
			</div>
		</div>
	);
}

// Policies: all three rules and their verdicts are always visible. The moving
// part is which rule is currently being evaluated.
const POLICY_RULES: { op: string; expr: string; allow: boolean }[] = [
	{ op: "read", expr: "auth.userId != null", allow: true },
	{ op: "insert", expr: "auth.userId == data.ownerId", allow: true },
	{ op: "delete", expr: "—  no rule", allow: false },
];

export function PolicyBento() {
	const { tick, still } = useMotionTick(1400);
	const active = still ? -1 : tick % POLICY_RULES.length;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div className="flex flex-col gap-1.5">
				{POLICY_RULES.map((r, i) => (
					<div
						key={r.op}
						className="flex items-center gap-2 rounded-[var(--radius-sm)] border px-2.5 py-2 font-mono text-[10.5px] transition-colors duration-500"
						style={{
							borderColor:
								i === active
									? "color-mix(in oklab, var(--color-cobalt) 45%, transparent)"
									: "var(--color-rule)",
							backgroundColor:
								i === active
									? "color-mix(in oklab, var(--color-cobalt) 7%, transparent)"
									: "transparent",
						}}
					>
						<span className="w-[42px] shrink-0 text-[var(--color-ink-3)]">{r.op}</span>
						<span className="flex-1 truncate text-[var(--color-ink-4)]">{r.expr}</span>
						<span
							className="shrink-0 font-medium"
							style={{
								color: r.allow
									? "var(--color-status-live)"
									: "var(--color-status-fail)",
							}}
						>
							{r.allow ? "allow" : "deny"}
						</span>
					</div>
				))}
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				unmatched operations fall through to deny
			</div>
		</div>
	);
}

// Auth: every provider is listed at rest; the highlight cycles to suggest a
// sign-in landing on one of them.
const PROVIDERS = ["Magic link", "Google", "GitHub", "Apple", "OIDC"];

export function AuthBento() {
	const { tick, still } = useMotionTick(1200);
	const active = still ? 0 : tick % PROVIDERS.length;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div className="flex flex-wrap gap-1.5">
				{PROVIDERS.map((p, i) => (
					<span
						key={p}
						className="rounded-full border px-2.5 py-1 text-[11px] transition-colors duration-500"
						style={{
							borderColor:
								i === active
									? "color-mix(in oklab, var(--color-cobalt) 45%, transparent)"
									: "var(--color-rule)",
							color:
								i === active ? "var(--color-cobalt)" : "var(--color-ink-3)",
							backgroundColor:
								i === active ? "var(--color-cobalt-soft)" : "transparent",
						}}
					>
						{p}
					</span>
				))}
			</div>
			<div className="mt-3 flex items-center gap-1.5 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				<Check className="size-3 text-[var(--color-status-live)]" />
				session · auth.userId set
			</div>
		</div>
	);
}

// Uploads: bars advance on each tick and settle at done. At rest every file is
// complete, which is the honest end state rather than a frozen half-upload.
const UPLOAD_FILES = [
	{ name: "invoice.pdf", size: "248 KB", offset: 0 },
	{ name: "avatar.png", size: "64 KB", offset: 1 },
	{ name: "export.csv", size: "1.2 MB", offset: 2 },
];

export function UploadBento() {
	const { tick, still } = useMotionTick(700);
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div className="flex flex-col gap-2.5">
				{UPLOAD_FILES.map((f) => {
					const pct = still ? 100 : ((tick + f.offset * 2) % 7) * 16.7;
					const done = pct >= 99;
					return (
						<div key={f.name} className="flex flex-col gap-1">
							<div className="flex items-baseline gap-2 font-mono text-[10.5px]">
								<span className="flex-1 truncate text-[var(--color-ink-2)]">
									{f.name}
								</span>
								<span className="shrink-0 tabular-nums text-[var(--color-ink-4)]">
									{done ? f.size : `${Math.round(pct)}%`}
								</span>
							</div>
							<div className="h-[3px] overflow-hidden rounded-full bg-[var(--color-paper-2)]">
								<div
									className="h-full rounded-full transition-[width] duration-500 ease-[var(--ease-out-quart)]"
									style={{
										width: `${Math.min(100, pct)}%`,
										backgroundColor: done
											? "var(--color-status-live)"
											: "var(--color-cobalt)",
									}}
								/>
							</div>
						</div>
					);
				})}
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				presigned · S3-compatible
			</div>
		</div>
	);
}

// Presence: labelled cursors drift between fixed positions. CSS handles the
// travel, so at rest they simply sit at their first position.
const CURSORS = [
	{ name: "Sarah", color: "var(--color-accent-blue)", path: [[14, 22], [58, 40], [30, 62]] },
	{ name: "Marcus", color: "var(--color-accent-purple)", path: [[62, 58], [22, 30], [70, 26]] },
	{ name: "Priya", color: "var(--color-accent-pink)", path: [[40, 70], [72, 62], [46, 34]] },
];

export function PresenceBento() {
	const { tick, still } = useMotionTick(1800);
	const step = still ? 0 : tick % 3;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			{/* Fixed height, not flex-1: this is the only visual with a canvas rather
			    than text rows, and letting it grow pushed the caption through the
			    card's bottom edge. 96px matches the text block height of its row. */}
			<div className="relative h-[96px] overflow-hidden rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				{CURSORS.map((c) => {
					const [x, y] = c.path[step]!;
					return (
						<div
							key={c.name}
							className="absolute flex items-center gap-1 transition-all duration-[1400ms] ease-[var(--ease-out-quart)]"
							style={{ left: `${x}%`, top: `${y}%` }}
						>
							<span
								className="size-1.5 rounded-full"
								style={{ backgroundColor: c.color }}
							/>
							<span
								className="rounded-[3px] px-1 py-px text-[9.5px] leading-tight text-[var(--color-paper)]"
								style={{ backgroundColor: c.color }}
							>
								{c.name}
							</span>
						</div>
					);
				})}
			</div>
			<div className="mt-3 shrink-0 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				room · 3 online
			</div>
		</div>
	);
}

// Server functions: the three kinds sit side by side and the call cursor walks
// them, so the card states what the surface *is* even when frozen.
const FUNCTIONS = [
	{ name: "getOrders", kind: "query", ms: "4ms" },
	{ name: "createOrder", kind: "mutation", ms: "11ms" },
	{ name: "sendReceipt", kind: "action", ms: "62ms" },
];

export function FunctionsBento() {
	const { tick, still } = useMotionTick(1500);
	const active = still ? -1 : tick % FUNCTIONS.length;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div className="flex flex-col gap-1.5">
				{FUNCTIONS.map((f, i) => (
					<div
						key={f.name}
						className="flex items-baseline gap-2 rounded-[var(--radius-sm)] border px-2.5 py-1.5 font-mono text-[10.5px] transition-colors duration-500"
						style={{
							borderColor:
								i === active
									? "color-mix(in oklab, var(--color-cobalt) 45%, transparent)"
									: "var(--color-rule)",
							backgroundColor:
								i === active
									? "color-mix(in oklab, var(--color-cobalt) 7%, transparent)"
									: "transparent",
						}}
					>
						<span className="truncate text-[var(--color-ink-2)]">{f.name}</span>
						<span className="text-[var(--color-cobalt)]">{f.kind}</span>
						<span className="ml-auto shrink-0 tabular-nums text-[var(--color-ink-4)]">
							{f.ms}
						</span>
					</div>
				))}
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				v.* validated · typed end to end
			</div>
		</div>
	);
}

// Reactive queries: a derived total that re-runs when one of the rows it read
// changes. The dependency list is always visible; the figures move.
const REACTIVE_FRAMES = [
	{ rows: [["us-east", "$48,120"], ["eu-west", "$31,904"], ["ap-south", "$12,470"]], note: "Order inserted → re-ran" },
	{ rows: [["us-east", "$49,300"], ["eu-west", "$31,904"], ["ap-south", "$12,470"]], note: "cached · no reads changed" },
	{ rows: [["us-east", "$49,300"], ["eu-west", "$33,644"], ["ap-south", "$12,470"]], note: "Order updated → re-ran" },
];

export function ReactiveBento() {
	const { tick, still } = useMotionTick(2400);
	const f = REACTIVE_FRAMES[still ? 0 : tick % REACTIVE_FRAMES.length]!;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div>
				<div className="font-mono text-[10px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]">
					reads Order · Region
				</div>
				<div className="mt-2 flex flex-col gap-1.5">
					{f.rows.map(([region, total]) => (
						<div
							key={region}
							className="flex items-baseline gap-2 font-mono text-[10.5px]"
						>
							<span className="text-[var(--color-ink-3)]">{region}</span>
							<span className="h-px flex-1 bg-[var(--color-rule)]" />
							<span className="tabular-nums text-[var(--color-ink-2)]">{total}</span>
						</div>
					))}
				</div>
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				{f.note}
			</div>
		</div>
	);
}

// Scheduler: queued work draining. Statuses advance and wrap, and the resting
// frame shows a populated queue rather than an empty one.
const JOBS = [
	{ name: "sendReceipt", when: "runAfter 30s" },
	{ name: "retryPayment", when: "runAt 09:00" },
	{ name: "sweepCarts", when: "every 5m" },
];
const JOB_STATES = ["queued", "running", "done"] as const;

export function SchedulerBento() {
	const { tick, still } = useMotionTick(1300);
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div className="flex flex-col gap-1.5">
				{JOBS.map((j, i) => {
					const state = still
						? JOB_STATES[i === 2 ? 1 : 2]!
						: JOB_STATES[(tick + i) % JOB_STATES.length]!;
					const color =
						state === "done"
							? "var(--color-status-live)"
							: state === "running"
								? "var(--color-cobalt)"
								: "var(--color-ink-4)";
					return (
						<div
							key={j.name}
							className="flex items-baseline gap-2 font-mono text-[10.5px]"
						>
							<span
								className="size-1.5 shrink-0 translate-y-[-1px] rounded-full transition-colors duration-500"
								style={{ backgroundColor: color }}
							/>
							<span className="truncate text-[var(--color-ink-2)]">{j.name}</span>
							<span className="text-[var(--color-ink-4)]">{j.when}</span>
							<span
								className="ml-auto shrink-0 transition-colors duration-500"
								style={{ color }}
							>
								{state}
							</span>
						</div>
					);
				})}
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				retries run in-process
			</div>
		</div>
	);
}

// Studio: the three surfaces it actually gives you, with the tab cycling. Each
// tab's body is a complete little view, so any frozen frame is a real one.
const STUDIO_TABS = [
	{
		name: "Tables",
		body: [["Order", "1,284"], ["Customer", "412"], ["Invoice", "980"]],
	},
	{
		name: "Live queries",
		body: [["useQuery(Order)", "47"], ["useQuery(Cart)", "12"], ["room:lobby", "8"]],
	},
	{
		name: "Logs",
		body: [["POST /orders", "201"], ["policy.check", "ok"], ["scheduler.run", "done"]],
	},
];

export function StudioTabsBento() {
	const { tick, still } = useMotionTick(2000);
	const active = still ? 0 : tick % STUDIO_TABS.length;
	const tab = STUDIO_TABS[active]!;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div>
				<div className="flex gap-1">
					{STUDIO_TABS.map((t, i) => (
						<span
							key={t.name}
							className="rounded-[var(--radius-xs)] px-1.5 py-0.5 font-mono text-[9.5px] transition-colors duration-500"
							style={{
								color:
									i === active ? "var(--color-cobalt)" : "var(--color-ink-4)",
								backgroundColor:
									i === active ? "var(--color-cobalt-soft)" : "transparent",
							}}
						>
							{t.name}
						</span>
					))}
				</div>
				<div className="mt-2.5 flex flex-col gap-1.5">
					{tab.body.map(([k, v]) => (
						<div key={k} className="flex items-baseline gap-2 font-mono text-[10.5px]">
							<span className="truncate text-[var(--color-ink-3)]">{k}</span>
							<span className="h-px flex-1 bg-[var(--color-rule)]" />
							<span className="tabular-nums text-[var(--color-ink-2)]">{v}</span>
						</div>
					))}
				</div>
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				/studio · admin-gated in prod
			</div>
		</div>
	);
}

// Engine: the point is that only the connection string moves. The active
// target alternates while the three invariants underneath stay put — which is
// the whole claim, so it has to be legible in a single frame.
const ENGINES = [
	{ name: "SQLite", url: "file:./pylon.db" },
	{ name: "Postgres", url: "postgres://…/app" },
];
const ENGINE_INVARIANTS = [
	["schema", "12 tables"],
	["policies", "8 rules"],
	["app code", "unchanged"],
];

export function EngineBento() {
	const { tick, still } = useMotionTick(2600);
	const active = still ? 0 : tick % ENGINES.length;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div>
				<div className="font-mono text-[10px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]">
					DATABASE_URL
				</div>
				<div className="mt-2 flex gap-2.5">
					{ENGINES.map((e, i) => (
						<div
							key={e.name}
							className="flex-1 rounded-[var(--radius-sm)] border px-3 py-2 transition-colors duration-500"
							style={{
								borderColor:
									i === active
										? "color-mix(in oklab, var(--color-cobalt) 45%, transparent)"
										: "var(--color-rule)",
								backgroundColor:
									i === active
										? "color-mix(in oklab, var(--color-cobalt) 7%, transparent)"
										: "transparent",
							}}
						>
							<div className="flex items-center gap-1.5">
								<span
									className="size-1.5 shrink-0 rounded-full transition-colors duration-500"
									style={{
										backgroundColor:
											i === active
												? "var(--color-status-live)"
												: "var(--color-ink-4)",
									}}
								/>
								<span className="text-[12px] font-medium tracking-tight text-[var(--color-ink)]">
									{e.name}
								</span>
							</div>
							<div className="mt-0.5 truncate font-mono text-[10.5px] text-[var(--color-ink-4)]">
								{e.url}
							</div>
						</div>
					))}
				</div>
			</div>
			<div className="mt-3 grid grid-cols-3 gap-x-4 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px]">
				{ENGINE_INVARIANTS.map(([k, v]) => (
					<div key={k} className="flex items-baseline gap-1.5">
						<span className="text-[var(--color-ink-4)]">{k}</span>
						<span className="truncate text-[var(--color-ink-2)]">{v}</span>
					</div>
				))}
			</div>
		</div>
	);
}

// SSR: a request walking its four stages. Stages already passed stay marked, so
// a frozen card reads as a completed request rather than a stalled one.
const SSR_STAGES = [
	{ label: "server render", detail: "query + policy run server-side" },
	{ label: "html streamed", detail: "first paint, no client fetch" },
	{ label: "hydrate", detail: "same typed client takes over" },
	{ label: "subscribed", detail: "live diffs from here on" },
];

export function SsrBento() {
	const { tick, still } = useMotionTick(1000);
	const at = still ? SSR_STAGES.length - 1 : tick % (SSR_STAGES.length + 1);
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div>
				<div className="flex items-baseline gap-2 font-mono text-[10.5px]">
					<span className="text-[var(--color-cobalt)]">GET</span>
					<span className="text-[var(--color-ink-2)]">/orders</span>
					<span className="ml-auto text-[var(--color-ink-4)]">200 · 14ms</span>
				</div>
				<div className="mt-2.5 flex flex-col gap-1">
					{SSR_STAGES.map((st, i) => {
						const done = i <= at;
						return (
							<div
								key={st.label}
								className="flex items-baseline gap-2 font-mono text-[10.5px] transition-opacity duration-500"
								style={{ opacity: done ? 1 : 0.35 }}
							>
								<span
									className="size-1.5 shrink-0 translate-y-[-1px] rounded-full transition-colors duration-500"
									style={{
										backgroundColor: done
											? "var(--color-status-live)"
											: "var(--color-ink-4)",
									}}
								/>
								<span className="w-[86px] shrink-0 text-[var(--color-ink-2)]">
									{st.label}
								</span>
								<span className="truncate text-[var(--color-ink-4)]">{st.detail}</span>
							</div>
						);
					})}
				</div>
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				one schema · server and client
			</div>
		</div>
	);
}

// Search: the query cycles and the facet counts move with it. Each frame is a
// coherent result set, so pausing on any one of them still makes sense.
const SEARCH_FRAMES = [
	{ q: "linen shirt", hits: "312", facets: [["Apparel", "184"], ["In stock", "96"], ["Sale", "32"]] },
	{ q: "wool coat", hits: "148", facets: [["Apparel", "121"], ["In stock", "44"], ["Sale", "18"]] },
	{ q: "canvas tote", hits: "526", facets: [["Bags", "298"], ["In stock", "173"], ["Sale", "55"]] },
];

export function SearchBento() {
	const { tick, still } = useMotionTick(2000);
	const f = SEARCH_FRAMES[still ? 0 : tick % SEARCH_FRAMES.length]!;
	return (
		<div className="flex h-full flex-col justify-between px-5 pb-5">
			<div>
				<div className="flex items-center gap-2 rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-2.5 py-1.5">
					<Search className="size-3 text-[var(--color-ink-4)]" />
					<span className="font-mono text-[11px] text-[var(--color-ink-2)]">{f.q}</span>
					<span className="ml-auto font-mono text-[10px] tabular-nums text-[var(--color-ink-4)]">
						{f.hits} hits
					</span>
				</div>
				<div className="mt-2.5 flex flex-col gap-1.5">
					{f.facets.map(([label, n]) => (
						<div
							key={label}
							className="flex items-baseline gap-2 font-mono text-[10.5px]"
						>
							<span className="text-[var(--color-ink-3)]">{label}</span>
							<span className="h-px flex-1 bg-[var(--color-rule)]" />
							<span className="tabular-nums text-[var(--color-ink-2)]">{n}</span>
						</div>
					))}
				</div>
			</div>
			<div className="mt-3 border-t border-[var(--color-rule)] pt-2.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
				indexed in the same transaction
			</div>
		</div>
	);
}

// `tone="sunken"` bands a section onto paper-1. Alternating bands give the page
// a rhythm — without them every section is the same white slab and the eye has
// no way to tell where one idea ends and the next begins.

// ── Product-page presentation ────────────────────────────────────────
// On a product page the visual *is* the content, not a teaser for a link, so
// it runs unconditionally: the provider hands down `true` rather than a hover
// flag. Reduced-motion still wins inside useMotionTick.
export function LiveVisual({
	caption,
	children,
}: {
	caption: string;
	children: React.ReactNode;
}) {
	return (
		<figure className="flex flex-col overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[var(--shadow-card)]">
			<figcaption className="flex items-center gap-2 border-b border-[var(--color-rule)] bg-[var(--color-paper-1)] px-5 py-2.5 font-mono text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]">
				<span
					className="block size-1.5 rounded-full motion-safe:animate-pulse"
					style={{ backgroundColor: "var(--color-status-live)" }}
				/>
				{caption}
			</figcaption>
			<div className="relative pt-5">
				<BentoHoverContext.Provider value={true}>
					{children}
				</BentoHoverContext.Provider>
			</div>
		</figure>
	);
}

// Which running demo belongs to which /product page. Slugs without an entry
// (cloud, swift) simply render no demo section rather than a placeholder.
export const PRIMITIVE_VISUALS: Record<
	string,
	{ caption: string; Visual: () => React.ReactElement }[]
> = {
	sync: [{ caption: "orders · subscribed", Visual: LiveQueryBento }],
	database: [
		{ caption: "schema.ts", Visual: SchemaBento },
		{ caption: "DATABASE_URL", Visual: EngineBento },
	],
	auth: [
		{ caption: "providers", Visual: AuthBento },
		{ caption: "policy evaluation", Visual: PolicyBento },
	],
	functions: [
		{ caption: "server functions", Visual: FunctionsBento },
		{ caption: "reactive server queries", Visual: ReactiveBento },
	],
	realtime: [{ caption: "room · presence", Visual: PresenceBento }],
	storage: [{ caption: "presigned uploads", Visual: UploadBento }],
	search: [{ caption: "faceted search", Visual: SearchBento }],
	workflows: [{ caption: "scheduler", Visual: SchedulerBento }],
	ssr: [{ caption: "GET /orders", Visual: SsrBento }],
	studio: [{ caption: "/studio", Visual: StudioTabsBento }],
};
