"use client";

import { Image, Link } from "@pylonsync/react";
import { createContext, useContext, useEffect, useState } from "react";
import {
	type LucideIcon,
	Activity,
	ArrowUpRight,
	Braces,
	Check,
	Copy,
	Database,
	Eye,
	FileText,
	KeyRound,
	LayoutDashboard,
	Radio,
	RefreshCw,
	Search,
	Server,
	ShieldCheck,
	Table2,
	Terminal,
	Upload,
	Workflow,
} from "lucide-react";
import { Button } from "./ui/button";
import { MarketingNav } from "./marketing-nav";
import { SiteFooter } from "./site-footer";
import { CodePanel } from "./code-panel";
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
	useMotionTick,
} from "./bento-visuals";
import { ctaUrl } from "../lib/account-urls";

// Concrete affordances that let a coding agent build, verify, and ship on Pylon.
// Each one ships with the artifact it describes (`visual`) — the claim and the
// evidence for it stay in the same card.
const AGENT_AFFORDANCES: {
	title: string;
	desc: string;
	icon: LucideIcon;
	href: string;
	visual: () => React.ReactElement;
}[] = [
	{
		title: "Rules live in the repo",
		desc: "New apps include AGENTS.md, and the Pylon skill installs with npx skills add pylonsync/pylon. The agent reads the conventions before it edits code.",
		icon: FileText,
		href: "/skill",
		visual: RepoVisual,
	},
	{
		title: "The path is command-line",
		desc: "npm create @pylonsync/pylon scaffolds the app. pylon dev runs it locally. pylon deploy ships it to Cloud.",
		icon: Terminal,
		href: "/product/cloud",
		visual: TerminalVisual,
	},
	{
		title: "Generated types catch drift",
		desc: "pylon codegen builds the client from your schema and functions. Bad entity names, missing fields, and wrong arguments fail at compile time.",
		icon: Braces,
		href: "/product/functions",
		visual: TypeErrorVisual,
	},
	{
		title: "Runtime state is visible",
		desc: "The agent can inspect tables, live queries, and logs in /studio while pylon dev runs. Debugging happens against current data.",
		icon: Eye,
		href: "/product/studio",
		visual: StudioVisual,
	},
];

// Signed-in state for the nav/CTA. Seeded from the SSR `auth` prop (resolved
// server-side from the shared SessionStore) so the correct CTA — "Dashboard"
// vs "Sign in" — is in the server-rendered HTML on first paint. No flash: the
// old version started at `null` and only learned the answer after a client
// /api/auth/session round-trip, so the CTA popped in after hydration. The
// effect just revalidates after a client-side nav back to this page.
function useSignedIn(initial: boolean): boolean {
	const [signedIn, setSignedIn] = useState<boolean>(initial);
	useEffect(() => {
		let cancelled = false;
		fetch("/api/auth/session", { credentials: "include" })
			.then((res) => res.json())
			.then((body) => {
				if (cancelled) return;
				const userId = body?.user?.id ?? body?.session?.user_id ?? null;
				setSignedIn(typeof userId === "string" && userId.length > 0);
			})
			.catch(() => {
				/* keep the SSR-seeded value on error */
			});
		return () => {
			cancelled = true;
		};
	}, []);
	return signedIn;
}

