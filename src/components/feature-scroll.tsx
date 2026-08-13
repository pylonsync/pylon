"use client";

import { Link } from "@pylonsync/react";
import {
	Activity,
	ArrowUpRight,
	Braces,
	Database,
	KeyRound,
	LayoutDashboard,
	Radio,
	Search,
	Server,
	ShieldCheck,
	Upload,
	Workflow,
	type LucideIcon,
} from "lucide-react";
import {
	type CSSProperties,
	type ReactNode,
	useEffect,
	useRef,
	useState,
} from "react";
import {
	AuthBento,
	BentoHoverContext,
	EngineBento,
	FunctionsBento,
	LiveQueryBento,
	PolicyBento,
	PresenceBento,
	ReactiveBento,
	SchedulerBento,
	SchemaBento,
	SearchBento,
	SsrBento,
	StudioTabsBento,
	UploadBento,
} from "./bento-visuals";
import { FRAME_COL } from "./marketing-frame";

type FeatureKey =
	"data" | "security" | "realtime" | "files" | "runtime" | "operations";

type FeatureChapter = {
	key: FeatureKey;
	label: string;
	title: string;
	description: string;
	details: string[];
	href: string;
	icon: LucideIcon;
	background: string;
};

const FEATURES: FeatureChapter[] = [
	{
		key: "data",
		label: "Typed data",
		title: "One schema, all the way through.",
		description:
			"Define entities in TypeScript and let Pylon keep storage, migrations, server code, and the client on the same model.",
		details: ["Typed schema", "Automatic migrations", "SQLite or Postgres"],
		href: "/product/database",
		icon: Database,
		background:
			"radial-gradient(circle at 18% 18%, rgba(167,139,250,.42), transparent 34%), radial-gradient(circle at 82% 74%, rgba(109,40,217,.28), transparent 42%), #15131d",
	},
	{
		key: "security",
		label: "Auth & policies",
		title: "Identity and access share the model.",
		description:
			"Add sign-in, API keys, and row-level rules without splitting authorization across another service and another dashboard.",
		details: ["25+ providers", "Row-level policies", "Default deny"],
		href: "/product/auth",
		icon: ShieldCheck,
		background:
			"linear-gradient(135deg, rgba(124,58,237,.35), transparent 42%), repeating-linear-gradient(125deg, transparent 0 22px, rgba(196,181,253,.055) 23px 24px), #14121b",
	},
	{
		key: "realtime",
		label: "Realtime",
		title: "Every write can become an update.",
		description:
			"Queries stream precise diffs while rooms carry presence and ephemeral events over the same server—no polling layer required.",
		details: ["Live queries", "Rooms & presence", "Transactional diffs"],
		href: "/product/realtime",
		icon: Radio,
		background:
			"radial-gradient(circle at 72% 28%, rgba(196,181,253,.34), transparent 18%), radial-gradient(circle at 72% 28%, transparent 0 24%, rgba(139,92,246,.18) 24.5% 25%, transparent 25.5% 38%, rgba(139,92,246,.1) 38.5% 39%, transparent 39.5%), #12121a",
	},
	{
		key: "files",
		label: "Files & search",
		title: "Uploads and discovery stay in sync.",
		description:
			"Move files through presigned uploads and update full-text indexes and facet counts in the same transaction as application data.",
		details: ["S3-compatible files", "Full-text search", "Live facets"],
		href: "/product/storage",
		icon: Upload,
		background:
			"linear-gradient(90deg, transparent 0 18%, rgba(139,92,246,.22) 18% 19%, transparent 19% 54%, rgba(196,181,253,.11) 54% 54.5%, transparent 54.5%), radial-gradient(ellipse at 50% 100%, rgba(109,40,217,.32), transparent 58%), #14131b",
	},
	{
		key: "runtime",
		label: "Server runtime",
		title: "Render, call, and react in one process.",
		description:
			"Server-render React against current data, expose typed functions, and let derived queries re-run only when their dependencies change.",
		details: ["Server-rendered React", "Typed functions", "Reactive queries"],
		href: "/product/functions",
		icon: Server,
		background:
			"conic-gradient(from 210deg at 72% 44%, rgba(124,58,237,.4), transparent 22%, rgba(196,181,253,.16) 35%, transparent 52%), linear-gradient(145deg, #17141f, #101016)",
	},
	{
		key: "operations",
		label: "Workflows & studio",
		title: "Background work stays observable.",
		description:
			"Schedule durable jobs, inspect current data, watch live queries, and tail logs without adding a worker fleet or a separate admin product.",
		details: ["Scheduled work", "Retries", "Admin studio"],
		href: "/product/workflows",
		icon: Workflow,
		background:
			"radial-gradient(circle at 22% 74%, rgba(167,139,250,.34), transparent 24%), radial-gradient(circle at 78% 24%, rgba(109,40,217,.3), transparent 30%), linear-gradient(120deg, #111118, #181421)",
	},
];

