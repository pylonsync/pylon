"use client";

import { Link } from "@pylonsync/react";
import {
	Activity,
	ArrowUpRight,
	Braces,
	Clock3,
	Database,
	KeyRound,
	LayoutDashboard,
	Radio,
	RefreshCw,
	Search,
	Server,
	ShieldCheck,
	Table2,
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
	WorkflowBento,
} from "./bento-visuals";
import { FRAME_COL } from "./marketing-frame";

type FeatureKey =
	| "schema"
	| "live-queries"
	| "policies"
	| "auth"
	| "uploads"
	| "presence"
	| "search"
	| "database"
	| "ssr"
	| "functions"
	| "reactive-queries"
	| "jobs"
	| "workflows"
	| "studio";

type FeatureChapter = {
	key: FeatureKey;
	label: string;
	title: string;
	description: string;
	details: string[];
	href: string;
	icon: LucideIcon;
	visualLabel: string;
	background: string;
};

const FEATURES: FeatureChapter[] = [
	{
		key: "schema",
		label: "Typed schema",
		title: "You define the data in TypeScript.",
		description:
			"Write each entity and field one time. Pylon makes the tables, runs the migrations, and writes the client types.",
		details: ["TypeScript entities", "Migrations on save", "Generated client types"],
		href: "/product/database",
		icon: Table2,
		visualLabel: "schema.ts",
		background:
			"radial-gradient(circle at 18% 18%, rgba(167,139,250,.42), transparent 34%), radial-gradient(circle at 82% 74%, rgba(109,40,217,.28), transparent 42%), #15131d",
	},
	{
		key: "live-queries",
		label: "Live queries",
		title: "Queries update themselves.",
		description:
			"Subscribe one time. After each write that affects the result, the server sends only the rows that changed. You do not poll or clear a cache.",
		details: ["Typed subscriptions", "Row-level updates", "Catch-up after reconnect"],
		href: "/product/sync",
		icon: RefreshCw,
		visualLabel: "live orders",
		background:
			"radial-gradient(circle at 72% 28%, rgba(196,181,253,.34), transparent 18%), radial-gradient(circle at 72% 28%, transparent 0 24%, rgba(139,92,246,.18) 24.5% 25%, transparent 25.5% 38%, rgba(139,92,246,.1) 38.5% 39%, transparent 39.5%), #12121a",
	},
	{
		key: "policies",
		label: "Row-level policies",
		title: "Access rules sit next to the data.",
		description:
			"Every read and every write goes through the same rules. An operation with no rule is denied.",
		details: ["Per-row checks", "Caller identity", "Denied by default"],
		href: "/product/auth",
		icon: ShieldCheck,
		visualLabel: "policy evaluation",
		background:
			"linear-gradient(135deg, rgba(124,58,237,.35), transparent 42%), repeating-linear-gradient(125deg, transparent 0 22px, rgba(196,181,253,.055) 23px 24px), #14121b",
	},
	{
		key: "auth",
		label: "Auth",
		title: "Sign-in is already built in.",
		description:
			"Use email links, OAuth, OIDC, or API keys. Your functions and your access rules read the same session.",
		details: ["Email links", "25+ OAuth providers", "OIDC and API keys"],
		href: "/product/auth",
		icon: KeyRound,
		visualLabel: "session providers",
		background:
			"radial-gradient(ellipse at 24% 22%, rgba(167,139,250,.34), transparent 38%), linear-gradient(160deg, transparent 42%, rgba(109,40,217,.2)), #121118",
	},
	{
		key: "uploads",
		label: "File uploads",
		title: "Uploads go straight to storage.",
		description:
			"The app signs the upload. Files go to local disk in development and to any S3-compatible bucket in production.",
		details: ["Signed uploads", "Local disk in development", "S3-compatible buckets"],
		href: "/product/storage",
		icon: Upload,
		visualLabel: "upload queue",
		background:
			"linear-gradient(90deg, transparent 0 18%, rgba(139,92,246,.22) 18% 19%, transparent 19% 54%, rgba(196,181,253,.11) 54% 54.5%, transparent 54.5%), radial-gradient(ellipse at 50% 100%, rgba(109,40,217,.32), transparent 58%), #14131b",
	},
	{
		key: "presence",
		label: "Rooms & presence",
		title: "Short-lived state stays out of your tables.",
		description:
			"Send cursors, typing signals, and who is online over a separate channel. None of it is written to your tables.",
		details: ["Rooms", "Presence", "Broadcast events"],
		href: "/product/realtime",
		icon: Radio,
		visualLabel: "room:orders",
		background:
			"radial-gradient(circle at 28% 70%, rgba(196,181,253,.3), transparent 16%), radial-gradient(circle at 70% 30%, rgba(124,58,237,.3), transparent 24%), #111119",
	},
	{
		key: "search",
		label: "Faceted search",
		title: "Search stays in step with the data.",
		description:
			"Run full-text queries with ranked results and facet counts. The index updates in the same transaction as the write.",
		details: ["BM25 ranking", "Live facet counts", "Indexed in the write"],
		href: "/product/search",
		icon: Search,
		visualLabel: "search index",
		background:
			"conic-gradient(from 30deg at 68% 34%, rgba(167,139,250,.3), transparent 28%, rgba(109,40,217,.18), transparent 62%), #14121b",
	},
	{
		key: "database",
		label: "SQLite or Postgres",
		title: "One SQLite file, or Postgres.",
		description:
			"Start on a single SQLite file. To move to Postgres, point DATABASE_URL at it. Your schema, rules, and client code do not change.",
		details: ["SQLite by default", "Postgres when you need it", "One data API"],
		href: "/product/database",
		icon: Database,
		visualLabel: "database target",
		background:
			"radial-gradient(ellipse at 50% 0%, rgba(167,139,250,.32), transparent 48%), repeating-linear-gradient(90deg, transparent 0 48px, rgba(196,181,253,.045) 49px 50px), #121218",
	},
	{
		key: "ssr",
		label: "File-based SSR",
		title: "React renders on the same server.",
		description:
			"Pages are files. The server reads your data, streams the HTML, then hands the page to the client, which subscribes for updates.",
		details: ["File routes", "Streamed HTML", "Hydrate and subscribe"],
		href: "/product/ssr",
		icon: Server,
		visualLabel: "GET /orders",
		background:
			"conic-gradient(from 210deg at 72% 44%, rgba(124,58,237,.4), transparent 22%, rgba(196,181,253,.16) 35%, transparent 52%), linear-gradient(145deg, #17141f, #101016)",
	},
	{
		key: "functions",
		label: "Server functions",
		title: "A TypeScript file is the endpoint.",
		description:
			"Write a query, a mutation, or an action. Pylon checks the inputs and gives the client a typed function to call it.",
		details: ["Queries", "Mutations", "Actions"],
		href: "/product/functions",
		icon: Braces,
		visualLabel: "functions",
		background:
			"linear-gradient(125deg, rgba(109,40,217,.28), transparent 38%), repeating-linear-gradient(0deg, transparent 0 31px, rgba(196,181,253,.05) 32px 33px), #121118",
	},
	{
		key: "reactive-queries",
		label: "Reactive server queries",
		title: "Derived data reruns only when its inputs change.",
		description:
			"The server records which rows a join or a total read. It runs the query again only when one of those rows changes.",
		details: ["Tracked reads", "Server-side joins", "Reruns only what changed"],
		href: "/product/functions",
		icon: Activity,
		visualLabel: "dependency graph",
		background:
			"radial-gradient(circle at 50% 50%, rgba(167,139,250,.28), transparent 22%), radial-gradient(circle at 50% 50%, transparent 0 34%, rgba(139,92,246,.11) 34.5% 35%, transparent 35.5%), #111118",
	},
	{
		key: "jobs",
		label: "Background jobs",
		title: "Slow work runs after the response.",
		description:
			"Queue email, file processing, and totals to run after you reply to the user. Schedule work once or on a repeat.",
		details: ["Background jobs", "runAfter and runAt", "Scheduled work"],
		href: "/product/workflows",
		icon: Clock3,
		visualLabel: "job queue",
		background:
			"radial-gradient(ellipse at 50% 105%, rgba(139,92,246,.4), transparent 52%), repeating-linear-gradient(90deg, transparent 0 54px, rgba(196,181,253,.06) 55px 56px), #121218",
	},
	{
		key: "workflows",
		label: "Workflows",
		title: "Long jobs continue after a restart.",
		description:
			"Each step is saved as it finishes. A job can wait days for an event, then continue after a deploy or a crash. Finished steps do not run twice.",
		details: ["Saved steps", "Waits for time or events", "Safe retries"],
		href: "/product/workflows",
		icon: Workflow,
		visualLabel: "durable workflow",
		background:
			"radial-gradient(circle at 22% 74%, rgba(167,139,250,.34), transparent 24%), radial-gradient(circle at 78% 24%, rgba(109,40,217,.3), transparent 30%), linear-gradient(120deg, #111118, #181421)",
	},
	{
		key: "studio",
		label: "Studio",
		title: "You can see what the server is doing.",
		description:
			"Open Studio to read the tables, watch live queries, follow the logs, and run a mutation against the environment you are debugging.",
		details: ["Table browser", "Live query inspector", "Logs and mutations"],
		href: "/product/studio",
		icon: LayoutDashboard,
		visualLabel: "/studio",
		background:
			"linear-gradient(145deg, rgba(109,40,217,.24), transparent 48%), radial-gradient(circle at 76% 70%, rgba(196,181,253,.25), transparent 28%), repeating-linear-gradient(0deg, transparent 0 34px, rgba(196,181,253,.045) 35px 36px), #13121a",
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
		const activeObserver = new IntersectionObserver(
			() => {
				const focusLine = window.innerHeight * 0.43;
				const index = rows.current.findIndex((row) => {
					if (!row) return false;
					const rect = row.getBoundingClientRect();
					return rect.top <= focusLine && rect.bottom > focusLine;
				});
				if (index >= 0) setActiveIndex(index);
			},
			{ rootMargin: "-42% 0px -56%", threshold: [0, 1] },
		);

		for (const row of rows.current) {
			if (!row) continue;
			revealObserver.observe(row);
			activeObserver.observe(row);
		}

		return () => {
			revealObserver.disconnect();
			activeObserver.disconnect();
		};
	}, []);

	useEffect(() => {
		const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
		if (window.innerWidth >= 1024) {
			const rail = document.querySelector<HTMLElement>("[data-feature-rail]");
			const activeRailItem = document.querySelector<HTMLElement>(
				`[data-feature-rail-tab="${FEATURES[activeIndex]?.key}"]`,
			);
			if (rail && activeRailItem) {
				rail.scrollTo({
					top:
						activeRailItem.offsetTop -
						rail.clientHeight / 2 +
						activeRailItem.offsetHeight / 2,
					behavior: reduced ? "auto" : "smooth",
				});
			}
			return;
		}
		const activeTab = document.querySelector<HTMLElement>(
			`[data-feature-tab="${FEATURES[activeIndex]?.key}"]`,
		);
		activeTab?.scrollIntoView({
			behavior: reduced ? "auto" : "smooth",
			block: "nearest",
			inline: "center",
		});
	}, [activeIndex]);

	const selectFeature = (index: number) => {
		const feature = FEATURES[index];
		if (!feature) return;
		setActiveIndex(index);
		document.getElementById(`feature-${feature.key}`)?.scrollIntoView({
			behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
				? "auto"
				: "smooth",
			block: "start",
		});
		window.history.pushState(null, "", `#feature-${feature.key}`);
	};

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
						Everything an app needs, in one server.
					</h2>
					<p className="mt-6 max-w-[650px] text-[16px] leading-[1.65] text-[var(--color-ink-3)] sm:text-[17px]">
						You write one schema. The same server stores the data, checks
						permissions, pushes updates, runs your code, and renders the pages.
						You do not add a second service for any of it.
					</p>
				</div>

				<div className="lg:grid lg:grid-cols-[238px_minmax(0,1fr)]">
					<FeatureRail activeIndex={activeIndex} onSelect={selectFeature} />

					<div className="min-w-0 lg:border-l lg:border-[var(--color-rule)]">
						<MobileFeatureRail
							activeIndex={activeIndex}
							onSelect={selectFeature}
						/>
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
								<FeatureCopy feature={feature} />
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