export function MarketingPage({
	initialSignedIn = false,
}: {
	initialSignedIn?: boolean;
}) {
	const signedIn = useSignedIn(initialSignedIn);

	return (
		<div className="min-h-screen bg-[var(--color-paper)] text-[var(--color-ink)]">
			<MarketingNav signedIn={signedIn} />

			{/* HERO — framework-first. The merged site leads with the framework
			    story (open source, runs anywhere); Smallware is the managed
			    way to ship it, covered further down + in the nav. */}
			<header className="relative isolate overflow-hidden">
				{/* Headline left, supporting copy in a right-hand column, CTAs beneath
				    the headline. DOM order is headline -> copy -> CTAs so the stacked
				    mobile layout reads correctly; the explicit row/column starts only
				    apply at lg, where the copy moves up beside the headline. */}
				<div className="mx-auto max-w-[1280px] px-5 pb-10 pt-14 sm:px-8 sm:pt-20">
					{/* The copy column is measured, not proportional: the headline caps
					    at 14ch, so a `1fr` first column just parks the copy against the
					    far edge and opens a canyon in the middle. 460px + a 48px gutter
					    keeps the two blocks reading as one paragraph-and-heading pair at
					    every width above lg. */}
					<div className="flex flex-col gap-7 lg:grid lg:grid-cols-[minmax(0,1fr)_minmax(0,460px)] lg:gap-x-12 lg:gap-y-9">
						<h1 className="max-w-[14ch] text-[clamp(42px,6.2vw,64px)] font-semibold leading-[1.0] tracking-[-0.045em] text-[var(--color-ink)] lg:col-start-1 lg:row-start-1">
							{/* NBSP, not a space: keeps the accented phrase on one line so the
							    headline breaks 15 / 13 / 9 instead of orphaning "agents". */}
							Full-stack apps <span className="text-[var(--color-cobalt)]">{"coding agents"}</span> can ship.
						</h1>

						<p className="text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px] lg:col-start-2 lg:row-start-1 lg:pt-2">
							Declare an entity once. Pylon derives the table, the access rules,
							the API, and the typed React client from it, so a schema change
							fails at compile time instead of in production.
						</p>

						<div className="flex flex-col items-start gap-3 lg:col-start-1 lg:row-start-2">
							<InstallCommand command="npm create @pylonsync/pylon@latest" />
							<div className="flex flex-col items-stretch gap-2.5 sm:flex-row sm:items-center sm:gap-3">
								<Button asChild variant="primary" size="lg">
									<a href="https://docs.pylonsync.com">Read the docs →</a>
								</Button>
								<Button asChild variant="ghost" size="lg">
									<Link href={ctaUrl(signedIn)}>
										{signedIn ? "Open dashboard" : "Create your account"}
									</Link>
								</Button>
							</div>
						</div>
					</div>
				</div>

				{/* The framework on screen: one card per pillar, each running the thing
				    it describes rather than illustrating it. Shares the copy's measure
				    so the hero reads as one left-aligned block. */}
				<div className="mx-auto mt-6 max-w-[1280px] px-5 pb-20 sm:mt-8 sm:px-8 sm:pb-24">
					<HeroBento />
				</div>
			</header>

			{/* Agent workflow. Each claim carries the artifact it's about — the
			    repo layout, the terminal, the type error, the studio table — so
			    the section reads as evidence instead of four more paragraphs. */}
			<Section id="agents" tone="sunken">
				<div>
					<H2>Give agents a system they can inspect.</H2>
					<p className="mt-5 max-w-[620px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Pylon keeps rules, commands, generated types, local data, and logs
						inside the workflow. Your agent can scaffold, run, debug, and
						deploy without guessing which console owns the next step.
					</p>
				</div>

				<div className="mt-14 grid gap-4 sm:grid-cols-2">
					{AGENT_AFFORDANCES.map((a) => (
						<Link
							key={a.title}
							href={a.href}
							className="group flex flex-col overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[var(--shadow-card)] transition-colors duration-200 ease-[var(--ease-out-quart)] hover:border-[var(--color-ink-4)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper-1)]"
						>
							{/* The artifact itself, cropped by the card. It used to sit in a
							    third frame — a bordered, shadowed panel inside a padded well
							    inside the card — so the reader was looking at an illustration
							    of a screenshot. One surface, one rule under it. */}
							<div className="h-[168px] overflow-hidden border-b border-[var(--color-rule)]">
								<a.visual />
							</div>
							<div className="flex flex-col gap-2 p-6">
								<div className="flex items-center gap-2">
									<a.icon className="size-4 shrink-0 text-[var(--color-cobalt)]" />
									<h3 className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
										{a.title}
									</h3>
									<ArrowUpRight className="ml-auto size-3.5 shrink-0 text-[var(--color-ink-4)] opacity-0 transition-opacity duration-200 group-focus-visible:opacity-100 group-hover:opacity-100" />
								</div>
								<p className="text-[14px] leading-[1.6] text-[var(--color-ink-2)]">
									{a.desc}
								</p>
							</div>
						</Link>
					))}
				</div>
			</Section>

			{/* The model */}
			<Section id="model">
				<div className="grid items-center gap-12 lg:grid-cols-2 lg:gap-16">
					<div>
						<H2>Your app model stays in TypeScript.</H2>
						<p className="mt-5 max-w-[460px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
							Declare an entity and its access policy. Pylon creates the table,
							REST and realtime API, row-level checks, and typed React client.
							That keeps resolvers, an ORM layer, and a separate backend service
							out of your stack.
						</p>
						<div className="mt-7">
							<Button asChild variant="ghost" size="lg">
								<a href="https://docs.pylonsync.com">Read the quickstart →</a>
							</Button>
						</div>
					</div>

					<CodePanel
						filename="app.ts"
						code={`// one entity → a synced table + typed client
const Order = entity("Order", {
  customer: field.string(),
  total: field.float(),
  paid: field.boolean().default(false),
});

// access rules next to the schema — deny by default
policy({ entity: "Order",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId == data.ownerId",
});

// the React side — live, typed, no fetch
const { data } = db.useQuery("Order");`}
					/>
				</div>
			</Section>

			{/* DEPLOY */}
			<Section id="deploy" tone="sunken">
				<div>
					<H2>Deploy from GitHub or the CLI.</H2>
					<p className="mt-5 max-w-[560px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Both paths reach the same Cloud runtime, so a repo push and a manual
						release produce the same deployment.
					</p>
				</div>

				<div className="mt-14 grid gap-4 lg:grid-cols-2">
					<Card>
						<h3 className="text-[18px] font-semibold tracking-tight text-[var(--color-ink)]">
							Connect GitHub
						</h3>
						<p className="mt-2.5 text-[14px] leading-[1.6] text-[var(--color-ink-2)]">
							Install the Smallware GitHub App once. Pull requests get preview
							environments that disappear after merge.
						</p>
						<ol className="mt-6 grid gap-2.5 text-[14px] leading-[1.55] text-[var(--color-ink-2)] [&>li]:flex [&>li]:gap-3 [&>li>span:first-child]:font-mono [&>li>span:first-child]:text-[var(--color-ink-3)] [&>li>span:first-child]:tabular-nums">
							<li>
								<span>1.</span>
								<span>Create a project and connect a repo.</span>
							</li>
							<li>
								<span>2.</span>
								<span>
									<InlineCode>git push origin main</InlineCode> triggers a deploy.
								</span>
							</li>
							<li>
								<span>3.</span>
								<span>
									Live at <InlineCode>your-app.smallware.run</InlineCode>.
								</span>
							</li>
						</ol>
					</Card>

					<Card>
						{/* Mono, but a heading — this sat in an <InlineCode> chip, so the
						    card's title rendered at 12px next to "Connect GitHub" at 18px
						    and the pair read as a heading beside a tag. */}
						<h3 className="font-mono text-[18px] font-semibold tracking-tight text-[var(--color-ink)]">
							pylon deploy
						</h3>
						<p className="mt-2.5 text-[14px] leading-[1.6] text-[var(--color-ink-2)]">
							Use the CLI for CI, locked-down environments, or a manual release.
						</p>
						<div className="mt-6 overflow-hidden rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper)] font-mono text-[12px] leading-[1.75] text-[var(--color-ink)]">
							{/* Same label strip as the four artifact cards above — plain mono,
							    sentence case. It was uppercase-tracked, which rendered
							    "MY-APP — PYLON DEPLOY" and shouted a filename. */}
							<div className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)] px-4 py-2 font-mono text-[10.5px] text-[var(--color-ink-3)]">
								my-app — pylon deploy
							</div>
							<div className="px-4 py-3">
								<div>
									<span className="text-[var(--color-cobalt)]">$</span> pylon login
								</div>
								<div>
									<span className="text-[var(--color-cobalt)]">$</span> pylon deploy{" "}
									<span className="text-[var(--color-ink-3)]">--target cloud</span>
								</div>
								<div className="text-[var(--color-status-live)]">  ✓ Build · 12s</div>
								<div className="text-[var(--color-status-live)]">  ✓ Schema synced</div>
								<div className="text-[var(--color-status-live)]">  ✓ Cutover · 0 errors</div>
								<div className="text-[var(--color-ink-3)]">  → https://acme.smallware.run</div>
							</div>
						</div>
					</Card>
				</div>
			</Section>

			{/* SCALE */}
			<Section id="scale">
				<div>
					<H2>Scale from one dashboard.</H2>
					<p className="mt-5 max-w-[560px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Every app sits behind a global edge network. Resize machines, add
						replicas and regions, or expand storage from the same dashboard,
						without pre-provisioning or per-seat pricing.
					</p>
				</div>

				{/* The Cloud dashboard itself — the managed surface for the framework
				    above. Given a browser frame and set wider than the copy column so
				    it reads as the section's centerpiece. Width/height match the
				    source (3456×2234) so the slot is reserved and CLS stays 0. */}
				<div className="mt-14">
					<div className="overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[0_40px_80px_-40px_rgba(15,23,42,0.45)]">
						<div className="flex items-center gap-2.5 border-b border-[var(--color-rule)] px-4 py-2.5">
							<span className="flex gap-1.5">
								<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
								<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
								<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
							</span>
							{/* The dashboard in the shot answers on the product host, not this
							    one. The bar used to read pylonsync.com/dashboard, which is a
							    404 — this site has no auth and no dashboard. */}
							<span className="mx-auto rounded-full border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-3 py-0.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
								usesmallware.com/dashboard
							</span>
						</div>
						<Image
							src="/marketing/pylon-cloud-dashboard.png"
							alt="Smallware dashboard — project overview with deployments, machine status, and live metrics"
							width={3456}
							height={2234}
							sizes="(min-width: 1120px) 1120px, 100vw"
							widths={[828, 1200, 2048]}
							className="block h-auto w-full"
						/>
					</div>
				</div>

				{/* A hairline ledger, not a card mesh. Ten items divide evenly across
				    two columns — the old three-column mesh left two dead cells that
				    rendered as gray voids. */}
				<div className="mt-12 grid gap-x-12 sm:grid-cols-2">
					{[
						["Global edge network", "Cloudflare's edge provides CDN caching, TLS, and DDoS protection worldwide with no extra configuration."],
						["Resize on demand", "Add RAM up to 64 GB, choose performance CPUs, and expand the volume without redeploying."],
						["Replicas", "Run up to 32 load-balanced replicas per region."],
						["Global regions", "Deploy in US, EU, APAC, and South America regions."],
						["Up to 500 GB volume", "Grow storage live when the app needs room."],
						["Managed Postgres — private beta", "Bundled SQLite by default; co-located managed Postgres is in private beta."],
						["Autostop on idle", "Scale to zero when idle, or keep a project always warm."],
						["Custom domains + TLS", "Bring your domain; Pylon handles TLS."],
						["SSO — OIDC + SAML", "Configure org-level SSO from the dashboard."],
						["Audit log + snapshots", "Activity log, one-click volume restore."],
					].map(([title, body]) => (
						<div
							key={title}
							className="flex flex-col gap-1.5 border-t border-[var(--color-rule)] py-5"
						>
							<h4 className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
								{title}
							</h4>
							<p className="text-[13px] leading-[1.55] text-[var(--color-ink-2)]">
								{body}
							</p>
						</div>
					))}
				</div>
			</Section>

			{/* FAQ — practical questions with checkable answers. This replaced a
			    "here are the objections we expect" block: writing the reader's
			    doubts for them is a rhetorical device, not information, and it left
			    the page with no plain statement of what Pylon actually supports. */}
			<Section id="faq" tone="sunken">
				<div>
					<H2>Common questions.</H2>
					<p className="mt-5 max-w-[560px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Everything else is in the{" "}
						<a
							href="https://docs.pylonsync.com"
							className="text-[var(--color-cobalt)] underline underline-offset-2"
						>
							docs
						</a>
						.
					</p>
				</div>

				<div className="mt-12 grid gap-x-14 sm:grid-cols-2">
					{[
						{
							q: "Which database does it use?",
							a: "SQLite by default — one file, nothing to provision. Set DATABASE_URL to a Postgres connection string and the same schema and application code target Postgres instead. On Cloud, bundled SQLite is the default and co-located managed Postgres is in private beta.",
						},
						{
							q: "Do I have to use Smallware?",
							a: "No. The runtime is a single open-source binary — run it on your own box or container platform with a volume for SQLite, or point it at your own Postgres. Cloud is the managed path, not a requirement, and it runs the same binary.",
						},
						{
							q: "How do migrations work?",
							a: "Your schema is TypeScript. In development Pylon diffs it and applies the change on save, so the tables follow the file. On deploy the schema is applied as part of the release, before traffic cuts over.",
						},
						{
							q: "How do I deploy?",
							a: "Two ways into the same runtime. Install the GitHub App and pushes to your default branch deploy, with pull requests getting preview environments. Or run pylon deploy from your machine or CI when you want a manual release.",
						},
						{
							q: "Which clients can talk to it?",
							a: "A typed React client with server-side rendering, and a Swift SDK for mobile. Every entity also gets a REST and realtime API, so anything that can speak HTTP or WebSocket can read and write subject to the same policies.",
						},
						{
							q: "What does auth cover?",
							a: "Magic-link email, 25+ OAuth providers, generic OIDC discovery, guest sessions, and API keys. Whatever the caller signed in with, policies read the same auth.userId, so access rules do not change per provider.",
						},
						{
							q: "Can it run background work?",
							a: "Yes — ctx.scheduler.runAfter, runAt, and cancel schedule follow-up work, and delays and retries run in the same process as the rest of your app. There is no separate queue or worker to deploy.",
						},
						{
							q: "What happens to my data if I leave?",
							a: "It is a SQLite file or an ordinary Postgres database, with no proprietary storage layer in between. Take a dump and it opens in any client. What you would rewrite on the way out is the SDK calls, not the data.",
						},
					].map((o) => (
						<div
							key={o.q}
							className="flex flex-col gap-2.5 border-t border-[var(--color-rule)] py-6"
						>
							<h3 className="text-[15px] font-semibold leading-snug tracking-tight text-[var(--color-ink)]">
								{o.q}
							</h3>
							<p className="text-[14px] leading-[1.65] text-[var(--color-ink-2)]">
								{o.a}
							</p>
						</div>
					))}
				</div>
			</Section>

			{/* CTA. Sits on the same column and heading scale as every section
			    above it. It used to be centred in an 860px box at its own
			    clamp(34,5vw,60), so the page's single left edge — and its type
			    scale — broke on the last screen. */}
			<Section>
				<H2>Create a Pylon app.</H2>
				<p className="mt-5 max-w-[520px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
					The framework is free to self-host. Smallware runs it for you —
					connect GitHub or deploy from the CLI.
				</p>
				<div className="mt-8 flex flex-col items-stretch gap-2.5 sm:flex-row sm:items-center sm:gap-3">
					<Button asChild variant="primary" size="lg">
						<Link href={ctaUrl(signedIn)}>
							{signedIn ? "Open dashboard →" : "Create your account →"}
						</Link>
					</Button>
					<Button asChild variant="ghost" size="lg">
						<a href="https://docs.pylonsync.com/cloud">Read the docs</a>
					</Button>
				</div>
			</Section>

			<SiteFooter />
		</div>
	);
}