// The section is intentionally dark in both themes. Local tokens let the live
// demos keep using the shared design system without inheriting the page's
// light-mode surfaces. Product violet remains the only decorative accent;
// green and red are reserved for meaningful runtime states.
const DARK_TOKENS = {
	"--color-paper": "#111116",
	"--color-paper-1": "#17171d",
	"--color-paper-2": "#202028",
	"--color-rule": "#34343e",
	"--color-rule-soft": "#292932",
	"--color-ink": "#fafafa",
	"--color-ink-2": "#d4d4d8",
	"--color-ink-3": "#a1a1aa",
	"--color-ink-4": "#71717a",
	"--color-brand": "#a78bfa",
	"--color-brand-soft": "rgba(139,92,246,.16)",
	"--color-accent-blue": "#a78bfa",
	"--color-accent-purple": "#8b5cf6",
	"--color-accent-pink": "#c4b5fd",
	"--color-status-live": "#34d399",
	"--color-status-fail": "#fb7185",
	"--color-status-fail-soft": "rgba(251,113,133,.12)",
} as CSSProperties;

export function FeatureScroll() {
	const [activeIndex, setActiveIndex] = useState(0);
	const [revealed, setRevealed] = useState<Set<number>>(() => new Set());
	const rows = useRef<(HTMLElement | null)[]>([]);

	useEffect(() => {
		const prefersReducedMotion = window.matchMedia(
			"(prefers-reduced-motion: reduce)",
		).matches;
		if (prefersReducedMotion) {
			setRevealed(new Set(FEATURES.map((_, index) => index)));
		}

		const revealObserver = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (!entry.isIntersecting) continue;
					const index = Number(
						(entry.target as HTMLElement).dataset.featureIndex,
					);
					setRevealed((current) => {
						if (current.has(index)) return current;
						const next = new Set(current);
						next.add(index);
						return next;
					});
				}
			},
			{ rootMargin: "0px 0px -18%", threshold: 0.12 },
		);

		for (const row of rows.current) {
			if (row) revealObserver.observe(row);
		}

		let frame = 0;
		const updateActive = () => {
			cancelAnimationFrame(frame);
			frame = requestAnimationFrame(() => {
				const focusLine = window.innerHeight * 0.44;
				let closestIndex = 0;
				let closestDistance = Number.POSITIVE_INFINITY;

				rows.current.forEach((row, index) => {
					if (!row) return;
					const rect = row.getBoundingClientRect();
					const sample = rect.top + Math.min(rect.height * 0.34, 240);
					const distance = Math.abs(sample - focusLine);
					if (distance < closestDistance) {
						closestDistance = distance;
						closestIndex = index;
					}
				});

				setActiveIndex(closestIndex);
			});
		};

		updateActive();
		window.addEventListener("scroll", updateActive, { passive: true });
		window.addEventListener("resize", updateActive);

		return () => {
			cancelAnimationFrame(frame);
			revealObserver.disconnect();
			window.removeEventListener("scroll", updateActive);
			window.removeEventListener("resize", updateActive);
		};
	}, []);

	useEffect(() => {
		if (window.innerWidth >= 1024) return;
		const activeTab = document.querySelector<HTMLElement>(
			`[data-feature-tab="${FEATURES[activeIndex]?.key}"]`,
		);
		activeTab?.scrollIntoView({
			behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
				? "auto"
				: "smooth",
			block: "nearest",
			inline: "center",
		});
	}, [activeIndex]);

	return (
		<section
			id="included"
			style={DARK_TOKENS}
			className="border-y border-[var(--color-rule)] bg-[var(--color-paper)] text-[var(--color-ink)]"
		>
			<div className={`${FRAME_COL} border-[var(--color-rule)]`}>
				<div className="border-b border-[var(--color-rule)] px-5 py-3 sm:px-8">
					<span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
						What ships with it
					</span>
				</div>

				<div className="border-b border-[var(--color-rule)] px-5 py-16 sm:px-8 sm:py-20 lg:py-24">
					<h2 className="max-w-[18ch] text-[clamp(34px,4.5vw,56px)] font-semibold leading-[1.02] tracking-[-0.04em]">
						Everything an app needs, moving as one.
					</h2>
					<p className="mt-6 max-w-[650px] text-[16px] leading-[1.65] text-[var(--color-ink-3)] sm:text-[17px]">
						One schema connects storage, security, realtime, server code, and
						operations. Scroll through the runtime from data model to
						production.
					</p>
				</div>

				<div className="lg:grid lg:grid-cols-[238px_minmax(0,1fr)]">
					<FeatureRail activeIndex={activeIndex} />

					<div className="min-w-0 lg:border-l lg:border-[var(--color-rule)]">
						<MobileFeatureRail activeIndex={activeIndex} />
						{FEATURES.map((feature, index) => (
							<article
								key={feature.key}
								id={`feature-${feature.key}`}
								ref={(node) => {
									rows.current[index] = node;
								}}
								data-feature-index={index}
								className={`scroll-mt-28 border-b border-[var(--color-rule)] px-5 py-16 transition-[opacity,transform] duration-700 ease-[var(--ease-out-quart)] last:border-b-0 motion-reduce:translate-y-0 motion-reduce:transition-none sm:px-8 lg:grid lg:min-h-[670px] lg:grid-cols-[minmax(250px,0.72fr)_minmax(430px,1.28fr)] lg:items-center lg:gap-10 lg:px-10 lg:py-20 xl:gap-14 xl:px-14 ${
									revealed.has(index)
										? "translate-y-0 opacity-100"
										: "translate-y-8 opacity-0"
								}`}
							>
								<FeatureCopy feature={feature} index={index} />
								<div className="mt-10 min-w-0 lg:mt-0">
									<BentoHoverContext.Provider value={activeIndex === index}>
										<FeatureVisual feature={feature} />
									</BentoHoverContext.Provider>
								</div>
							</article>
						))}
					</div>
				</div>
			</div>
		</section>
	);
}

