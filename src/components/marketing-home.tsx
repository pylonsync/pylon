"use client";

import { Image, Link } from "@pylonsync/react";
import { createContext, useContext, useEffect, useState } from "react";
import {
	type LucideIcon,
	Github,
	GitPullRequest,
	Globe2,
	PackageCheck,
} from "lucide-react";
import { Button } from "./ui/button";
import { MarketingNav } from "./marketing-nav";
import { SiteFooter } from "./site-footer";
import { CodePanel } from "./code-panel";
import { TemplatesStrip } from "./templates-strip";
import { FRAME_COL } from "./marketing-frame";
import { HeroStack } from "./hero-stack";
import { HeroStart } from "./hero-start";
import { FeatureScroll } from "./feature-scroll";
import { ctaUrl } from "../lib/account-urls";

// Concrete affordances that let a coding agent build, verify, and ship on Pylon.
// Each one ships with the artifact it describes (`visual`). The claim and the
// evidence for it stay in the same card.
const AGENT_AFFORDANCES: {
	title: string;
	desc: string;
	href: string;
	action: string;
	artifact: string;
	lines: string[];
}[] = [
	{
		title: "Rules live in the repo",
		desc: "New apps include AGENTS.md and the Pylon skill. The agent reads project rules before it edits code.",
		href: "/skill",
		action: "Read the skill",
		artifact: "Repository",
		lines: ["AGENTS.md", ".agents/skills/pylon/SKILL.md", "app.ts"],
	},
	{
		title: "The path is command-line",
		desc: "Run npm create, pylon dev, and pylon deploy from one terminal.",
		href: "/product/cloud",
		action: "See cloud deployment",
		artifact: "Terminal",
		lines: [
			"npm create @pylonsync/pylon",
			"pylon dev",
			"pylon deploy",
		],
	},
	{
		title: "Generated types catch drift",
		desc: "Generated clients turn schema drift, missing fields, and invalid arguments into compile errors.",
		href: "/product/functions",
		action: "See server functions",
		artifact: "Type check",
		lines: [
			'db.useQuery("Order")',
			'Property "owner" does not exist',
			"Build stopped before deploy",
		],
	},
	{
		title: "Runtime state is visible",
		desc: "The agent can inspect tables, live queries, and logs in /studio while the local server runs.",
		href: "/product/studio",
		action: "See Studio",
		artifact: "Studio",
		lines: ["/studio/tables", "/studio/live-queries", "/studio/logs"],
	},
];

const DEPLOY_STEPS: {
	icon: LucideIcon;
	title: string;
	body: string;
}[] = [
	{
		icon: Github,
		title: "Connect the repo",
		body: "Install the GitHub App once, or use the CLI from CI.",
	},
	{
		icon: GitPullRequest,
		title: "Open a preview",
		body: "Each pull request gets an isolated preview environment.",
	},
	{
		icon: PackageCheck,
		title: "Build the release",
		body: "Pylon validates the app and applies the schema before cutover.",
	},
	{
		icon: Globe2,
		title: "Send traffic",
		body: "The release moves to production with the same runtime.",
	},
];

