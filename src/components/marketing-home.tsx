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
import { PricingPlans } from "./pricing-plans";
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
				<div className="mx-auto max-w-[1180px] px-5 pb-10 pt-14 sm:px-8 sm:pt-20">
					<div className="flex flex-col gap-7 lg:grid lg:grid-cols-[minmax(0,1fr)_minmax(0,400px)] lg:gap-x-16 lg:gap-y-9">
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
									<Link href={signedIn ? "/dashboard" : "/signup"}>
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
				<div className="mx-auto mt-6 max-w-[1180px] px-5 pb-20 sm:mt-8 sm:px-8 sm:pb-24">
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
							className="group flex flex-col overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[var(--shadow-card)] transition-[filter,box-shadow] duration-300 ease-[var(--ease-out-quart)] hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper-1)] [@media(hover:hover)]:grayscale [@media(hover:hover)]:focus-visible:grayscale-0 [@media(hover:hover)]:hover:grayscale-0"
						>
							{/* The artifact, cropped by the card so it reads as a window
							    onto something real rather than a boxed illustration. */}
							<div className="relative h-[186px] overflow-hidden bg-[var(--color-paper-1)] px-6 pt-6">
								<div className="h-full overflow-hidden rounded-t-[var(--radius-lg)] border border-b-0 border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[0_8px_24px_-16px_rgba(15,23,42,0.35)]">
									<a.visual />
								</div>
							</div>
							<div className="flex flex-col gap-2 border-t border-[var(--color-rule)] p-6">
								<div className="flex items-center gap-2">
									<a.icon className="size-4 shrink-0 text-[var(--color-cobalt)]" />
									<h3 className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
										{a.title}
									</h3>
									<ArrowUpRight className="ml-auto size-3.5 shrink-0 text-[var(--color-ink-4)] opacity-0 transition-opacity duration-200 group-focus-visible:opacity-100 group-hover:opacity-100" />
								</div>
								<p className="text-[13.5px] leading-[1.55] text-[var(--color-ink-2)]">
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
						<h2 className="text-[clamp(28px,3.6vw,44px)] font-semibold leading-[1.06] tracking-[-0.035em] text-[var(--color-ink)]">
							Your app model stays in TypeScript.
						</h2>
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
				</div>

				<div className="mt-14 grid gap-4 lg:grid-cols-2">
					<Card>
						<h3 className="text-[19px] font-semibold tracking-tight text-[var(--color-ink)]">
							Connect GitHub
						</h3>
						<p className="mt-2.5 text-[14.5px] leading-[1.6] text-[var(--color-ink-2)]">
							Install the Smallware GitHub App once. Pushes to the default
							branch deploy; pull requests get previews that disappear after
							merge.
						</p>
						<ol className="mt-6 grid gap-2.5 text-[13.5px] leading-[1.55] text-[var(--color-ink-2)] [&>li]:flex [&>li]:gap-3 [&>li>span:first-child]:font-mono [&>li>span:first-child]:text-[var(--color-ink-3)] [&>li>span:first-child]:tabular-nums">
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
						<h3 className="text-[19px] font-semibold tracking-tight text-[var(--color-ink)]">
							<InlineCode>pylon deploy</InlineCode>
						</h3>
						<p className="mt-2.5 text-[14.5px] leading-[1.6] text-[var(--color-ink-2)]">
							Use the CLI for CI, locked-down environments, or a manual
							release. It reaches the same Cloud runtime as the GitHub flow.
						</p>
						<div className="mt-6 overflow-hidden rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] font-mono text-[12.5px] leading-[1.75] text-[var(--color-ink)]">
							<div className="border-b border-[var(--color-rule)] px-4 py-2 font-mono text-[10.5px] uppercase tracking-[0.06em] text-[var(--color-ink-4)]">
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
							<span className="mx-auto rounded-full border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-3 py-0.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
								pylonsync.com/dashboard
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
						["Replicas", "Run up to 32 load-balanced replicas per region on Pro."],
						["Global regions", "Deploy in US, EU, APAC, and South America regions."],
						["Up to 500 GB volume", "Grow storage live when the app needs room."],
						["Managed Postgres — private beta", "Bundled SQLite by default; co-located managed Postgres is in private beta."],
						["Autostop on idle", "Free tier sleeps when idle. Paid projects stay warm."],
						["Custom domains + TLS", "Bring your domain; Pylon handles TLS."],
						["SSO — OIDC + SAML", "Configure org-level SSO from the dashboard."],
						["Audit log + snapshots", "Activity log, one-click volume restore."],
					].map(([title, body]) => (
						<div
							key={title}
							className="flex flex-col gap-1.5 border-t border-[var(--color-rule)] py-5"
						>
							<h4 className="text-[14.5px] font-semibold tracking-tight text-[var(--color-ink)]">
								{title}
							</h4>
							<p className="text-[13.5px] leading-[1.55] text-[var(--color-ink-2)]">
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
							<h3 className="text-[15.5px] font-semibold leading-snug tracking-tight text-[var(--color-ink)]">
								{o.q}
							</h3>
							<p className="text-[13.5px] leading-[1.65] text-[var(--color-ink-2)]">
								{o.a}
							</p>
						</div>
					))}
				</div>
			</Section>

			{/* PRICING */}
			<Section id="pricing">
				<H2>Pricing you can start with.</H2>
				<p className="mt-5 max-w-[560px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
					Start with one free project. Pro is $25 per org per month; heavier
					compute, replicas, and storage bill as usage.
				</p>

				<div className="mt-14">
					<PricingPlans signedIn={signedIn} />
				</div>
			</Section>

			{/* CTA */}
			<section>
				<div className="mx-auto max-w-[860px] px-5 pb-20 pt-10 text-center sm:px-8 sm:pb-24">
					<h2 className="mx-auto max-w-[18ch] text-[clamp(34px,5vw,60px)] font-semibold leading-[1.05] tracking-[-0.035em] text-[var(--color-ink)]">
						Create a Pylon app.
					</h2>
					<p className="mx-auto mt-5 max-w-[460px] text-[16px] leading-[1.55] text-[var(--color-ink-2)] sm:text-[17px]">
						The framework is free to self-host. Smallware runs it for you —
						connect GitHub or deploy from the CLI.
					</p>
					<div className="mt-8 flex flex-col items-stretch justify-center gap-2.5 sm:flex-row sm:items-center sm:gap-3">
						<Button asChild variant="primary" size="lg">
							<Link href={signedIn ? "/dashboard" : "/signup"}>
								{signedIn ? "Open dashboard →" : "Create your account →"}
							</Link>
						</Button>
						<Button asChild variant="ghost" size="lg">
							<a href="https://docs.pylonsync.com/cloud">Read the docs</a>
						</Button>
					</div>
				</div>
			</section>

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
			className="group inline-flex items-center gap-3 rounded-full border border-[var(--color-rule)] bg-[var(--color-paper)] py-3 pl-5 pr-4 font-mono text-[13.5px] text-[var(--color-ink)] shadow-[var(--shadow-card)] transition-colors hover:border-[var(--color-cobalt)]/50 hover:bg-[var(--color-paper-1)]"
		>
			<span className="select-none text-[var(--color-cobalt)]">$</span>
			<span className="tracking-tight">{command}</span>
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
		<div className="flex items-center gap-2 border-b border-[var(--color-rule)] px-3.5 py-2">
			<span className="flex gap-1">
				<span className="size-2 rounded-full bg-[var(--color-rule)]" />
				<span className="size-2 rounded-full bg-[var(--color-rule)]" />
				<span className="size-2 rounded-full bg-[var(--color-rule)]" />
			</span>
			<span className="truncate font-mono text-[10.5px] text-[var(--color-ink-4)]">
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
			<div className="px-3.5 py-2.5 font-mono text-[11.5px] leading-[1.95]">
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
			<div className="px-3.5 py-2.5 font-mono text-[11.5px] leading-[1.9]">
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
					<span className="font-mono text-[10px] uppercase tracking-[0.1em] text-[var(--color-status-fail)]">
						1 error
					</span>
				}
			/>
			<div className="px-3.5 py-2.5 font-mono text-[11.5px] leading-[1.9]">
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
					<span className="inline-flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
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
						className="flex items-center gap-3 px-3.5 py-[7px] font-mono text-[11.5px]"
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
		// The whole card is the link to its product page. Grayscale is scoped to
		// `(hover: hover)` so touch users — who can never un-grey it — get the
		// coloured card instead. Focus mirrors hover so keyboard users get the
		// same reveal, and the ring keeps the card visible as a tab stop.
		<Link
			href={href}
			onMouseEnter={() => setHovered(true)}
			onMouseLeave={() => setHovered(false)}
			onFocus={() => setHovered(true)}
			onBlur={() => setHovered(false)}
			className={`group flex flex-col overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[var(--shadow-card)] transition-[filter,box-shadow,border-color] duration-300 ease-[var(--ease-out-quart)] hover:border-[var(--color-rule)] hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper)] [@media(hover:hover)]:grayscale [@media(hover:hover)]:focus-within:grayscale-0 [@media(hover:hover)]:hover:grayscale-0 ${className ?? ""}`}
		>
			<div className="flex flex-col gap-1.5 p-5 pb-3">
				<div className="flex items-center gap-2">
					<Icon className="size-4 shrink-0 text-[var(--color-cobalt)]" />
					<h3 className="text-[14.5px] font-semibold tracking-tight text-[var(--color-ink)]">
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