function FeatureRail({ activeIndex }: { activeIndex: number }) {
	return (
		<aside className="relative hidden lg:block">
			<nav
				aria-label="Pylon capabilities"
				className="sticky top-[98px] px-6 py-12"
			>
				<ol className="space-y-1">
					{FEATURES.map((feature, index) => {
						const Icon = feature.icon;
						const active = activeIndex === index;
						return (
							<li key={feature.key}>
								<a
									href={`#feature-${feature.key}`}
									aria-current={active ? "step" : undefined}
									className={`group flex items-center gap-3 rounded-[var(--radius-md)] px-3 py-2.5 text-[13px] transition-colors duration-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] ${
										active
											? "bg-[var(--color-brand-soft)] text-[var(--color-ink)]"
											: "text-[var(--color-ink-4)] hover:text-[var(--color-ink-2)]"
									}`}
								>
									<span
										className={`flex size-8 shrink-0 items-center justify-center rounded-[9px] border transition-colors duration-300 ${
											active
												? "border-[var(--color-brand)]/55 bg-[var(--color-brand)] text-[#111116]"
												: "border-[var(--color-rule)] text-[var(--color-ink-4)] group-hover:border-[var(--color-ink-4)]"
										}`}
									>
										<Icon className="size-3.5" />
									</span>
									<span>{feature.label}</span>
								</a>
							</li>
						);
					})}
				</ol>
			</nav>
		</aside>
	);
}

