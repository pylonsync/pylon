"use client";

import {
	Braces,
	Database,
	type LucideIcon,
	RefreshCw,
	Server,
	ShieldCheck,
	Table2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { PylonMark } from "./brand";

// A Pylon app is not a pile of separate products. These services meet in one
// runtime, so the hero draws them as a connected circuit around a single core.
// The moving trace explains that relationship one service at a time. Hovering
// a label lets a reader inspect it without fighting the automatic sequence.
const SERVICES: {
	label: string;
	detail: string;
	side: "left" | "right";
	icon: LucideIcon;
	node: readonly [number, number];
	labelY: number;
	path: string;
}[] = [
	{
		label: "Typed schema",
		detail: "entities, migrations, types",
		side: "left",
		icon: Table2,
		node: [398, 108],
		labelY: 92,
		path: "M398 108 L440 130 L480 153",
	},
	{
		label: "Server-rendered React",
		detail: "streamed and hydrated",
		side: "right",
		icon: Server,
		node: [602, 108],
		labelY: 92,
		path: "M602 108 L560 130 L520 153",
	},
	{
		label: "Auth + policies",
		detail: "row-level, default deny",
		side: "left",
		icon: ShieldCheck,
		node: [344, 166],
		labelY: 166,
		path: "M344 166 L430 166 L474 166",
	},
	{
		label: "Live queries",
		detail: "subscriptions, no polling",
		side: "right",
		icon: RefreshCw,
		node: [656, 166],
		labelY: 166,
		path: "M656 166 L570 166 L526 166",
	},
	{
		label: "Database + files",
		detail: "SQLite, Postgres, S3",
		side: "left",
		icon: Database,
		node: [420, 232],
		labelY: 244,
		path: "M420 232 L458 211 L487 181",
	},
	{
		label: "Server functions",
		detail: "queries, mutations, actions",
		side: "right",
		icon: Braces,
		node: [580, 232],
		labelY: 244,
		path: "M580 232 L542 211 L513 181",
	},
];

const VIEWBOX = { width: 1000, height: 330 };
const CYCLE_MS = 2600;

function diamondPath(
	cx: number,
	cy: number,
	halfWidth: number,
	halfHeight: number,
) {
	return `M ${cx} ${cy - halfHeight} L ${cx + halfWidth} ${cy} L ${cx} ${cy + halfHeight} L ${cx - halfWidth} ${cy} Z`;
}

function roundedDiamondPath(
	cx: number,
	cy: number,
	halfWidth: number,
	halfHeight: number,
) {
	const cornerX = 6;
	const cornerY = 3;
	return [
		`M ${cx - cornerX} ${cy - halfHeight + cornerY}`,
		`Q ${cx} ${cy - halfHeight} ${cx + cornerX} ${cy - halfHeight + cornerY}`,
		`L ${cx + halfWidth - cornerX} ${cy - cornerY}`,
		`Q ${cx + halfWidth} ${cy} ${cx + halfWidth - cornerX} ${cy + cornerY}`,
		`L ${cx + cornerX} ${cy + halfHeight - cornerY}`,
		`Q ${cx} ${cy + halfHeight} ${cx - cornerX} ${cy + halfHeight - cornerY}`,
		`L ${cx - halfWidth + cornerX} ${cy + cornerY}`,
		`Q ${cx - halfWidth} ${cy} ${cx - halfWidth + cornerX} ${cy - cornerY}`,
		"Z",
	].join(" ");
}

function useReducedMotion(): boolean {
	const [reduced, setReduced] = useState(false);

	useEffect(() => {
		const query = window.matchMedia("(prefers-reduced-motion: reduce)");
		const update = () => setReduced(query.matches);
		update();
		query.addEventListener("change", update);
		return () => query.removeEventListener("change", update);
	}, []);

	return reduced;
}

export function HeroStack() {
	const [active, setActive] = useState(0);
	const [paused, setPaused] = useState(false);
	const reducedMotion = useReducedMotion();

	useEffect(() => {
		if (paused || reducedMotion) return;
		const timer = window.setInterval(
			() => setActive((current) => (current + 1) % SERVICES.length),
			CYCLE_MS,
		);
		return () => window.clearInterval(timer);
	}, [paused, reducedMotion]);

	const activeService = SERVICES[active];
	const ActiveServiceIcon = activeService.icon;

	return (
		<div className="relative mx-auto mt-8 w-full max-w-[1000px] select-none overflow-hidden sm:mt-14 lg:overflow-visible">
			<style>{`
				@keyframes pylon-runtime-enter {
					from { opacity: 0; transform: translateY(14px) scale(.985); }
					to { opacity: 1; transform: translateY(0) scale(1); }
				}
				@keyframes pylon-runtime-flow {
					to { stroke-dashoffset: -1; }
				}
				@keyframes pylon-runtime-breathe {
					0%, 100% { opacity: .12; transform: scale(.92); }
					50% { opacity: .24; transform: scale(1); }
				}
				.pylon-runtime-art { animation: pylon-runtime-enter 900ms cubic-bezier(.16,1,.3,1) both; }
				.pylon-runtime-flow { animation: pylon-runtime-flow 1150ms linear infinite; }
				.pylon-runtime-breathe { animation: pylon-runtime-breathe 2200ms ease-in-out infinite; transform-origin: center; }
				.pylon-runtime-leader { display: none; }
				@media (min-width: 1024px) {
					.pylon-runtime-leader { display: inline; }
				}
				@media (prefers-reduced-motion: reduce) {
					.pylon-runtime-art, .pylon-runtime-flow, .pylon-runtime-breathe { animation: none; }
				}
			`}</style>

			<div className="pylon-runtime-art">
				<svg
					viewBox={`0 0 ${VIEWBOX.width} ${VIEWBOX.height}`}
					role="img"
					aria-label="Pylon services connected through one runtime"
					className="w-[160%] max-w-none -translate-x-[18.75%] lg:w-full lg:translate-x-0"
				>
					<defs>
						<linearGradient id="runtime-top" x1="0" y1="0" x2="1" y2="1">
							<stop offset="0%" stopColor="var(--color-paper)" />
							<stop offset="100%" stopColor="var(--color-paper-1)" />
						</linearGradient>
						<linearGradient id="runtime-edge" x1="0" y1="0" x2="0" y2="1">
							<stop
								offset="0%"
								stopColor="var(--color-brand)"
								stopOpacity="0.14"
							/>
							<stop
								offset="100%"
								stopColor="var(--color-brand)"
								stopOpacity="0.035"
							/>
						</linearGradient>
						<filter
							id="runtime-shadow"
							x="-25%"
							y="-25%"
							width="150%"
							height="165%"
						>
							<feDropShadow
								dx="0"
								dy="14"
								stdDeviation="14"
								floodColor="var(--color-brand)"
								floodOpacity="0.09"
							/>
						</filter>
					</defs>

					{/* The thin edge gives the circuit a physical base without turning it
				    into VoidZero's stack of interchangeable product slabs. */}
					<path
						d="M276 163 L500 283 L724 163 L724 181 L508 299 Q500 304 492 299 L276 181 Z"
						fill="url(#runtime-edge)"
						stroke="var(--color-rule)"
					/>
					<path
						d="M500 39 Q506 39 513 43 L718 153 Q730 159 718 166 L513 277 Q500 284 487 277 L282 166 Q270 159 282 153 L487 43 Q494 39 500 39 Z"
						fill="url(#runtime-top)"
						stroke="var(--color-rule)"
						filter="url(#runtime-shadow)"
					/>

					{/* Two routing rails make the outer board legible as one circuit. */}
					<path
						d="M500 64 L687 163 L500 264 L313 163 Z"
						fill="none"
						stroke="var(--color-rule-soft)"
						strokeDasharray="3 6"
					/>
					<path
						d="M500 94 L632 164 L500 236 L368 164 Z"
						fill="none"
						stroke="var(--color-rule-soft)"
						strokeDasharray="2 6"
					/>

					{SERVICES.map((service, index) => {
						const isActive = index === active;
						const [x, y] = service.node;
						const NodeIcon = service.icon;
						const leaderStart = service.side === "left" ? 248 : 752;
						const leaderEnd = service.side === "left" ? x - 20 : x + 20;

						return (
							<g key={service.label}>
								<line
									x1={leaderStart}
									y1={service.labelY}
									x2={leaderEnd}
									y2={y}
									stroke={
										isActive ? "var(--color-brand)" : "var(--color-ink-4)"
									}
									strokeDasharray={isActive ? "1 0" : "2 5"}
									strokeOpacity={isActive ? 1 : 0.42}
									className="pylon-runtime-leader"
									style={{
										transition:
											"stroke 420ms ease, stroke-dasharray 420ms ease, stroke-opacity 420ms ease",
									}}
								/>

								<path
									d={service.path}
									fill="none"
									stroke="var(--color-ink-4)"
									strokeOpacity="0.38"
									strokeWidth="1.25"
									strokeDasharray="3 5"
								/>
								{isActive ? (
									<path
										d={service.path}
										pathLength="1"
										fill="none"
										stroke="var(--color-brand)"
										strokeWidth="2"
										strokeLinecap="round"
										strokeDasharray=".16 .84"
										className="pylon-runtime-flow"
									/>
								) : null}

								<g
									style={{
										transform: isActive ? "translateY(-5px)" : "translateY(0)",
										transformOrigin: `${x}px ${y}px`,
										transition: "transform 480ms cubic-bezier(.16,1,.3,1)",
									}}
								>
									<path
										d={roundedDiamondPath(x, y, 20, 11)}
										fill={
											isActive ? "var(--color-brand)" : "var(--color-paper)"
										}
										stroke={
											isActive ? "var(--color-brand)" : "var(--color-ink-3)"
										}
										style={{ transition: "fill 360ms ease, stroke 360ms ease" }}
									/>
									<NodeIcon
										x={x - 7}
										y={y - 7}
										width={14}
										height={14}
										strokeWidth={1.75}
										color={
											isActive ? "var(--color-brand-fg)" : "var(--color-ink-3)"
										}
										aria-hidden="true"
										style={{
											transition: "color 360ms ease",
											pointerEvents: "none",
										}}
									/>
								</g>
							</g>
						);
					})}

					{/* The center stays constant while the services move around it: one
				    runtime is the product, not whichever feature happens to be active. */}
					<ellipse
						cx="500"
						cy="167"
						rx="70"
						ry="38"
						fill="var(--color-brand)"
						className="pylon-runtime-breathe"
					/>
					<path
						d={diamondPath(500, 167, 48, 27)}
						fill="var(--color-paper)"
						stroke="var(--color-brand)"
						strokeWidth="1.5"
					/>
					<PylonMark
						x={488}
						y={149}
						size={24}
						className="text-[var(--color-brand)]"
						aria-hidden="true"
					/>
				</svg>
			</div>

			{SERVICES.map((service, index) => (
				<ServiceLabel
					key={service.label}
					service={service}
					active={index === active}
					onSelect={() => setActive(index)}
					onPauseChange={setPaused}
				/>
			))}

			{/* Desktop labels sit on the traces. Small screens get one compact
			    caption so the art remains readable instead of shrinking six callouts. */}
			<div className="-mt-1 min-h-12 text-center lg:hidden">
				<div className="flex items-center justify-center gap-2">
					<span className="grid size-6 place-items-center rounded-[var(--radius-sm)] border border-[var(--color-brand)] bg-[var(--color-brand)] text-[var(--color-brand-fg)] shadow-[var(--shadow-card-hover)]">
						<ActiveServiceIcon
							className="size-3.5"
							strokeWidth={1.75}
							aria-hidden="true"
						/>
					</span>
					<p className="font-mono text-[10.5px] font-medium uppercase tracking-[0.14em] text-[var(--color-brand)]">
						{activeService.label}
					</p>
				</div>
				<p className="mt-1 text-[12px] text-[var(--color-ink-3)]">
					{activeService.detail}
				</p>
			</div>
		</div>
	);
}

function ServiceLabel({
	service,
	active,
	onSelect,
	onPauseChange,
}: {
	service: (typeof SERVICES)[number];
	active: boolean;
	onSelect: () => void;
	onPauseChange: (paused: boolean) => void;
}) {
	const onLeft = service.side === "left";
	const Icon = service.icon;

	return (
		<div
			className={`absolute hidden -translate-y-1/2 lg:block ${onLeft ? "text-right" : "text-left"}`}
			style={{
				top: `${(service.labelY / VIEWBOX.height) * 100}%`,
				[onLeft ? "right" : "left"]: "76%",
			}}
		>
			<button
				type="button"
				onMouseEnter={() => {
					onPauseChange(true);
					onSelect();
				}}
				onMouseLeave={() => onPauseChange(false)}
				onFocus={() => onPauseChange(true)}
				onBlur={() => onPauseChange(false)}
				onClick={onSelect}
				aria-pressed={active}
				className={`inline-flex items-center gap-2.5 whitespace-nowrap rounded-[var(--radius-sm)] px-1.5 py-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper)] ${onLeft ? "flex-row-reverse" : ""}`}
			>
				<span
					className="grid size-6 place-items-center rounded-[var(--radius-sm)] border"
					style={{
						borderColor: active ? "var(--color-brand)" : "var(--color-ink-4)",
						background: active ? "var(--color-brand)" : "var(--color-paper-1)",
						color: active ? "var(--color-brand-fg)" : "var(--color-ink-3)",
						boxShadow: active
							? "var(--shadow-card-hover)"
							: "var(--shadow-card)",
						transform: active ? "translateY(-1px)" : "translateY(0)",
						transition:
							"background 320ms ease, border-color 320ms ease, color 320ms ease, transform 320ms ease",
					}}
				>
					<Icon className="size-3.5" strokeWidth={1.75} />
				</span>
				<span
					className="font-mono text-[10.5px] font-medium uppercase tracking-[0.14em]"
					style={{
						color: active ? "var(--color-brand)" : "var(--color-ink-2)",
						opacity: active ? 1 : 0.72,
						transition: "color 320ms ease, opacity 320ms ease",
					}}
				>
					{service.label}
				</span>
			</button>
			<p
				className="mt-0.5 whitespace-nowrap px-1.5 text-[11.5px] text-[var(--color-ink-3)]"
				style={{
					opacity: active ? 1 : 0,
					transform: active ? "translateY(0)" : "translateY(-3px)",
					transition: "opacity 320ms ease, transform 320ms ease",
				}}
			>
				{service.detail}
			</p>
		</div>
	);
}