// Copy-to-clipboard install command for the hero. Click anywhere on the pill
// to copy; the icon flips to a check for a beat. Falls back silently if the
// Clipboard API is unavailable (older browsers / insecure origins).
function InstallCommand({ command }: { command: string }) {
	const [copied, setCopied] = useState(false);
	function copy() {
		navigator.clipboard
			?.writeText(command)
			.then(() => {
				setCopied(true);
				setTimeout(() => setCopied(false), 1600);
			})
			.catch(() => {
				/* clipboard unavailable — no-op */
			});
	}
	return (
		<button
			type="button"
			onClick={copy}
			aria-label={`Copy: ${command}`}
			// max-w-full + nowrap: on a 390px screen the command wrapped inside the
			// pill, which centred the two halves under a vertically-centred `$` and
			// stopped looking like a command line. It sheds a point of type below sm
			// instead, where it fits on one line.
			className="group inline-flex max-w-full items-center gap-3 rounded-full border border-[var(--color-rule)] bg-[var(--color-paper)] py-3 pl-5 pr-4 font-mono text-[12px] text-[var(--color-ink)] shadow-[var(--shadow-card)] transition-colors hover:border-[var(--color-cobalt)]/50 hover:bg-[var(--color-paper-1)] sm:text-[13px]"
		>
			<span className="select-none text-[var(--color-cobalt)]">$</span>
			<span className="truncate tracking-tight">{command}</span>
			<span className="ml-1 text-[var(--color-ink-3)] transition-colors group-hover:text-[var(--color-ink)]">
				{copied ? (
					<Check className="size-4 text-[var(--color-status-live)]" />
				) : (
					<Copy className="size-4" />
				)}
			</span>
		</button>
	);
}