function MobileFeatureRail({ activeIndex }: { activeIndex: number }) {
	return (
		<nav
			aria-label="Pylon capabilities"
			className="sticky top-[62px] z-30 overflow-x-auto border-b border-[var(--color-rule)] bg-[var(--color-paper)]/92 px-5 py-3 backdrop-blur-xl lg:hidden"
		>
			<div className="flex w-max gap-5">
				{FEATURES.map((feature, index) => {
					const Icon = feature.icon;
					const active = index === activeIndex;
					return (
						<a
							key={feature.key}
							href={`#feature-${feature.key}`}
							data-feature-tab={feature.key}
							aria-current={active ? "step" : undefined}
							className={`flex items-center gap-1.5 border-b py-1.5 text-[12px] transition-colors ${
								active
									? "border-[var(--color-brand)] text-[var(--color-ink)]"
									: "border-transparent text-[var(--color-ink-4)]"
							}`}
						>
							<Icon className="size-3" />
							{feature.label}
						</a>
					);
				})}
			</div>
		</nav>
	);
}

function FeatureCopy({
	feature,
	index,
}: {
	feature: FeatureChapter;
	index: number;
}) {
	const Icon = feature.icon;
	return (
		<div>
			<div className="flex items-center gap-3 font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--color-brand)]">
				<span className="flex size-9 items-center justify-center rounded-[10px] border border-[var(--color-brand)]/45 bg-[var(--color-brand-soft)]">
					<Icon className="size-4" />
				</span>
				<span>0{index + 1}</span>
			</div>
			<h3 className="mt-6 max-w-[15ch] text-[clamp(28px,3.2vw,42px)] font-semibold leading-[1.06] tracking-[-0.035em]">
				{feature.title}
			</h3>
			<p className="mt-5 max-w-[42ch] text-[15px] leading-[1.65] text-[var(--color-ink-3)]">
				{feature.description}
			</p>
			<div className="mt-7 border-y border-[var(--color-rule)]">
				{feature.details.map((detail, detailIndex) => (
					<div
						key={detail}
						className="flex items-center gap-3 border-b border-[var(--color-rule-soft)] py-2.5 text-[12.5px] text-[var(--color-ink-2)] last:border-b-0"
					>
						<span className="font-mono text-[9.5px] text-[var(--color-ink-4)]">
							0{detailIndex + 1}
						</span>
						{detail}
					</div>
				))}
			</div>
			<Link
				href={feature.href}
				className="group mt-8 inline-flex items-center gap-2 text-[13.5px] font-medium text-[var(--color-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)]"
			>
				Explore {feature.label.toLowerCase()}
				<ArrowUpRight className="size-3.5 text-[var(--color-brand)] transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
			</Link>
		</div>
	);
}

function FeatureVisual({ feature }: { feature: FeatureChapter }) {
	return (
		<div
			className="relative overflow-hidden rounded-[var(--radius-xl)] border border-white/10 p-3 shadow-[0_32px_80px_-38px_rgba(109,40,217,.75)] sm:p-6"
			style={{ background: feature.background }}
		>
			<div
				aria-hidden="true"
				className="pointer-events-none absolute inset-0 opacity-25"
				style={{
					backgroundImage:
						"linear-gradient(rgba(255,255,255,.1) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,.1) 1px, transparent 1px)",
					backgroundSize: "40px 40px",
					maskImage:
						"linear-gradient(to bottom right, rgba(0,0,0,.78), transparent 78%)",
				}}
			/>
			<div className="relative z-10 min-h-[390px]">
				{feature.key === "data" ? <DataVisual /> : null}
				{feature.key === "security" ? <SecurityVisual /> : null}
				{feature.key === "realtime" ? <RealtimeVisual /> : null}
				{feature.key === "files" ? <FilesVisual /> : null}
				{feature.key === "runtime" ? <RuntimeVisual /> : null}
				{feature.key === "operations" ? <OperationsVisual /> : null}
			</div>
		</div>
	);
}