function FeatureRail({
	activeIndex,
	onSelect,
}: {
	activeIndex: number;
	onSelect: (index: number) => void;
}) {
	return (
		<aside className="relative hidden lg:block">
			<nav
				aria-label="Pylon capabilities"
				data-feature-rail
				className="sticky top-[82px] max-h-[calc(100dvh-102px)] overflow-y-auto px-4 py-5"
			>
				<ol className="space-y-0.5">
					{FEATURES.map((feature, index) => {
						const Icon = feature.icon;
						const active = activeIndex === index;
						return (
							<li key={feature.key}>
								<button
									type="button"
									onClick={() => onSelect(index)}
									data-feature-rail-tab={feature.key}
									aria-current={active ? "step" : undefined}
									className={`group flex w-full items-center gap-2 rounded-[var(--radius-md)] px-2 py-1.5 text-left text-[12px] transition-colors duration-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] ${
										active
											? "bg-[var(--color-brand-soft)] text-[var(--color-ink)]"
											: "text-[var(--color-ink-4)] hover:text-[var(--color-ink-2)]"
									}`}
								>
									<span
										className={`flex size-7 shrink-0 items-center justify-center rounded-[8px] border transition-colors duration-300 ${
											active
												? "border-[var(--color-brand)]/55 bg-[var(--color-brand)] text-[#111116]"
												: "border-[var(--color-rule)] text-[var(--color-ink-4)] group-hover:border-[var(--color-ink-4)]"
										}`}
									>
										<Icon className="size-3" />
									</span>
									<span>{feature.label}</span>
								</button>
							</li>
						);
					})}
				</ol>
			</nav>
		</aside>
	);
}