// ── Hero furniture ───────────────────────────────────────────────────
// ── Agent affordance visuals ─────────────────────────────────────────
// Small, honest mocks of the four artifacts an agent actually touches. They
// are cropped by their card, so each reads as a window onto a real surface.
function VisualBar({
	label,
	right,
}: {
	label: string;
	right?: React.ReactNode;
}) {
	return (
		// No traffic-light dots. Between these four cards and the browser frame
		// around the dashboard screenshot the page was drawing five fake windows,
		// and the dots said nothing the label doesn't say better.
		<div className="flex items-center gap-2 border-b border-[var(--color-rule)] bg-[var(--color-paper-1)] px-4 py-2">
			<span className="truncate font-mono text-[10.5px] text-[var(--color-ink-3)]">
				{label}
			</span>
			{right && <span className="ml-auto shrink-0">{right}</span>}
		</div>
	);
}

function RepoVisual() {
	const files: { name: string; hint: string; mark?: boolean }[] = [
		{ name: "AGENTS.md", hint: "conventions, read first", mark: true },
		{ name: "schema/index.ts", hint: "entities · policies" },
		{ name: "functions/orders.ts", hint: "queries · mutations" },
		{ name: "app/page.tsx", hint: "db.useQuery" },
	];
	return (
		<div>
			<VisualBar label="my-app" />
			<div className="px-4 py-3 font-mono text-[11.5px] leading-[1.95]">
				{files.map((f) => (
					<div key={f.name} className="flex items-center gap-2">
						<span
							className={
								f.mark
									? "text-[var(--color-cobalt)]"
									: "text-[var(--color-ink-4)]"
							}
						>
							{f.mark ? "▸" : "·"}
						</span>
						<span
							className={
								f.mark
									? "font-medium text-[var(--color-cobalt)]"
									: "text-[var(--color-ink-2)]"
							}
						>
							{f.name}
						</span>
						<span className="truncate text-[var(--color-ink-4)]">{f.hint}</span>
					</div>
				))}
			</div>
		</div>
	);
}