function DemoPanel({
	label,
	icon: Icon,
	className = "",
	children,
}: {
	label: string;
	icon: LucideIcon;
	className?: string;
	children: ReactNode;
}) {
	return (
		<div
			className={`flex min-h-[186px] min-w-0 flex-col overflow-hidden rounded-[var(--radius-lg)] border border-white/15 bg-[#101015]/94 shadow-[0_18px_50px_-32px_rgba(0,0,0,.95)] backdrop-blur-sm ${className}`}
		>
			<div className="flex items-center gap-2 border-b border-[var(--color-rule-soft)] px-4 py-2.5 font-mono text-[9.5px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]">
				<Icon className="size-3 text-[var(--color-brand)]" />
				{label}
				<span className="ml-auto flex gap-1">
					<span className="size-1 rounded-full bg-[var(--color-ink-4)]" />
					<span className="size-1 rounded-full bg-[var(--color-brand)]" />
				</span>
			</div>
			<div className="min-h-0 flex-1 pt-5">{children}</div>
		</div>
	);
}

function DataVisual() {
	return (
		<div className="grid min-h-[390px] content-center gap-3 xl:grid-cols-[.82fr_1.18fr]">
			<DemoPanel
				label="schema.ts"
				icon={Braces}
				className="xl:translate-y-[-18px]"
			>
				<SchemaBento />
			</DemoPanel>
			<DemoPanel
				label="database target"
				icon={Database}
				className="xl:translate-y-[18px]"
			>
				<EngineBento />
			</DemoPanel>
		</div>
	);
}

function SecurityVisual() {
	return (
		<div className="relative grid min-h-[390px] content-center gap-3 sm:grid-cols-[1.12fr_.88fr]">
			<DemoPanel
				label="policy evaluation"
				icon={ShieldCheck}
				className="sm:min-h-[260px]"
			>
				<PolicyBento />
			</DemoPanel>
			<DemoPanel label="session" icon={KeyRound} className="sm:mt-16">
				<AuthBento />
			</DemoPanel>
		</div>
	);
}

function RealtimeVisual() {
	return (
		<div className="grid min-h-[390px] content-center gap-3 sm:grid-cols-[1.15fr_.85fr]">
			<DemoPanel
				label="live orders"
				icon={Activity}
				className="sm:min-h-[300px]"
			>
				<LiveQueryBento />
			</DemoPanel>
			<DemoPanel label="room:orders" icon={Radio} className="sm:mt-20">
				<PresenceBento />
			</DemoPanel>
		</div>
	);
}

function FilesVisual() {
	return (
		<div className="grid min-h-[390px] content-center gap-3 sm:grid-cols-[.85fr_1.15fr]">
			<DemoPanel label="uploads" icon={Upload} className="sm:mt-16">
				<UploadBento />
			</DemoPanel>
			<DemoPanel
				label="search index"
				icon={Search}
				className="sm:min-h-[280px]"
			>
				<SearchBento />
			</DemoPanel>
		</div>
	);
}

function RuntimeVisual() {
	return (
		<div className="grid min-h-[390px] content-center gap-3 sm:grid-cols-2">
			<DemoPanel label="GET /orders" icon={Server} className="sm:col-span-2">
				<SsrBento />
			</DemoPanel>
			<DemoPanel label="functions" icon={Braces}>
				<FunctionsBento />
			</DemoPanel>
			<DemoPanel label="dependency graph" icon={Activity}>
				<ReactiveBento />
			</DemoPanel>
		</div>
	);
}

function OperationsVisual() {
	return (
		<div className="grid min-h-[390px] content-center gap-3 sm:grid-cols-[.88fr_1.12fr]">
			<DemoPanel label="job queue" icon={Workflow} className="sm:mt-16">
				<SchedulerBento />
			</DemoPanel>
			<DemoPanel
				label="/studio"
				icon={LayoutDashboard}
				className="sm:min-h-[280px]"
			>
				<StudioTabsBento />
			</DemoPanel>
		</div>
	);
}