function HeroBento() {
	return (
		<div>
			<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-12">
			<BentoCard
				icon={Table2}
				title="Typed schema"
				href="/product/database"
				desc="Entities and fields in TypeScript. Pylon creates the tables and migrates on save."
				className="min-h-[290px] lg:col-span-4"
			>
				<SchemaBento />
			</BentoCard>

			<BentoCard
				icon={RefreshCw}
				title="Live queries"
				href="/product/sync"
				desc="db.useQuery opens a subscription. Every write pushes a diff — no polling, no refetch."
				className="min-h-[290px] lg:col-span-4"
			>
				<LiveQueryBento />
			</BentoCard>

			<BentoCard
				icon={ShieldCheck}
				title="Row-level policies"
				href="/product/auth"
				desc="Access rules sit next to the schema. Every read and write is checked; default deny."
				className="min-h-[290px] lg:col-span-4"
			>
				<PolicyBento />
			</BentoCard>

			<BentoCard
				icon={KeyRound}
				title="Auth, included"
				href="/product/auth"
				desc="Magic link, 25+ OAuth providers, OIDC, and API keys."
				className="min-h-[230px] lg:col-span-3"
			>
				<AuthBento />
			</BentoCard>

			<BentoCard
				icon={Upload}
				title="File uploads"
				href="/product/storage"
				desc="Presigned uploads to local disk or any S3-compatible bucket."
				className="min-h-[230px] lg:col-span-3"
			>
				<UploadBento />
			</BentoCard>

			<BentoCard
				icon={Radio}
				title="Rooms & presence"
				href="/product/realtime"
				desc="Cursors, typing, and who is online over the same server."
				className="min-h-[230px] lg:col-span-3"
			>
				<PresenceBento />
			</BentoCard>

			<BentoCard
				icon={Search}
				title="Faceted search"
				href="/product/search"
				desc="Full-text queries with live facet counts, updated in the same transaction as your writes."
				className="min-h-[230px] lg:col-span-3"
			>
				<SearchBento />
			</BentoCard>

			<BentoCard
				icon={Database}
				title="SQLite or Postgres"
				href="/product/database"
				desc="Start on a single SQLite file. Point DATABASE_URL at Postgres and nothing above the driver changes."
				className="min-h-[210px] lg:col-span-6"
			>
				<EngineBento />
			</BentoCard>

			<BentoCard
				icon={Server}
				title="Server-rendered React"
				href="/product/ssr"
				desc="Queries and policies run on the server for first paint, then the same typed client hydrates and subscribes."
				className="min-h-[210px] lg:col-span-6"
			>
				<SsrBento />
			</BentoCard>

			<BentoCard
				icon={Braces}
				title="Server functions"
				href="/product/functions"
				desc="Queries, mutations, and actions in TypeScript files, validated and called through the typed client."
				className="min-h-[230px] lg:col-span-3"
			>
				<FunctionsBento />
			</BentoCard>

			<BentoCard
				icon={Activity}
				title="Reactive server queries"
				href="/product/functions"
				desc="Joins and derived data. The server tracks what it read and re-runs when those rows change."
				className="min-h-[230px] lg:col-span-3"
			>
				<ReactiveBento />
			</BentoCard>

			<BentoCard
				icon={Workflow}
				title="Scheduled & deferred work"
				href="/product/workflows"
				desc="runAfter, runAt, and cancel. Delays and retries run in the same process — no separate worker."
				className="min-h-[230px] lg:col-span-3"
			>
				<SchedulerBento />
			</BentoCard>

			<BentoCard
				icon={LayoutDashboard}
				title="Admin studio"
				href="/product/studio"
				desc="Browse tables, inspect live queries, and tail logs at /studio. Admin-gated in production."
				className="min-h-[230px] lg:col-span-3"
			>
				<StudioTabsBento />
			</BentoCard>
			</div>

			{/* Carries the thesis and the /product link that the primitives grid
			    used to hold, now that the grid is gone. */}
			<div className="mt-6 flex flex-col items-start justify-between gap-3 border-t border-[var(--color-rule)] pt-5 sm:flex-row sm:items-center">
				<p className="text-[14.5px] leading-[1.55] text-[var(--color-ink-2)]">
					<span className="font-semibold text-[var(--color-ink)]">
						Pick what you need.
					</span>{" "}
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
		<code className="rounded-[2px] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-1.5 py-0.5 font-mono text-[12.5px] text-[var(--color-ink)]">
			{children}
		</code>
	);
}