function TerminalVisual() {
	return (
		<div>
			<VisualBar label="zsh — my-app" />
			<div className="px-4 py-3 font-mono text-[11.5px] leading-[1.9]">
				<div className="text-[var(--color-ink-2)]">
					<span className="text-[var(--color-cobalt)]">$</span> npm create
					@pylonsync/pylon
				</div>
				<div className="text-[var(--color-ink-4)]">&nbsp;&nbsp;✓ scaffolded my-app</div>
				<div className="text-[var(--color-ink-2)]">
					<span className="text-[var(--color-cobalt)]">$</span> pylon dev
				</div>
				<div className="text-[var(--color-status-live)]">
					&nbsp;&nbsp;✓ schema applied · studio ready
				</div>
				<div className="text-[var(--color-ink-3)]">
					&nbsp;&nbsp;→ http://localhost:3000
				</div>
			</div>
		</div>
	);
}

function TypeErrorVisual() {
	return (
		<div>
			<VisualBar
				label="app/page.tsx"
				right={
					<span className="font-mono text-[10.5px] uppercase tracking-[0.1em] text-[var(--color-status-fail)]">
						1 error
					</span>
				}
			/>
			<div className="px-4 py-3 font-mono text-[11.5px] leading-[1.9]">
				<div className="text-[var(--color-ink-3)]">
					<span className="text-[var(--color-cobalt)]">const</span> {"{ data }"} ={" "}
					db.useQuery(
					<span className="text-[var(--color-status-live)] underline decoration-[var(--color-status-fail)] decoration-wavy underline-offset-[3px]">
						&quot;Ordr&quot;
					</span>
					);
				</div>
				<div className="mt-2 rounded-[var(--radius-sm)] border border-[var(--color-status-fail)]/25 bg-[var(--color-status-fail-soft)] px-2 py-1.5 text-[10.5px] leading-[1.5] text-[var(--color-ink-2)]">
					Argument of type <span className="text-[var(--color-status-fail)]">&quot;Ordr&quot;</span>{" "}
					is not assignable to &quot;Order&quot; | &quot;Customer&quot;.
				</div>
			</div>
		</div>
	);
}