// Signed-in state for the nav/CTA. Seeded from the SSR `auth` prop (resolved
// server-side from the shared SessionStore) so the correct CTA, "Dashboard"
// or "Sign in", is in the server-rendered HTML on first paint. No flash: the
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
		<div className="relative min-h-screen bg-[var(--color-paper)] text-[var(--color-ink)]">
			<MarketingNav signedIn={signedIn} />

			{/* The <main> landmark is load-bearing, not decoration: Pylon's
			    markdown representation of a page (Accept: text/markdown, or
			    /index.md) reads this element. Without it the converter falls
			    back to <body> and an agent gets the page bracketed by chrome. */}
			<main>

			{/* HERO: one subject.
			    Centred headline, one line of copy, one focal diagram. The nine
			    bento cards used to open the page: the reader met nine small
			    equal-weight claims before meeting one idea, so nothing was the
			    subject and the thesis in the paragraph was drawn nowhere. The
			    bento now runs as its own band below, and the entity diagram
			    carries the hero. */}
			<header className="relative isolate overflow-hidden">
				<div
					className={`${FRAME_COL} px-5 pb-20 pt-16 text-center sm:px-8 sm:pb-24 sm:pt-24`}
				>
					<h1 className="mx-auto max-w-[21ch] text-[clamp(42px,6.4vw,68px)] font-semibold leading-[1.02] tracking-[-0.045em] text-[var(--color-ink)]">
						Give your agent{" "}
						{/* No `whitespace-nowrap` here: it's three words, and forcing them
						    onto one line overflows the measure below ~420px. Colour
						    carries across a wrap fine. */}
						<span className="text-[var(--color-brand)]">
							app building superpowers
						</span>
					</h1>

					<p className="mx-auto mt-6 max-w-[62ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Pylon is a full-stack framework built for agents to ship
						high-performance and secure apps quickly
					</p>

					{/* The only action in the hero. Docs and sign-up live in the nav;
					    a pair of large buttons here competed with the diagram for the
					    reader's first move. */}
					<HeroStart />

					<HeroStack />
				</div>
			</header>

			{/* A dark scroll chapter interrupts the white hero and makes the runtime
			    relationship tangible. The sticky index follows the reader while each
			    capability group reveals its own live product composition. */}
			<FeatureScroll />

			{/* Templates, straight after the pillars. Those say what the framework
			    does; this is the shortest path from reading that to running one. */}
			<TemplatesStrip />

			{/* Agent workflow. One continuous frame replaces the repeated card grid.
			    Each row keeps the claim and its evidence in the same reading path. */}
			<Section id="agents" tone="sunken">
				<SectionLabel>Agent workflow</SectionLabel>
				<div className="border-b border-[var(--color-rule)] px-5 py-16 sm:px-8 sm:py-20 lg:py-24">
					<H2>Give agents a system they can inspect.</H2>
					<p className="mt-5 max-w-[620px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Rules, commands, types, data, and logs stay in one workflow. Your
						agent can create, run, inspect, and deploy the app without changing
						tools.
					</p>
				</div>

				<div className="bg-[var(--color-paper)]">
					{AGENT_AFFORDANCES.map((a) => (
						<Link
							key={a.title}
							href={a.href}
							className="t-learn group grid border-b border-[var(--color-rule)] transition-colors duration-200 last:border-b-0 hover:bg-[var(--color-paper-1)] focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--color-brand)] md:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)]"
						>
							<div className="flex flex-col justify-center p-6 sm:p-8 md:min-h-[230px] lg:p-10">
								<h3 className="text-[20px] font-semibold tracking-[-0.02em] text-[var(--color-ink)]">
									{a.title}
								</h3>
								<p className="mt-5 max-w-[440px] text-[15px] leading-[1.65] text-[var(--color-ink-2)]">
									{a.desc}
								</p>
								<span className="mt-7 text-[13px] font-medium text-[var(--color-brand)] underline-offset-4 group-hover:underline">
									{a.action}
								</span>
							</div>

							<div className="flex min-h-[190px] items-center border-t border-white/10 bg-[#111116] p-6 sm:p-8 md:min-h-[230px] md:border-l md:border-t-0 md:p-10">
								<div className="w-full min-w-0 font-mono">
									<div className="border-b border-white/10 pb-3 text-[10.5px] uppercase tracking-[0.12em] text-[#85858f]">
										{a.artifact}
									</div>
									<div className="mt-4 grid gap-3 text-[12px] text-[#d4d4d8] sm:text-[12.5px]">
										{a.lines.map((line) => (
											<code key={line} className="block truncate">
												{line}
											</code>
										))}
									</div>
								</div>
							</div>
						</Link>
					))}
				</div>
			</Section>

			{/* The model */}
			<Section id="model">
				<SectionLabel>Application model</SectionLabel>
				<div className="grid min-w-0 lg:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)]">
					<div className="flex min-w-0 items-center px-5 py-16 sm:px-8 sm:py-20 lg:min-h-[560px] lg:px-12 lg:py-24">
						<div className="min-w-0">
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
					</div>

					<div className="flex min-w-0 items-center border-t border-[var(--color-rule)] bg-[var(--color-brand-soft)]/35 p-5 sm:p-8 lg:min-h-[560px] lg:border-l lg:border-t-0 lg:p-12">
						<CodePanel
							filename="app.ts"
							className="rounded-[6px] shadow-none"
							code={`// one entity → a synced table + typed client
const Order = entity("Order", {
  customer: field.string(),
  total: field.float(),
  paid: field.boolean().default(false),
});

// access rules next to the schema. Deny by default.
policy({ entity: "Order",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId == data.ownerId",
});

// the React side: live, typed, no fetch
const { data } = db.useQuery("Order");`}
						/>
					</div>
				</div>
			</Section>

			{/* DEPLOY */}
			<Section id="deploy" tone="sunken">
				<SectionLabel>Release path</SectionLabel>
				<div className="border-b border-[var(--color-rule)] px-5 py-16 sm:px-8 sm:py-20 lg:py-24">
					<H2>Deploy from GitHub or the CLI.</H2>
					<p className="mt-5 max-w-[560px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						A repository push and a CLI release use the same build, preview, and
						production path.
					</p>
				</div>

				<div className="bg-[var(--color-paper)]">
					<div className="grid grid-cols-2 lg:grid-cols-4">
						{DEPLOY_STEPS.map((step, index) => (
							<div
								key={step.title}
								className="border-b border-[var(--color-rule)] p-4 odd:border-r [&:nth-child(n+3)]:border-b-0 sm:p-6 lg:border-b-0 lg:even:border-r lg:last:border-r-0"
							>
								<div className="mb-5 flex items-center justify-between">
								<span className="flex size-9 items-center justify-center text-[var(--color-brand)]">
										<step.icon className="size-4" />
									</span>
								<span className="font-mono text-[10px] text-[var(--color-ink-4)]">
									0{index + 1}
								</span>
								</div>
								<h3 className="text-[15px] font-semibold text-[var(--color-ink)]">
									{step.title}
								</h3>
								<p className="mt-2 text-[13px] leading-[1.6] text-[var(--color-ink-3)]">
									{step.body}
								</p>
							</div>
						))}
					</div>

					<div className="grid border-t border-[var(--color-rule)] lg:grid-cols-[minmax(0,0.72fr)_minmax(0,1.28fr)]">
						<div className="flex flex-col justify-between p-6 sm:p-8 lg:p-10">
							<div>
								<p className="font-mono text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-brand)]">
									One release path
								</p>
								<h3 className="mt-4 max-w-[15ch] text-[28px] font-semibold leading-[1.05] tracking-[-0.03em] text-[var(--color-ink)] sm:text-[34px]">
									Push from GitHub or run the command.
								</h3>
							</div>
							<p className="mt-8 max-w-[420px] text-[14px] leading-[1.65] text-[var(--color-ink-2)]">
								Preview environments disappear after merge. Production keeps the
								release history and logs.
							</p>
						</div>

						<div className="border-t border-[var(--color-rule)] bg-[#111116] p-5 font-mono text-[12px] leading-[1.9] text-[#d4d4d8] sm:p-8 lg:border-l lg:border-t-0 lg:p-10">
							<div className="mb-6 flex items-center justify-between border-b border-white/10 pb-3 text-[10.5px] text-[#85858f]">
								<span>my-app / release</span>
								<span className="text-[#6ee7b7]">ready</span>
							</div>
							<div>
								<span className="text-[#9f8cff]">$</span> git push origin main
							</div>
							<div className="text-[#85858f]">or</div>
							<div>
								<span className="text-[#9f8cff]">$</span> pylon deploy --target
								cloud
							</div>
							<div className="mt-5 text-[#6ee7b7]">✓ Build complete in 12s</div>
							<div className="text-[#6ee7b7]">✓ Schema applied</div>
							<div className="text-[#6ee7b7]">✓ Traffic moved with 0 errors</div>
							<div className="mt-5 text-[#a8a8b2]">
								→ https://your-app.smallware.run
							</div>
						</div>
					</div>
				</div>
			</Section>

			{/* SCALE */}
			<Section id="scale">
				<SectionLabel>Managed cloud</SectionLabel>
				<div className="border-b border-[var(--color-rule)] px-5 py-16 sm:px-8 sm:py-20 lg:py-24">
					<H2>Scale from one dashboard.</H2>
					<p className="mt-5 max-w-[560px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Every app sits behind a global edge network. Resize machines, add
						replicas and regions, or expand storage from the same dashboard,
						without pre-provisioning or per-seat pricing.
					</p>
				</div>

				{/* The Cloud dashboard itself: the managed surface for the framework
				    above. Given a browser frame and set wider than the copy column so
				    it reads as the section's centerpiece. Width/height match the
				    source (3456×2234) so the slot is reserved and CLS stays 0. */}
				<div className="border-b border-[var(--color-rule)] bg-[var(--color-paper)]">
					<div className="overflow-hidden">
						<div className="flex items-center gap-2.5 border-b border-[var(--color-rule)] px-4 py-2.5">
							<span className="flex gap-1.5">
								<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
								<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
								<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
							</span>
							{/* The dashboard in the shot answers on the product host, not this
							    one. The bar used to read pylonsync.com/dashboard, which is a
							    404. This site has no auth and no dashboard. */}
							<span className="mx-auto rounded-full border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-3 py-0.5 font-mono text-[10.5px] text-[var(--color-ink-4)]">
								usesmallware.com/dashboard
							</span>
						</div>
						<div className="overflow-hidden">
							<Image
								src="/marketing/pylon-cloud-dashboard.png"
								alt="Stack0 Cloud dashboard with deployments, machine status, and live metrics"
								width={3456}
								height={2234}
								sizes="(min-width: 1120px) 1120px, 100vw"
								widths={[828, 1200, 2048]}
								className="block h-[320px] w-full object-cover object-[20%_top] sm:h-auto sm:object-contain"
							/>
						</div>
					</div>
				</div>

				{/* A hairline ledger, not a card mesh. Ten items divide evenly across
				    two columns. The old three-column mesh left two dead cells that
				    rendered as gray voids. */}
				<div className="grid sm:grid-cols-2">
					{[
						[
							"Global edge network",
							"Cloudflare's edge provides CDN caching, TLS, and DDoS protection worldwide with no extra configuration.",
						],
						[
							"Resize on demand",
							"Add RAM up to 64 GB, choose performance CPUs, and expand the volume without redeploying.",
						],
						["Replicas", "Run up to 32 load-balanced replicas per region."],
						[
							"Global regions",
							"Deploy in US, EU, APAC, and South America regions.",
						],
						[
							"Up to 500 GB volume",
							"Grow storage live when the app needs room.",
						],
						[
							"Managed Postgres (private beta)",
							"Bundled SQLite by default; co-located managed Postgres is in private beta.",
						],
						[
							"Autostop on idle",
							"Scale to zero when idle, or keep a project always warm.",
						],
						["Custom domains + TLS", "Bring your domain; Pylon handles TLS."],
						[
							"SSO: OIDC + SAML",
							"Configure org-level SSO from the dashboard.",
						],
						[
							"Audit log + snapshots",
							"Activity log, one-click volume restore.",
						],
					].map(([title, body]) => (
						<div
							key={title}
							className="flex flex-col gap-1.5 border-b border-[var(--color-rule)] px-5 py-6 odd:sm:border-r sm:px-8 lg:px-10"
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

			{/* FAQ: practical questions with checkable answers. This replaced a
			    "here are the objections we expect" block: writing the reader's
			    doubts for them is a rhetorical device, not information, and it left
			    the page with no plain statement of what Pylon actually supports. */}
			<Section id="faq" tone="sunken">
				<SectionLabel>FAQ</SectionLabel>
				<div className="border-b border-[var(--color-rule)] px-5 py-16 sm:px-8 sm:py-20 lg:py-24">
					<H2>Common questions.</H2>
					<p className="mt-5 max-w-[560px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Everything else is in the{" "}
						<a
							href="https://docs.pylonsync.com"
							className="text-[var(--color-brand)] underline underline-offset-2"
						>
							docs
						</a>
						.
					</p>
				</div>

				<div className="grid sm:grid-cols-2">
					{[
						{
							q: "Which database does it use?",
							a: "SQLite is the default. It uses one file and needs no setup. Set DATABASE_URL to a Postgres connection string to use the same schema and application code with Postgres. On Cloud, bundled SQLite is the default. Co-located managed Postgres is in private beta.",
						},
						{
							q: "Do I have to use Stack0 Cloud?",
							a: "No. The runtime is one open-source binary. Run it on your own computer or container platform with a volume for SQLite, or use your own Postgres database. Cloud is the managed option, not a requirement. It runs the same binary.",
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
							a: "Yes. ctx.scheduler.runAfter, runAt, and cancel schedule follow-up work. Delays and retries run in the same process as the rest of your app. You do not deploy a separate queue or worker.",
						},
						{
							q: "What happens to my data if I leave?",
							a: "It is a SQLite file or an ordinary Postgres database, with no proprietary storage layer in between. Take a dump and it opens in any client. What you would rewrite on the way out is the SDK calls, not the data.",
						},
					].map((o) => (
						<div
							key={o.q}
							className="flex flex-col gap-2.5 border-b border-[var(--color-rule)] px-5 py-8 odd:sm:border-r sm:px-8 lg:px-10"
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

			<section className="border-t border-[var(--color-rule)] bg-[#111116] text-[#fafafa]">
				<div className={`${FRAME_COL} grid md:grid-cols-[minmax(0,1fr)_360px]`}>
					<div className="border-b border-white/10 px-5 py-16 sm:px-8 sm:py-20 md:border-b-0 md:border-r md:border-white/10 lg:px-12 lg:py-24">
						<h2 className="max-w-[14ch] text-[clamp(34px,5vw,54px)] font-semibold leading-[1.02] tracking-[-0.04em]">
							Create a Pylon app.
						</h2>
						<p className="mt-5 max-w-[520px] text-[16px] leading-[1.6] text-[#a8a8b2] sm:text-[17px]">
							Self-host the framework, or use Stack0 Cloud to run it. Connect
							GitHub or deploy from the CLI.
						</p>
					</div>

					<div className="flex flex-col justify-end gap-5 px-5 py-10 sm:px-8 md:px-10 md:py-20 lg:py-24">
						<Link
							href={ctaUrl(signedIn)}
							className="inline-flex min-h-11 items-center justify-between gap-6 rounded-[var(--radius-sm)] bg-[#f4f4f5] px-4 py-3 text-[14px] font-medium text-[#18181b] transition-[background-color,transform] hover:bg-white active:translate-y-px focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-[#111116]"
						>
							{signedIn ? "Open dashboard" : "Create your account"}
							<span aria-hidden="true">→</span>
						</Link>
						<a
							href="https://docs.pylonsync.com/cloud"
							className="w-fit text-[14px] text-[#a8a8b2] underline decoration-white/25 underline-offset-4 transition-colors hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white"
						>
							Read the cloud docs
						</a>
					</div>
				</div>
			</section>

			</main>

			<SiteFooter />
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
			<div className={FRAME_COL}>{children}</div>
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

function SectionLabel({ children }: { children: React.ReactNode }) {
	return (
		<div className="border-b border-[var(--color-rule)] px-5 py-3 sm:px-8">
			<span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
				{children}
			</span>
		</div>
	);
}