function MobileFeatureRail({
	activeIndex,
	onSelect,
}: {
	activeIndex: number;
	onSelect: (index: number) => void;
}) {
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
						<button
							type="button"
							onClick={() => onSelect(index)}
							key={feature.key}
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
						</button>
					);
				})}
			</div>
		</nav>
	);
}

function FeatureCopy({ feature }: { feature: FeatureChapter }) {
	const Icon = feature.icon;
	return (
		<div>
			<div className="flex items-center gap-3 text-[12px] font-medium text-[var(--color-brand)]">
				<span className="flex size-9 items-center justify-center rounded-[10px] border border-[var(--color-brand)]/45 bg-[var(--color-brand-soft)]">
					<Icon className="size-4" />
				</span>
				<span>{feature.label}</span>
			</div>
			<h3 className="mt-6 max-w-[15ch] text-[clamp(28px,3.2vw,42px)] font-semibold leading-[1.06] tracking-[-0.035em]">
				{feature.title}
			</h3>
			<p className="mt-5 max-w-[42ch] text-[15px] leading-[1.65] text-[var(--color-ink-3)]">
				{feature.description}
			</p>
			<div className="mt-7 border-y border-[var(--color-rule)]">
				{feature.details.map((detail) => (
					<div
						key={detail}
						className="border-b border-[var(--color-rule-soft)] py-2.5 text-[12.5px] text-[var(--color-ink-2)] last:border-b-0"
					>
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
			<div className="relative z-10 flex min-h-[390px] items-center justify-center">
				<DemoPanel
					label={feature.visualLabel}
					icon={feature.icon}
					className={`w-full max-w-[510px] sm:min-h-[280px] ${VISUAL_PLACEMENT[feature.key]}`}
				>
					<FeatureDemo featureKey={feature.key} />
				</DemoPanel>
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
			</div>
			<div className="min-h-0 flex-1 pt-5">{children}</div>
		</div>
	);
}

const VISUAL_PLACEMENT: Record<FeatureKey, string> = {
	schema: "sm:-translate-x-4 sm:-translate-y-3",
	"live-queries": "sm:translate-x-4 sm:translate-y-3",
	policies: "sm:translate-x-3 sm:-translate-y-4",
	auth: "sm:-translate-x-5 sm:translate-y-4",
	uploads: "sm:translate-x-5 sm:-translate-y-2",
	presence: "sm:-translate-x-3 sm:translate-y-5",
	search: "sm:translate-x-4 sm:translate-y-2",
	database: "sm:-translate-x-4 sm:-translate-y-4",
	ssr: "sm:translate-x-3 sm:-translate-y-3",
	functions: "sm:-translate-x-5 sm:translate-y-3",
	"reactive-queries": "sm:translate-x-5 sm:translate-y-4",
	jobs: "sm:-translate-x-3 sm:-translate-y-4",
	workflows: "sm:translate-x-4 sm:-translate-y-2",
	studio: "sm:-translate-x-4 sm:translate-y-4",
};

function FeatureDemo({ featureKey }: { featureKey: FeatureKey }) {
	switch (featureKey) {
		case "schema":
			return <SchemaBento />;
		case "live-queries":
			return <LiveQueryBento />;
		case "policies":
			return <PolicyBento />;
		case "auth":
			return <AuthBento />;
		case "uploads":
			return <UploadBento />;
		case "presence":
			return <PresenceBento />;
		case "search":
			return <SearchBento />;
		case "database":
			return <EngineBento />;
		case "ssr":
			return <SsrBento />;
		case "functions":
			return <FunctionsBento />;
		case "reactive-queries":
			return <ReactiveBento />;
		case "jobs":
			return <SchedulerBento />;
		case "workflows":
			return <WorkflowBento />;
		case "studio":
			return <StudioTabsBento />;
	}
}