function StudioVisual() {
	const rows: [string, string, string][] = [
		["ord_9f2a", "Sarah Chen", "$1,240"],
		["ord_7c41", "Marcus Lee", "$880"],
		["ord_5b88", "Priya Nair", "$2,100"],
	];
	return (
		<div>
			<VisualBar
				label="/studio — Order"
				right={
					<span className="inline-flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
						<span
							className="block size-1.5 rounded-full"
							style={{ backgroundColor: "var(--color-status-live)" }}
						/>
						live
					</span>
				}
			/>
			<div className="divide-y divide-[var(--color-rule-soft)]">
				{rows.map(([id, name, total]) => (
					<div
						key={id}
						className="flex items-center gap-3 px-4 py-[7px] font-mono text-[11.5px]"
					>
						<span className="w-[62px] shrink-0 text-[var(--color-ink-4)]">{id}</span>
						<span className="flex-1 truncate font-sans text-[12px] text-[var(--color-ink-2)]">
							{name}
						</span>
						<span className="shrink-0 tabular-nums text-[var(--color-ink-3)]">
							{total}
						</span>
					</div>
				))}
			</div>
		</div>
	);
}

// ── Hero feature bento ───────────────────────────────────────────────
// The framework's pillars, each carrying a live visual instead of a static
// illustration. Every visual is decorative, so all of it is gated on
// prefers-reduced-motion: `still` freezes the loop, and each card is written so
// that its resting frame is already a complete, truthful picture — nothing is
// only legible mid-animation.
function BentoCard({
	icon: Icon,
	title,
	desc,
	href,
	className,
	children,
}: {
	icon: LucideIcon;
	title: string;
	desc: string;
	href: string;
	className?: string;
	children: React.ReactNode;
}) {
	const [hovered, setHovered] = useState(false);
	return (
		// The whole card is the link to its product page. The cards used to sit
		// under `filter: grayscale` until pointed at — but these twelve are the
		// only cards on the site whose contents actually carry colour, so at rest
		// the page's centrepiece was twelve grey panels and the reveal was a
		// gimmick a reader had to find. They render in their own colour now, and
		// hover moves the border only.
		<Link
			href={href}
			onMouseEnter={() => setHovered(true)}
			onMouseLeave={() => setHovered(false)}
			onFocus={() => setHovered(true)}
			onBlur={() => setHovered(false)}
			className={`group flex flex-col overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[var(--shadow-card)] transition-colors duration-200 ease-[var(--ease-out-quart)] hover:border-[var(--color-ink-4)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper)] ${className ?? ""}`}
		>
			<div className="flex flex-col gap-1.5 p-5 pb-3">
				<div className="flex items-center gap-2">
					<Icon className="size-4 shrink-0 text-[var(--color-cobalt)]" />
					<h3 className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
						{title}
					</h3>
					<ArrowUpRight className="ml-auto size-3.5 shrink-0 text-[var(--color-ink-4)] opacity-0 transition-opacity duration-200 group-focus-visible:opacity-100 group-hover:opacity-100" />
				</div>
				<p className="text-[13px] leading-[1.5] text-[var(--color-ink-2)]">{desc}</p>
			</div>
			<div className="relative min-h-0 flex-1 overflow-hidden">
				<BentoHoverContext.Provider value={hovered}>
					{children}
				</BentoHoverContext.Provider>
			</div>
		</Link>
	);
}

// Two card heights, not four. The grid ran 290 / 230 / 210 / 230 down its four
// rows, so every row landed on a different baseline and the block read as four
// unrelated grids stacked up. One height for the three lead cards, one for
// everything under them. Cards in a row already stretch to their tallest
// sibling, so these floors are set at where the copy actually lands — a lower
// floor is inert on the dense rows and leaves the two wide cards short.
const BENTO_LEAD = "min-h-[290px]";
const BENTO_ROW = "min-h-[280px]";

function HeroBento() {
	return (
		<div>
			<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-12">
				<BentoCard
				icon={Table2}
				title="Typed schema"
				href="/product/database"
				desc="Entities and fields in TypeScript. Pylon creates the tables and migrates on save."
				className={`${BENTO_LEAD} lg:col-span-4`}
				>
				<SchemaBento />
				</BentoCard>

				<BentoCard
				icon={RefreshCw}
				title="Live queries"
				href="/product/sync"
				desc="db.useQuery opens a subscription. Every write pushes a diff — no polling, no refetch."
				className={`${BENTO_LEAD} lg:col-span-4`}
				>
				<LiveQueryBento />
				</BentoCard>

				<BentoCard
				icon={ShieldCheck}
				title="Row-level policies"
				href="/product/auth"
				desc="Access rules sit next to the schema. Every read and write is checked; default deny."
				className={`${BENTO_LEAD} lg:col-span-4`}
				>
				<PolicyBento />
				</BentoCard>

				<BentoCard
				icon={KeyRound}
				title="Auth, included"
				href="/product/auth"
				desc="Magic link, 25+ OAuth providers, OIDC, and API keys."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<AuthBento />
				</BentoCard>

				<BentoCard
				icon={Upload}
				title="File uploads"
				href="/product/storage"
				desc="Presigned uploads to local disk or any S3-compatible bucket."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<UploadBento />
				</BentoCard>

				<BentoCard
				icon={Radio}
				title="Rooms & presence"
				href="/product/realtime"
				desc="Cursors, typing, and who is online over the same server."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<PresenceBento />
				</BentoCard>

				<BentoCard
				icon={Search}
				title="Faceted search"
				href="/product/search"
				desc="Full-text queries with live facet counts, updated in the same transaction as your writes."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<SearchBento />
				</BentoCard>

				<BentoCard
				icon={Database}
				title="SQLite or Postgres"
				href="/product/database"
				desc="Start on a single SQLite file. Point DATABASE_URL at Postgres and nothing above the driver changes."
				className={`${BENTO_ROW} lg:col-span-6`}
				>
				<EngineBento />
				</BentoCard>

				<BentoCard
				icon={Server}
				title="Server-rendered React"
				href="/product/ssr"
				desc="Queries and policies run on the server for first paint, then the same typed client hydrates and subscribes."
				className={`${BENTO_ROW} lg:col-span-6`}
				>
				<SsrBento />
				</BentoCard>

				<BentoCard
				icon={Braces}
				title="Server functions"
				href="/product/functions"
				desc="Queries, mutations, and actions in TypeScript files, validated and called through the typed client."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<FunctionsBento />
				</BentoCard>

				<BentoCard
				icon={Activity}
				title="Reactive server queries"
				href="/product/functions"
				desc="Joins and derived data. The server tracks what it read and re-runs when those rows change."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<ReactiveBento />
				</BentoCard>

				<BentoCard
				icon={Workflow}
				title="Scheduled & deferred work"
				href="/product/workflows"
				desc="runAfter, runAt, and cancel. Delays and retries run in the same process — no separate worker."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<SchedulerBento />
				</BentoCard>

				<BentoCard
				icon={LayoutDashboard}
				title="Admin studio"
				href="/product/studio"
				desc="Browse tables, inspect live queries, and tail logs at /studio. Admin-gated in production."
				className={`${BENTO_ROW} lg:col-span-3`}
				>
				<StudioTabsBento />
				</BentoCard>
			</div>

			{/* Carries the thesis and the /product link that the primitives grid
			    used to hold, now that the grid is gone. The line opened on a bold
			    "Pick what you need." — a slogan the sentence after it didn't need. */}
			<div className="mt-6 flex flex-col items-start justify-between gap-3 border-t border-[var(--color-rule)] pt-5 sm:flex-row sm:items-center">
				<p className="text-[15px] leading-[1.6] text-[var(--color-ink-2)]">
					Every piece shares one schema and one runtime.
				</p>
				<Button asChild variant="ghost" size="sm">
					<Link href="/product">Explore the product →</Link>
				</Button>
			</div>
		</div>
	);
}

function Section({
	id,
	tone = "paper",
	children,
}: {
	id?: string;
	tone?: "paper" | "sunken";
	children: React.ReactNode;
}) {
	return (
		<section
			id={id}
			className={`border-t border-[var(--color-rule)]${
				tone === "sunken" ? " bg-[var(--color-paper-1)]" : ""
			}`}
		>
			<div className="mx-auto max-w-[1280px] px-5 py-16 sm:px-8 sm:py-20 lg:py-24">
				{children}
			</div>
		</section>
	);
}

function H2({ children }: { children: React.ReactNode }) {
	return (
		<h2 className="max-w-[20ch] text-[clamp(30px,4vw,46px)] font-semibold leading-[1.05] tracking-[-0.035em] text-[var(--color-ink)]">
			{children}
		</h2>
	);
}

function Card({ children }: { children: React.ReactNode }) {
	return (
		<div className="rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] p-7 sm:p-8">
			{children}
		</div>
	);
}

function InlineCode({ children }: { children: React.ReactNode }) {
	return (
		<code className="rounded-[2px] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-1.5 py-0.5 font-mono text-[12px] text-[var(--color-ink)]">
			{children}
		</code>
	);
}
